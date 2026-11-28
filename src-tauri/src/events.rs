//! Hook → app channel (no sockets). The Claude Code hook appends one JSON line per event
//! to `<app local data>/events.jsonl`; this thread tails it, keeps live per-session state
//! and shows notifications. Lines are untrusted: parsed as JSON, shown only as plain text.

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::AppHandle;

const MAX_FILE_BYTES: u64 = 1024 * 1024;
const REPLAY_BYTES: u64 = 256 * 1024;
const MAX_TEXT_CHARS: usize = 200;

#[derive(Clone, Copy, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Working,
    NeedsYou,
    Done,
    Ended,
}

#[derive(Clone, Debug)]
pub struct Live {
    pub state: State,
    pub since: Timestamp,
    pub project: String,
    pub message: String,
}

pub type LiveStore = Arc<Mutex<HashMap<String, Live>>>;

/// A permission prompt the hook is holding open while it waits for the user's answer here.
#[derive(Clone, Debug, PartialEq)]
pub struct Approval {
    pub request_id: String,
    pub session_id: String,
    pub project: String,
    pub tool_name: String,
    /// Full tool input as compact JSON. Never truncated for display: the user must see
    /// exactly what they approve.
    pub tool_input: String,
    pub requested_at: Timestamp,
    /// After this the hook has given up and Claude Code shows its own prompt.
    pub expires_at: Timestamp,
}

pub type Approvals = Arc<Mutex<HashMap<String, Approval>>>;

const MAX_TOOL_INPUT_CHARS: usize = 20_000;

#[derive(Deserialize, Debug, Default)]
#[serde(default)]
pub struct HookEvent {
    pub event: String,
    pub project: String,
    pub message: String,
    pub session_id: String,
    pub notification_type: String,
    pub ts: String,
    // PermissionRequest / PermissionResolved / PermissionTimeout
    pub request_id: String,
    pub tool_name: String,
    pub tool_input: String,
    pub wait: u64,
}

/// Writes the user's answer for a pending, unexpired request where the waiting hook will find it.
/// The only code that can approve anything. The id must be a UUID (safe as a file name) that
/// the hook itself sent; each request can be answered once.
pub fn decide(map: &mut HashMap<String, Approval>, dir: &Path, request_id: &str, allow: bool, reason: &str, now: Timestamp) -> Result<(), String> {
    if !crate::actions::is_uuid(request_id) {
        return Err("Invalid request.".into());
    }
    if map.remove(request_id).is_none_or(|a| a.expires_at <= now) {
        return Err("This request already timed out. Answer it in Claude Code.".into());
    }
    let reason: String = reason.chars().filter(|c| !c.is_control()).take(500).collect();
    let decision = if allow {
        serde_json::json!({ "behavior": "allow" })
    } else {
        let reason = if reason.trim().is_empty() { "Denied in AgentPulse.".to_string() } else { reason };
        serde_json::json!({ "behavior": "deny", "message": reason })
    };
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // tmp + rename so the hook never reads a half-written file.
    let tmp = dir.join(format!("{request_id}.tmp"));
    fs::write(&tmp, decision.to_string()).map_err(|e| e.to_string())?;
    fs::rename(&tmp, dir.join(format!("{request_id}.json"))).map_err(|e| e.to_string())
}

/// Answers nobody is waiting for any more (the app restarted, or a click raced the timeout).
pub fn clear_stale_decisions(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let _ = fs::remove_file(e.path());
        }
    }
}

/// Tracks permission prompts waiting for an answer in AgentPulse. Pure, so testable.
pub fn apply_approval(map: &mut HashMap<String, Approval>, e: &HookEvent, now: Timestamp) {
    match e.event.as_str() {
        "PermissionRequest" if crate::actions::is_uuid(&e.request_id) && e.tool_input.chars().count() <= MAX_TOOL_INPUT_CHARS => {
            let at = e.ts.parse::<Timestamp>().unwrap_or(now);
            let wait = SignedDuration::from_secs(e.wait.clamp(1, 300) as i64);
            map.insert(
                e.request_id.clone(),
                Approval {
                    request_id: e.request_id.clone(),
                    session_id: e.session_id.clone(),
                    project: clean(&e.project),
                    tool_name: clean(&e.tool_name),
                    tool_input: e.tool_input.clone(),
                    requested_at: at,
                    expires_at: at + wait,
                },
            );
        }
        "PermissionResolved" | "PermissionTimeout" => {
            map.remove(&e.request_id);
        }
        _ => {}
    }
    map.retain(|_, a| a.expires_at > now);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertKind {
    Done,
    NeedsYou,
}

/// Something worth telling the user about (desktop notification, maybe phone push).
#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    pub kind: AlertKind,
    pub project: String,
    pub title: String,
    pub body: String,
    /// Which session it is about (for the "not checked yet" badge). Empty when unknown.
    pub session: String,
    /// How long the task ran, when known (hooks saw it start working). For the phone message.
    pub took_secs: Option<i64>,
}

/// Updates session state for one event. Returns an alert when the user should be told.
pub fn apply(store: &mut HashMap<String, Live>, e: &HookEvent, now: Timestamp) -> Option<Alert> {
    let since = e.ts.parse::<Timestamp>().unwrap_or(now);
    let project = clean(if e.project.is_empty() { "Claude Code" } else { &e.project });
    let message = clean(&e.message);
    let prev = store.get(&e.session_id).map(|l| l.state);
    let alert = |kind, title: String, body: String| Some(Alert { kind, project: project.clone(), title, body, session: e.session_id.clone(), took_secs: None });
    // Working since → now: how long the task that just finished ran.
    let took = store.get(&e.session_id).filter(|l| l.state == State::Working).map(|l| since.duration_since(l.since).as_secs()).filter(|s| *s >= 0);

    let (state, alert) = match e.event.as_str() {
        // A resumed session can report SessionStart right after the prompt that resumed it: that
        // must not cancel Working. Otherwise a session that (re)starts waits for you.
        "SessionStart" if store.get(&e.session_id).is_some_and(|l| l.state == State::Working && since.duration_since(l.since) < SignedDuration::from_secs(30)) => return None,
        "SessionStart" => (State::Done, None),
        "UserPromptSubmit" => (State::Working, None),
        "Stop" => (State::Done, alert(AlertKind::Done, format!("Claude finished: {project}"), "Task complete.".into()).map(|a| Alert { took_secs: took, ..a })),
        // Claude Code also sends an "idle" reminder after a finished turn; don't re-alert for it.
        "Notification" if e.notification_type == "idle_prompt" && prev == Some(State::Done) => (State::Done, None),
        "Notification" => (State::NeedsYou, alert(AlertKind::NeedsYou, format!("Claude needs you: {project}"), message.clone())),
        "PermissionRequest" => {
            let tool = clean(&e.tool_name);
            let body = format!("Allow {tool}? Approve or deny it in AgentPulse.");
            let live_msg = format!("Wants to use {tool}");
            let a = alert(AlertKind::NeedsYou, format!("Approval needed: {project}"), body);
            if !e.session_id.is_empty() {
                store.insert(e.session_id.clone(), Live { state: State::NeedsYou, since, project, message: live_msg });
            }
            return a;
        }
        "PermissionResolved" => (State::Working, None),
        // The hook gave up waiting; Claude Code now shows its own prompt.
        "PermissionTimeout" => (State::NeedsYou, None),
        "SessionEnd" => (State::Ended, None),
        _ => return None,
    };
    if !e.session_id.is_empty() {
        store.insert(e.session_id.clone(), Live { state, since, project, message });
    }
    store.retain(|_, l| now.duration_since(l.since) < SignedDuration::from_hours(24));
    alert
}

/// Drops control characters and caps length so a hostile line can't flood the UI.
fn clean(s: &str) -> String {
    let s: String = s.chars().filter(|c| !c.is_control()).collect();
    if s.chars().count() > MAX_TEXT_CHARS {
        s.chars().take(MAX_TEXT_CHARS).collect::<String>() + "..."
    } else {
        s
    }
}

/// Splits a chunk into complete lines; returns them plus the bytes consumed.
/// A trailing partial line (hook still writing) is left for the next poll.
pub fn complete_lines(buf: &[u8]) -> (Vec<&str>, usize) {
    let Some(last_nl) = buf.iter().rposition(|&b| b == b'\n') else { return (vec![], 0) };
    let lines = buf[..=last_nl]
        .split(|&b| b == b'\n')
        .filter_map(|l| std::str::from_utf8(l).ok())
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    (lines, last_nl + 1)
}

fn read_range(path: &PathBuf, from: u64, to: u64) -> Option<Vec<u8>> {
    let mut f = File::open(path).ok()?;
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = Vec::new();
    f.take(to - from).read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Tails the events file forever. `on_alert` runs for each new alert (not for replayed history).
pub fn spawn_watcher(
    app: AppHandle,
    path: PathBuf,
    store: LiveStore,
    approvals: Approvals,
    on_alert: impl Fn(&AppHandle, &Alert) + Send + 'static,
) {
    std::thread::spawn(move || {
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        // Rebuild recent session state from the file's tail, without notifying.
        let len = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if let Some(buf) = read_range(&path, len.saturating_sub(REPLAY_BYTES), len) {
            let mut s = store.lock().unwrap_or_else(|e| e.into_inner()); // keep running even if another thread panicked
            let mut ap = approvals.lock().unwrap_or_else(|e| e.into_inner());
            let (lines, _) = complete_lines(&buf);
            for line in lines.into_iter().skip(usize::from(len > REPLAY_BYTES)) {
                if let Ok(e) = serde_json::from_str::<HookEvent>(line) {
                    apply(&mut s, &e, Timestamp::now());
                    apply_approval(&mut ap, &e, Timestamp::now()); // expired ones are dropped
                }
            }
        }
        let mut offset = len;
        if offset > MAX_FILE_BYTES && OpenOptions::new().write(true).open(&path).and_then(|f| f.set_len(0)).is_ok() {
            offset = 0;
        }

        loop {
            std::thread::sleep(Duration::from_millis(750));
            let Ok(len) = fs::metadata(&path).map(|m| m.len()) else { continue };
            if len < offset {
                offset = 0; // truncated or replaced
            }
            if len == offset {
                continue;
            }
            if len - offset > MAX_FILE_BYTES {
                offset = len; // a flood; skip rather than spam
                continue;
            }
            let Some(buf) = read_range(&path, offset, len) else { continue };
            let (lines, used) = complete_lines(&buf);
            offset += used as u64;
            let mut alerts = Vec::new();
            {
                let mut s = store.lock().unwrap_or_else(|e| e.into_inner()); // keep running even if another thread panicked
                let mut ap = approvals.lock().unwrap_or_else(|e| e.into_inner());
                for line in lines {
                    let Ok(e) = serde_json::from_str::<HookEvent>(line) else { continue };
                    alerts.extend(apply(&mut s, &e, Timestamp::now()));
                    apply_approval(&mut ap, &e, Timestamp::now());
                }
            }
            for a in &alerts {
                on_alert(&app, a);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(event: &str, session: &str) -> HookEvent {
        HookEvent { event: event.into(), session_id: session.into(), project: "demo".into(), ..Default::default() }
    }

    #[test]
    fn keeps_partial_trailing_line_for_next_poll() {
        let (lines, used) = complete_lines(b"{\"a\":1}\n{\"b\":2}\n{\"c\":");
        assert_eq!(lines, vec!["{\"a\":1}", "{\"b\":2}"]);
        assert_eq!(used, 16);
        assert_eq!(complete_lines(b"no newline yet"), (vec![], 0));
    }

    #[test]
    fn session_lifecycle_and_toasts() {
        let mut s = HashMap::new();
        let now = Timestamp::now();
        assert_eq!(apply(&mut s, &ev("SessionStart", "a"), now), None);
        assert_eq!(apply(&mut s, &ev("UserPromptSubmit", "a"), now), None);
        assert_eq!(s["a"].state, State::Working);
        let mut n = ev("Notification", "a");
        n.message = "needs\u{7} ok".into();
        let a = apply(&mut s, &n, now).unwrap();
        assert_eq!((a.kind, a.title.as_str(), a.body.as_str()), (AlertKind::NeedsYou, "Claude needs you: demo", "needs ok"));
        assert_eq!(s["a"].state, State::NeedsYou);
        assert!(apply(&mut s, &ev("Stop", "a"), now).is_some());
        assert_eq!(s["a"].state, State::Done);
        // idle reminder after Stop: no second alert, stays Done
        let mut idle = ev("Notification", "a");
        idle.notification_type = "idle_prompt".into();
        assert_eq!(apply(&mut s, &idle, now), None);
        assert_eq!(s["a"].state, State::Done);
        apply(&mut s, &ev("SessionEnd", "a"), now);
        assert_eq!(s["a"].state, State::Ended);
        assert_eq!(apply(&mut s, &ev("PreToolUse", "a"), now), None);
    }

    #[test]
    fn session_start_right_after_a_prompt_keeps_working() {
        let mut s = HashMap::new();
        let t = Timestamp::now();
        let at = |event: &str, secs: i64| HookEvent { ts: (t + SignedDuration::from_secs(secs)).to_string(), ..ev(event, "a") };
        apply(&mut s, &at("UserPromptSubmit", 0), t);
        apply(&mut s, &at("SessionStart", 1), t); // resume reports in after the prompt
        assert_eq!(s["a"].state, State::Working);
        apply(&mut s, &at("Stop", 60), t);
        apply(&mut s, &at("SessionStart", 120), t); // a later start: waiting for you
        assert_eq!(s["a"].state, State::Done);
    }

    #[test]
    fn stop_reports_how_long_the_task_ran() {
        let mut s = HashMap::new();
        let start = Timestamp::now();
        let mut prompt = ev("UserPromptSubmit", "a");
        prompt.ts = start.to_string();
        apply(&mut s, &prompt, start);
        let mut stop = ev("Stop", "a");
        stop.ts = (start + SignedDuration::from_secs(754)).to_string();
        assert_eq!(apply(&mut s, &stop, start).unwrap().took_secs, Some(754));
        // a Stop without a Working before it (e.g. app started mid-task): unknown, not zero
        assert_eq!(apply(&mut s, &ev("Stop", "b"), start).unwrap().took_secs, None);
    }

    #[test]
    fn approvals_lifecycle() {
        let now = Timestamp::now();
        let mut ap = HashMap::new();
        let mut s = HashMap::new();
        let id = "12345678-90ab-4cde-8f01-23456789abc8";
        let req = HookEvent {
            event: "PermissionRequest".into(),
            session_id: "s".into(),
            project: "demo".into(),
            request_id: id.into(),
            tool_name: "Bash".into(),
            tool_input: r#"{"command":"rm -rf build"}"#.into(),
            wait: 60,
            ..Default::default()
        };
        apply_approval(&mut ap, &req, now);
        let a = apply(&mut s, &req, now).unwrap();
        assert_eq!((a.kind, a.title.as_str()), (AlertKind::NeedsYou, "Approval needed: demo"));
        assert_eq!(ap[id].tool_input, r#"{"command":"rm -rf build"}"#); // shown in full
        assert_eq!(s["s"].state, State::NeedsYou);

        let done = HookEvent { event: "PermissionResolved".into(), session_id: "s".into(), request_id: id.into(), ..Default::default() };
        apply_approval(&mut ap, &done, now);
        apply(&mut s, &done, now);
        assert!(ap.is_empty());
        assert_eq!(s["s"].state, State::Working);

        // expired requests disappear; non-UUID ids are ignored
        apply_approval(&mut ap, &req, now);
        apply_approval(&mut ap, &HookEvent::default(), now + SignedDuration::from_secs(61));
        assert!(ap.is_empty());
        apply_approval(&mut ap, &HookEvent { request_id: "../../evil".into(), ..req }, now);
        assert!(ap.is_empty());
    }

    #[test]
    fn decide_only_answers_pending_unexpired_requests_once() {
        let now = Timestamp::now();
        let dir = std::env::temp_dir().join(format!("agentpulse-decide-{}", std::process::id()));
        let id = "12345678-90ab-4cde-8f01-23456789abc8";
        let pending = |secs: i64| Approval {
            request_id: id.into(),
            session_id: "s".into(),
            project: "demo".into(),
            tool_name: "Bash".into(),
            tool_input: "{}".into(),
            requested_at: now,
            expires_at: now + SignedDuration::from_secs(secs),
        };
        let read = || fs::read_to_string(dir.join(format!("{id}.json"))).unwrap();

        // allow → exactly {"behavior":"allow"}; answering twice fails
        let mut map = HashMap::from([(id.to_string(), pending(30))]);
        decide(&mut map, &dir, id, true, "ignored", now).unwrap();
        assert_eq!(read(), r#"{"behavior":"allow"}"#);
        assert!(decide(&mut map, &dir, id, true, "", now).is_err());

        // deny → reason passed on, control chars stripped, capped at 500; empty → default text
        let mut map = HashMap::from([(id.to_string(), pending(30))]);
        decide(&mut map, &dir, id, false, &format!("not\u{7} now{}", "x".repeat(600)), now).unwrap();
        let v: serde_json::Value = serde_json::from_str(&read()).unwrap();
        assert_eq!(v["behavior"], "deny");
        assert!(v["message"].as_str().unwrap().starts_with("not now") && v["message"].as_str().unwrap().chars().count() == 500);
        let mut map = HashMap::from([(id.to_string(), pending(30))]);
        decide(&mut map, &dir, id, false, "  ", now).unwrap();
        assert!(read().contains("Denied in AgentPulse."));

        // expired, unknown, or non-UUID ids are refused and write nothing
        fs::remove_dir_all(&dir).unwrap();
        let mut map = HashMap::from([(id.to_string(), pending(0))]);
        assert!(decide(&mut map, &dir, id, true, "", now).is_err());
        assert!(decide(&mut HashMap::new(), &dir, id, true, "", now).is_err());
        assert!(decide(&mut HashMap::new(), &dir, "../../evil", true, "", now).is_err());
        assert!(!dir.exists());

        // stale answers are cleared
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("old.json"), "{}").unwrap();
        clear_stale_decisions(&dir);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn caps_text_and_prunes_old_sessions() {
        let mut s = HashMap::new();
        let mut long = ev("Notification", "b");
        long.message = "x".repeat(500);
        let body = apply(&mut s, &long, Timestamp::now()).unwrap().body;
        assert_eq!(body.chars().count(), MAX_TEXT_CHARS + 3);
        let mut old = ev("Stop", "c");
        old.ts = (Timestamp::now() - SignedDuration::from_hours(30)).to_string();
        apply(&mut s, &old, Timestamp::now());
        assert!(!s.contains_key("c"));
    }
}
