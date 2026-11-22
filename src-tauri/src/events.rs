//! Hook → app channel (no sockets). The Claude Code hook appends one JSON line per event
//! to `<app local data>/events.jsonl`; this thread tails it, keeps live per-session state
//! and shows notifications. Lines are untrusted: parsed as JSON, shown only as plain text.

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
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

/// Tracks permission prompts waiting for an answer in AgentPulse. Pure, so testable.
pub fn apply_approval(map: &mut HashMap<String, Approval>, e: &HookEvent, now: Timestamp) {
    match e.event.as_str() {
        "PermissionRequest" if crate::actions::valid_session_id(&e.request_id) && e.tool_input.chars().count() <= MAX_TOOL_INPUT_CHARS => {
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
}

/// Updates session state for one event. Returns an alert when the user should be told.
pub fn apply(store: &mut HashMap<String, Live>, e: &HookEvent, now: Timestamp) -> Option<Alert> {
    let since = e.ts.parse::<Timestamp>().unwrap_or(now);
    let project = clean(if e.project.is_empty() { "Claude Code" } else { &e.project });
    let message = clean(&e.message);
    let prev = store.get(&e.session_id).map(|l| l.state);
    let alert = |kind, title: String, body: String| Some(Alert { kind, project: project.clone(), title, body });

    let (state, alert) = match e.event.as_str() {
        "SessionStart" => (State::Done, None),
        "UserPromptSubmit" => (State::Working, None),
        "Stop" => (State::Done, alert(AlertKind::Done, format!("Claude finished: {project}"), "Task complete.".into())),
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

/// Tails the events file forever. `on_alert` runs for each new alert (not for replayed history);
/// `on_change` runs after any state change (e.g. to update the tray).
pub fn spawn_watcher(
    app: AppHandle,
    path: PathBuf,
    store: LiveStore,
    approvals: Approvals,
    on_alert: impl Fn(&AppHandle, &Alert) + Send + 'static,
    on_change: impl Fn(&AppHandle, &HashMap<String, Live>) + Send + 'static,
) {
    std::thread::spawn(move || {
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        // Rebuild recent session state from the file's tail, without notifying.
        let len = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if let Some(buf) = read_range(&path, len.saturating_sub(REPLAY_BYTES), len) {
            let mut s = store.lock().unwrap();
            let mut ap = approvals.lock().unwrap();
            let (lines, _) = complete_lines(&buf);
            for line in lines.into_iter().skip(usize::from(len > REPLAY_BYTES)) {
                if let Ok(e) = serde_json::from_str::<HookEvent>(line) {
                    apply(&mut s, &e, Timestamp::now());
                    apply_approval(&mut ap, &e, Timestamp::now()); // expired ones are dropped
                }
            }
            on_change(&app, &s);
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
                let mut s = store.lock().unwrap();
                let mut ap = approvals.lock().unwrap();
                for line in lines {
                    let Ok(e) = serde_json::from_str::<HookEvent>(line) else { continue };
                    alerts.extend(apply(&mut s, &e, Timestamp::now()));
                    apply_approval(&mut ap, &e, Timestamp::now());
                }
                on_change(&app, &s);
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
