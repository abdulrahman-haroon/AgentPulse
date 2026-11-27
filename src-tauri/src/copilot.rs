//! Reads GitHub Copilot CLI session logs (`~/.copilot/session-state/<id>/events.jsonl`, older
//! `<id>.jsonl`), read-only. Undocumented format; checked 2026-11-27 against the shape (not the
//! content) of 68 real sessions. Lines are `{"type", "data", "id", "timestamp", "parentId"}`.
//! - session.start / session.resume: `context.cwd`, `selectedModel` → waiting for you
//! - user.message (`content`), assistant.turn_start → working. NOT assistant.turn_end: Copilot ends
//!   a "turn" after every model call; `session.usage_checkpoint` marks the finished task.
//! - permission.requested / .completed → needs you / working again
//! - session.usage_record (`usage.model`, `inputTokens`, `outputTokens`,
//!   `copilotUsage.totalNanoAiu` = AI credits used) → one request
//! - subagent.started / .completed (`toolCallId`, `agentName`, `agentDescription`, `executionMode`)
//! - abort, session.shutdown (→ ended)
//!
//! Copilot bills in AI credits, not per token at list price, so no USD cost is estimated; the
//! logged credits feed the plan-usage estimate (`limits.rs`).

use crate::logs::{self, FileData, Provider, Request, SubAgent};
use jiff::Timestamp;
use serde_json::Value;

/// Events we read. Everything else (tool calls, hooks, base64 assets, model snapshots; most of
/// the ~1 GB a heavy user has) is skipped before JSON parsing.
const HANDLED: &[&str] = &[
    "session.start",
    "session.resume",
    "session.model_change",
    "session.auto_mode_resolved",
    "user.message",
    "assistant.turn_start",
    "session.usage_record",
    "session.usage_checkpoint",
    "permission.requested",
    "permission.completed",
    "abort",
    "session.shutdown",
    "subagent.started",
    "subagent.completed",
    "subagent.failed",
];

/// First non-empty string under any of `keys`.
fn text<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| v[k].as_str()).filter(|s| !s.is_empty())
}

/// A user message's text: `content` (string or `[{text}]`), `text`, or `message`.
fn prompt(p: &Value) -> String {
    if let Some(t) = text(p, &["content", "text", "message", "prompt"]) {
        return t.to_string();
    }
    p["content"].as_array().map(|parts| parts.iter().filter_map(|x| x["text"].as_str()).collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

pub fn ingest(d: &mut FileData, line: &str, session: &str) {
    // Real lines start with the type, so most are skipped without parsing.
    if let Some(kind) = line.strip_prefix("{\"type\":\"").and_then(|r| r.split('"').next()) {
        if !HANDLED.contains(&kind) {
            return;
        }
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else { return };
    let Some(kind) = v["type"].as_str() else { return };
    let p = if v["data"].is_object() { &v["data"] } else if v["payload"].is_object() { &v["payload"] } else { &v };
    let ts = v["timestamp"].as_str().and_then(|t| t.parse::<Timestamp>().ok());
    let m = d.meta.entry(session.to_string()).or_default();
    m.provider = Provider::Copilot;
    if m.entrypoint.is_empty() {
        m.entrypoint = "copilot-cli".into();
    }
    if let Some(t) = ts {
        m.last_ts = Some(m.last_ts.map_or(t, |l| l.max(t)));
    }
    // The session's own model. Never from sub-agent or usage events (those may be other models).
    let model = match kind {
        "session.start" | "session.resume" => text(p, &["selectedModel", "model", "modelId"]),
        "session.model_change" => text(p, &["newModel", "model"]),
        "session.auto_mode_resolved" => text(p, &["chosenModel"]),
        _ => None,
    };
    if let Some(model) = model.filter(|x| *x != "auto") {
        d.model = Some(model.to_string());
        m.model = Some(logs::one_line(model, 64));
    }

    match kind {
        "session.start" | "session.resume" => {
            if let Some(cwd) = text(&p["context"], &["cwd"]).or_else(|| text(p, &["cwd", "workingDirectory"])) {
                m.cwd = cwd.to_string();
            }
            m.last_stop = Some("end_turn".into()); // open, waiting for a prompt
            m.ended = false;
        }
        "user.message" => {
            let t = logs::one_line(&prompt(p), logs::MAX_PROMPT_CHARS);
            if !t.is_empty() {
                d.titles.entry(session.to_string()).or_insert_with(|| logs::title_from(&t));
                m.last_prompt = t;
            }
            m.last_stop = None;
            m.ended = false;
        }
        "assistant.turn_start" | "permission.completed" => m.last_stop = None,
        "permission.requested" => m.last_stop = Some("needs_input".into()),
        "session.usage_checkpoint" | "abort" => m.last_stop = Some("end_turn".into()),
        "session.shutdown" => {
            m.last_stop = Some("end_turn".into());
            m.ended = true;
        }
        "session.usage_record" => {
            let Some(ts) = ts else { return };
            let u = &p["usage"];
            let n = |k: &str| u[k].as_u64().unwrap_or(0);
            let (input, output) = (n("inputTokens"), n("outputTokens"));
            let project = if m.cwd.is_empty() { "unknown".to_string() } else { logs::project_name(&m.cwd) };
            let model = text(u, &["model", "responseModel"]).map(str::to_string).or_else(|| d.model.clone()).unwrap_or_else(|| "copilot".into());
            d.push_request(Request {
                provider: Provider::Copilot,
                key: v["id"].as_str().map(|id| format!("copilot:{id}")),
                ts,
                session: session.to_string(),
                project,
                model: logs::one_line(&model, 64),
                tokens: input + output, // shortcut: cache tokens left out; unclear whether input already includes them
                output,
                cost: None,
                cache_saved: 0.0,
                // Nano AI units; 1e9 = one GitHub AI credit ($0.01).
                ai_credits: u["copilotUsage"]["totalNanoAiu"].as_f64().unwrap_or(0.0) / 1e9,
            });
        }
        "subagent.started" | "subagent.completed" | "subagent.failed" => {
            let Some(id) = text(p, &["toolCallId", "tool_call_id", "agentId", "id"]) else { return };
            let a = d.agents.entry(id.to_string()).or_insert_with(|| SubAgent {
                id: id.to_string(),
                session: session.to_string(),
                kind: logs::one_line(text(p, &["agentName", "agentDisplayName", "agentType", "name"]).unwrap_or("subagent"), 40),
                description: logs::one_line(text(p, &["agentDescription", "description", "prompt"]).unwrap_or(""), 120),
                background: p["executionMode"].as_str() == Some("background"),
                model: logs::one_line(text(p, &["model"]).unwrap_or(""), 64),
                started: ts,
                ..Default::default()
            });
            a.last = ts.or(a.last);
            a.finished = kind != "subagent.started";
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic lines in the real shape (values made up).
    fn feed(lines: &[&str]) -> FileData {
        let mut d = FileData::default();
        for l in lines {
            ingest(&mut d, l, "cs1");
        }
        d
    }

    const START: &str = r#"{"type":"session.start","data":{"sessionId":"cs1","selectedModel":"claude-sonnet-4.5","context":{"cwd":"/home/dev/api-gateway","branch":"main"}},"id":"e1","timestamp":"2026-06-15T12:00:00Z","parentId":null}"#;
    const ASK: &str = r#"{"type":"user.message","data":{"content":"add rate limiting","parentAgentTaskId":"x"},"id":"e2","timestamp":"2026-06-15T12:00:05Z","parentId":"e1"}"#;

    #[test]
    fn reads_session_prompt_usage_and_working_state() {
        let d = feed(&[
            START,
            ASK,
            r#"{"type":"assistant.turn_start","data":{"turnId":"0"},"id":"e3","timestamp":"2026-06-15T12:00:06Z","parentId":"e2"}"#,
            r#"{"type":"session.usage_record","data":{"usage":{"model":"gpt-6.1-terra","inputTokens":1000,"outputTokens":120,"cacheReadTokens":500,"cost":1,"copilotUsage":{"totalNanoAiu":2500000000}}},"id":"e4","timestamp":"2026-06-15T12:00:09Z","parentId":"e3"}"#,
            r#"{"type":"assistant.turn_end","data":{"turnId":"0"},"id":"e5","timestamp":"2026-06-15T12:00:10Z","parentId":"e4"}"#,
            r#"{"type":"subagent.started","data":{"toolCallId":"t1","agentName":"explore","agentDescription":"Find the router","executionMode":"background","model":"claude-haiku-4.5"},"id":"e6","timestamp":"2026-06-15T12:00:11Z","parentId":"e5"}"#,
            "not json",
        ]);
        let m = &d.meta["cs1"];
        // turn_end after one model call is NOT the end of the task
        assert_eq!((m.provider, m.cwd.as_str(), m.last_stop.as_deref()), (Provider::Copilot, "/home/dev/api-gateway", None));
        assert_eq!(m.model.as_deref(), Some("claude-sonnet-4.5")); // not the sub-agent's model
        assert_eq!(d.titles["cs1"], "add rate limiting");
        let r = &d.requests[0];
        assert_eq!((d.requests.len(), r.output, r.tokens, r.model.as_str(), r.project.as_str(), r.cost), (1, 120, 1120, "gpt-6.1-terra", "api-gateway", None));
        assert_eq!(r.ai_credits, 2.5);
        let a = &d.agents["t1"];
        assert_eq!((a.kind.as_str(), a.description.as_str(), a.background, a.finished), ("explore", "Find the router", true, false));
    }

    #[test]
    fn checkpoint_is_done_permission_is_needs_you_shutdown_ends() {
        let mut d = feed(&[START, ASK]);
        ingest(&mut d, r#"{"type":"permission.requested","data":{"requestId":"r1"},"id":"e7","timestamp":"2026-06-15T12:00:20Z","parentId":null}"#, "cs1");
        assert_eq!(d.meta["cs1"].last_stop.as_deref(), Some("needs_input"));
        ingest(&mut d, r#"{"type":"permission.completed","data":{"requestId":"r1"},"id":"e8","timestamp":"2026-06-15T12:00:25Z","parentId":null}"#, "cs1");
        assert_eq!(d.meta["cs1"].last_stop, None);
        ingest(&mut d, r#"{"type":"session.usage_checkpoint","data":{"totalPremiumRequests":1},"id":"e9","timestamp":"2026-06-15T12:01:00Z","parentId":null}"#, "cs1");
        assert_eq!(d.meta["cs1"].last_stop.as_deref(), Some("end_turn"));
        ingest(&mut d, r#"{"type":"session.shutdown","data":{},"id":"e10","timestamp":"2026-06-15T12:10:00Z","parentId":null}"#, "cs1");
        assert!(d.meta["cs1"].ended);
        // skipped event types never touch state, even when they'd parse
        ingest(&mut d, r#"{"type":"tool.execution_start","data":{"model":"other"},"id":"e11","timestamp":"2026-06-15T13:00:00Z"}"#, "cs1");
        assert_eq!(d.meta["cs1"].last_ts, Some("2026-06-15T12:10:00Z".parse().unwrap()));
    }
}
