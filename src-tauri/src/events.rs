//! Hook → app channel (ADR: file drop, no sockets). The Claude Code hook appends one JSON
//! line per event to `<app local data>/events.jsonl`; this thread tails it and shows a
//! native notification. Lines are untrusted: parsed as JSON, shown only as plain text.

use serde::Deserialize;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_TEXT_CHARS: usize = 200;

#[derive(Deserialize, Debug, PartialEq)]
pub struct HookEvent {
    pub event: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub message: String,
}

/// Title and body for a notification, or None for events we don't notify on.
pub fn render(e: &HookEvent) -> Option<(String, String)> {
    let project = clean(if e.project.is_empty() { "Claude Code" } else { &e.project });
    match e.event.as_str() {
        "Stop" => Some((format!("Claude finished: {project}"), "Task complete.".into())),
        "Notification" => Some((format!("Claude needs you: {project}"), clean(&e.message))),
        _ => None,
    }
}

/// Drops control characters and caps length so a hostile line can't flood the toast.
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

pub fn spawn_watcher(app: AppHandle, path: PathBuf) {
    std::thread::spawn(move || {
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        // Start at the end: events from before launch are stale. Reset an oversized file.
        let mut offset = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if offset > MAX_FILE_BYTES && OpenOptions::new().write(true).open(&path).and_then(|f| f.set_len(0)).is_ok() {
            offset = 0;
        }
        loop {
            std::thread::sleep(Duration::from_millis(750));
            let Ok(len) = fs::metadata(&path).map(|m| m.len()) else { continue };
            if len < offset {
                offset = 0; // file was truncated or replaced
            }
            if len == offset {
                continue;
            }
            if len - offset > MAX_FILE_BYTES {
                offset = len; // something wrote a flood; skip it rather than toast-spam
                continue;
            }
            let Ok(mut f) = File::open(&path) else { continue };
            let mut buf = Vec::new();
            if f.seek(SeekFrom::Start(offset)).is_err() || f.take(len - offset).read_to_end(&mut buf).is_err() {
                continue;
            }
            let (lines, used) = complete_lines(&buf);
            offset += used as u64;
            for line in lines {
                let Ok(e) = serde_json::from_str::<HookEvent>(line) else { continue };
                if let Some((title, body)) = render(&e) {
                    let _ = app.notification().builder().title(title).body(body).show();
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_partial_trailing_line_for_next_poll() {
        let (lines, used) = complete_lines(b"{\"a\":1}\n{\"b\":2}\n{\"c\":");
        assert_eq!(lines, vec!["{\"a\":1}", "{\"b\":2}"]);
        assert_eq!(used, 16);
        assert_eq!(complete_lines(b"no newline yet"), (vec![], 0));
    }

    #[test]
    fn renders_known_events_and_sanitizes_text() {
        let e: HookEvent = serde_json::from_str(r#"{"event":"Notification","project":"demo","message":"needs\u0007 ok"}"#).unwrap();
        assert_eq!(render(&e), Some(("Claude needs you: demo".into(), "needs ok".into())));
        let long = HookEvent { event: "Notification".into(), project: String::new(), message: "x".repeat(500) };
        let (title, body) = render(&long).unwrap();
        assert_eq!(title, "Claude needs you: Claude Code");
        assert_eq!(body.chars().count(), MAX_TEXT_CHARS + 3);
        let other = HookEvent { event: "PreToolUse".into(), project: String::new(), message: String::new() };
        assert_eq!(render(&other), None);
    }
}
