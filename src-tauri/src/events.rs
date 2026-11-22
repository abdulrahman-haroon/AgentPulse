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
use tauri_plugin_notification::NotificationExt;

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

#[derive(Deserialize, Debug, Default)]
#[serde(default)]
pub struct HookEvent {
    pub event: String,
    pub project: String,
    pub message: String,
    pub session_id: String,
    pub notification_type: String,
    pub ts: String,
}

/// Updates session state for one event. Returns (title, body) when a notification should show.
pub fn apply(store: &mut HashMap<String, Live>, e: &HookEvent, now: Timestamp) -> Option<(String, String)> {
    let since = e.ts.parse::<Timestamp>().unwrap_or(now);
    let project = clean(if e.project.is_empty() { "Claude Code" } else { &e.project });
    let message = clean(&e.message);
    let prev = store.get(&e.session_id).map(|l| l.state);

    let (state, toast) = match e.event.as_str() {
        "SessionStart" => (State::Done, None),
        "UserPromptSubmit" => (State::Working, None),
        "Stop" => (State::Done, Some((format!("Claude finished: {project}"), "Task complete.".to_string()))),
        // Claude Code also sends an "idle" reminder after a finished turn; don't re-alert for it.
        "Notification" if e.notification_type == "idle_prompt" && prev == Some(State::Done) => (State::Done, None),
        "Notification" => (State::NeedsYou, Some((format!("Claude needs you: {project}"), message.clone()))),
        "SessionEnd" => (State::Ended, None),
        _ => return None,
    };
    if !e.session_id.is_empty() {
        store.insert(e.session_id.clone(), Live { state, since, project, message });
    }
    store.retain(|_, l| now.duration_since(l.since) < SignedDuration::from_hours(24));
    toast
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

/// Tails the events file forever. `on_change` runs after any state change (e.g. to update the tray).
pub fn spawn_watcher(
    app: AppHandle,
    path: PathBuf,
    store: LiveStore,
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
            let (lines, _) = complete_lines(&buf);
            for line in lines.into_iter().skip(usize::from(len > REPLAY_BYTES)) {
                if let Ok(e) = serde_json::from_str::<HookEvent>(line) {
                    apply(&mut s, &e, Timestamp::now());
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
            let mut s = store.lock().unwrap();
            for line in lines {
                let Ok(e) = serde_json::from_str::<HookEvent>(line) else { continue };
                if let Some((title, body)) = apply(&mut s, &e, Timestamp::now()) {
                    let _ = app.notification().builder().title(title).body(body).show();
                }
            }
            on_change(&app, &s);
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
        assert_eq!(apply(&mut s, &n, now), Some(("Claude needs you: demo".into(), "needs ok".into())));
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
    fn caps_text_and_prunes_old_sessions() {
        let mut s = HashMap::new();
        let mut long = ev("Notification", "b");
        long.message = "x".repeat(500);
        let (_, body) = apply(&mut s, &long, Timestamp::now()).unwrap();
        assert_eq!(body.chars().count(), MAX_TEXT_CHARS + 3);
        let mut old = ev("Stop", "c");
        old.ts = (Timestamp::now() - SignedDuration::from_hours(30)).to_string();
        apply(&mut s, &old, Timestamp::now());
        assert!(!s.contains_key("c"));
    }
}
