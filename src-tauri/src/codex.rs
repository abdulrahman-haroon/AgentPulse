//! Reads OpenAI Codex CLI session logs (`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`), read-only.
//! Undocumented format (community-documented, checked 2026-11-27); every field is optional and
//! bad lines are skipped. Each line is `{"timestamp", "type", "payload"}`. We use:
//! - `session_meta`: id, cwd, originator (first line)
//! - `turn_context`: model
//! - `event_msg`: user_message, task_started / task_complete / turn_aborted (state),
//!   token_count (per-response usage + the plan's rate limits)

use crate::logs::{self, FileData, PlanLimit, Provider, Request};
use crate::pricing;
use jiff::Timestamp;
use serde_json::Value;

pub fn ingest(d: &mut FileData, line: &str, fallback: &str) {
    // Cheap skip: most lines are large response items we don't need.
    if !["\"session_meta\"", "\"turn_context\"", "\"event_msg\""].iter().any(|k| line.contains(k)) {
        return;
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else { return };
    let ts = v["timestamp"].as_str().and_then(|t| t.parse::<Timestamp>().ok());
    let p = &v["payload"];
    let kind = v["type"].as_str().unwrap_or("");
    if kind == "session_meta" {
        if let Some(id) = p["id"].as_str().filter(|id| !id.is_empty()) {
            d.session = Some(id.to_string());
        }
    }
    let session = d.session.clone().unwrap_or_else(|| session_from_file(fallback));
    let m = d.meta.entry(session.clone()).or_default();
    m.provider = Provider::Codex;
    if let Some(t) = ts {
        m.last_ts = Some(m.last_ts.map_or(t, |l| l.max(t)));
    }

    match (kind, p["type"].as_str().unwrap_or("")) {
        ("session_meta" | "turn_context", _) => {
            if let (true, Some(c)) = (m.cwd.is_empty(), p["cwd"].as_str()) {
                m.cwd = c.to_string();
            }
            if let Some(o) = p["originator"].as_str() {
                m.entrypoint = logs::one_line(o, 40);
            }
            if let Some(model) = p["model"].as_str() {
                d.model = Some(model.to_string());
                m.model = Some(model.to_string());
            }
        }
        ("event_msg", "user_message") => {
            let text = logs::one_line(p["message"].as_str().unwrap_or(""), logs::MAX_PROMPT_CHARS);
            if !text.is_empty() {
                d.titles.entry(session.clone()).or_insert_with(|| logs::title_from(&text));
                m.last_prompt = text;
            }
            m.last_stop = None;
        }
        ("event_msg", "task_started" | "turn_started") => m.last_stop = None,
        // Finished or interrupted: either way it's the user's turn.
        ("event_msg", "task_complete" | "turn_complete" | "turn_aborted") => m.last_stop = Some("end_turn".into()),
        ("event_msg", "token_count") => {
            let project = if m.cwd.is_empty() { "unknown".to_string() } else { logs::project_name(&m.cwd) };
            let total = p["info"]["total_token_usage"]["total_tokens"].as_u64();
            let last = &p["info"]["last_token_usage"];
            // The same snapshot is often logged twice; the running total tells them apart.
            if let (Some(ts), Some(total), true) = (ts, total, last.is_object()) {
                if total != d.total {
                    d.total = total;
                    let n = |k: &str| last[k].as_u64().unwrap_or(0);
                    let (input, cached, output) = (n("input_tokens"), n("cached_input_tokens"), n("output_tokens"));
                    let model = d.model.clone().unwrap_or_else(|| "unknown".into());
                    d.push_request(Request {
                        provider: Provider::Codex,
                        key: None,
                        ts,
                        session,
                        project,
                        tokens: input + output, // cached is part of input
                        output,
                        cost: pricing::openai_cost(&model, input, cached, output),
                        cache_saved: pricing::openai_cache_savings(&model, input, cached),
                        model,
                    });
                }
            }
            let limits: Vec<PlanLimit> = ["primary", "secondary"].iter().filter_map(|k| limit(&p["rate_limits"][k])).collect();
            if let (Some(ts), false) = (ts, limits.is_empty()) {
                if d.limits.as_ref().is_none_or(|(at, _)| ts >= *at) {
                    d.limits = Some((ts, limits));
                }
            }
        }
        _ => {}
    }
}

/// `{used_percent, window_minutes, resets_at (unix seconds)}` → a labelled limit.
fn limit(v: &Value) -> Option<PlanLimit> {
    let used = v["used_percent"].as_f64()?;
    let mins = v["window_minutes"].as_u64().unwrap_or(0);
    let window = match mins {
        10_080 => "Weekly".to_string(),
        m if m >= 1440 && m % 1440 == 0 => format!("{}-day", m / 1440),
        m if m >= 60 && m % 60 == 0 => format!("{}-hour", m / 60),
        0 => "Plan".to_string(),
        m => format!("{m}-min"),
    };
    let resets_at = v["resets_at"].as_i64().and_then(|s| Timestamp::from_second(s).ok()).map(|t| t.to_string());
    Some(PlanLimit { provider: Provider::Codex, window, used_percent: used.clamp(0.0, 100.0), resets_at })
}

/// `rollout-2026-06-15T12-00-00-<uuid>` → the uuid (used only if session_meta is missing).
fn session_from_file(stem: &str) -> String {
    let tail = stem.get(stem.len().saturating_sub(36)..).unwrap_or(stem);
    if crate::actions::is_uuid(tail) { tail.to_string() } else { stem.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic lines in the documented shape; never real session logs.
    const ID: &str = "0199a1b2-c3d4-4e5f-8a6b-7c8d9e0f1a2b";

    fn feed(lines: &[String]) -> FileData {
        let mut d = FileData::default();
        for l in lines {
            ingest(&mut d, l, "rollout-2026-06-15T12-00-00-0199a1b2-c3d4-4e5f-8a6b-7c8d9e0f1a2b");
        }
        d
    }

    fn tokens(ts: &str, total: u64, input: u64, cached: u64, output: u64) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"total_tokens":{total}}},"last_token_usage":{{"input_tokens":{input},"cached_input_tokens":{cached},"output_tokens":{output},"reasoning_output_tokens":10,"total_tokens":{}}}}},"rate_limits":{{"primary":{{"used_percent":42.5,"window_minutes":300,"resets_at":1781524800}},"secondary":{{"used_percent":12.0,"window_minutes":10080,"resets_at":1781956800}},"plan_type":null}}}}}}"#,
            input + output
        )
    }

    fn base() -> Vec<String> {
        vec![
            format!(r#"{{"timestamp":"2026-06-15T12:00:00Z","type":"session_meta","payload":{{"id":"{ID}","cwd":"C:\\code\\web-shop","originator":"codex_cli_rs","cli_version":"0.63.0"}}}}"#),
            r#"{"timestamp":"2026-06-15T12:00:01Z","type":"turn_context","payload":{"cwd":"C:\\code\\web-shop\\src","model":"gpt-5-codex","effort":"medium"}}"#.into(),
            r#"{"timestamp":"2026-06-15T12:00:02Z","type":"event_msg","payload":{"type":"user_message","message":"add a dark mode\ntoggle","images":[]}}"#.into(),
            r#"{"timestamp":"2026-06-15T12:00:03Z","type":"event_msg","payload":{"type":"task_started","turn_id":"1"}}"#.into(),
        ]
    }

    #[test]
    fn reads_session_title_state_and_usage() {
        let mut lines = base();
        lines.push(tokens("2026-06-15T12:00:10Z", 1100, 1000, 400, 100));
        lines.push(tokens("2026-06-15T12:00:10Z", 1100, 1000, 400, 100)); // repeated snapshot
        lines.push(r#"{"timestamp":"2026-06-15T12:00:11Z","type":"response_item","payload":{"type":"message","role":"assistant"}}"#.into());
        lines.push(tokens("2026-06-15T12:00:20Z", 3300, 2000, 1500, 200));
        let d = feed(&lines);

        let m = &d.meta[ID];
        assert_eq!((m.provider, m.cwd.as_str(), m.entrypoint.as_str()), (Provider::Codex, r"C:\code\web-shop", "codex_cli_rs"));
        assert_eq!(m.last_stop, None); // mid-turn
        assert_eq!(d.titles[ID], "add a dark mode toggle");
        assert_eq!(d.requests.len(), 2);
        let r = &d.requests[0];
        assert_eq!((r.session.as_str(), r.project.as_str(), r.model.as_str(), r.tokens, r.output), (ID, "web-shop", "gpt-5-codex", 1100, 100));
        // 600 fresh * 1.25 + 400 cached * 0.125 + 100 out * 10, per million
        assert!((r.cost.unwrap() - 0.00180).abs() < 1e-9, "{:?}", r.cost);

        let (_, limits) = d.limits.as_ref().unwrap();
        assert_eq!(limits.iter().map(|l| (l.window.as_str(), l.used_percent)).collect::<Vec<_>>(), [("5-hour", 42.5), ("Weekly", 12.0)]);
        assert!(limits[0].resets_at.is_some());
    }

    #[test]
    fn task_complete_means_done_and_junk_is_skipped() {
        let mut lines = base();
        lines.push("{not json \"event_msg\"".into());
        lines.push(r#"{"timestamp":"2026-06-15T12:01:00Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":null}}"#.into());
        lines.push(r#"{"timestamp":"2026-06-15T12:01:00Z","type":"event_msg","payload":{"type":"task_complete","last_agent_message":"done"}}"#.into());
        let d = feed(&lines);
        let m = &d.meta[ID];
        assert_eq!(m.last_stop.as_deref(), Some("end_turn"));
        assert_eq!(m.last_ts, Some("2026-06-15T12:01:00Z".parse().unwrap()));
        assert!(d.requests.is_empty() && d.limits.is_none());
    }

    #[test]
    fn falls_back_to_the_uuid_in_the_file_name() {
        let d = feed(&[r#"{"timestamp":"2026-06-15T12:00:02Z","type":"event_msg","payload":{"type":"user_message","message":"hi"}}"#.into()]);
        assert!(d.meta.contains_key(ID));
        assert_eq!(session_from_file("odd-name"), "odd-name");
    }
}
