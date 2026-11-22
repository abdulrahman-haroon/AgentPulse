//! The only module that starts processes. Rules:
//! - The UI sends a session id (and reply text); paths come from our own log cache, never the UI.
//! - Session ids must be UUIDs; folders must exist; programs are resolved from PATH by fixed name.
//! - Arguments are passed as a list (no shell). Reply text goes through stdin, never argv.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

const MAX_REPLY_CHARS: usize = 20_000;
const MAX_OUTPUT_CHARS: usize = 20_000;
pub const PERMISSION_MODES: [&str; 3] = ["default", "acceptEdits", "plan"];

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct ReplyView {
    pub session_id: String,
    /// running | done | error | cancelled
    pub status: String,
    /// Claude's text so far (assistant text blocks, then the final result).
    pub text: String,
    /// Recent tool calls, e.g. "Read", "Bash".
    pub activity: Vec<String>,
    pub cost_usd: Option<f64>,
    /// Tools that were blocked because the permission mode didn't allow them.
    pub denied: Vec<String>,
    pub error: String,
}

struct Running {
    view: ReplyView,
    child: Option<Arc<Mutex<Child>>>,
}

#[derive(Default, Clone)]
pub struct Replies(Arc<Mutex<HashMap<String, Running>>>);

impl Replies {
    pub fn views(&self) -> Vec<ReplyView> {
        self.0.lock().map(|m| m.values().map(|r| r.view.clone()).collect()).unwrap_or_default()
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.0.lock().is_ok_and(|m| m.get(id).is_some_and(|r| r.view.status == "running"))
    }

    pub fn cancel(&self, id: &str) {
        if let Ok(mut m) = self.0.lock() {
            if let Some(r) = m.get_mut(id) {
                if let Some(c) = &r.child {
                    let _ = c.lock().map(|mut c| c.kill());
                }
                if r.view.status == "running" {
                    r.view.status = "cancelled".into();
                }
            }
        }
    }

    pub fn dismiss(&self, id: &str) {
        if let Ok(mut m) = self.0.lock() {
            if m.get(id).is_some_and(|r| r.view.status != "running") {
                m.remove(id);
            }
        }
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut ReplyView)) {
        if let Ok(mut m) = self.0.lock() {
            if let Some(r) = m.get_mut(id) {
                f(&mut r.view);
            }
        }
    }
}

pub fn is_uuid(id: &str) -> bool {
    id.len() == 36
        && id.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

/// Model ids like "claude-opus-5-5"; rejects placeholders like "<synthetic>" and anything flag-like.
fn valid_model(m: &str) -> bool {
    m.starts_with("claude-") && m.len() <= 64 && m.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
}

/// First match for any of `names` in PATH directories.
fn find_on_path(names: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).flat_map(|dir| names.iter().map(move |n| dir.join(n))).find(|p| p.is_file())
}

fn claude_exe() -> Result<PathBuf, String> {
    let names: &[&str] = if cfg!(windows) { &["claude.exe", "claude.cmd"] } else { &["claude"] };
    find_on_path(names).ok_or_else(|| "Couldn't find the `claude` command on PATH.".into())
}

fn existing_dir(cwd: &str) -> Result<&Path, String> {
    let p = Path::new(cwd);
    if cwd.is_empty() || !p.is_dir() {
        return Err("This session's project folder no longer exists.".into());
    }
    Ok(p)
}

#[cfg(windows)]
fn flags(cmd: &mut Command, flag: u32) -> &mut Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(flag)
}
#[cfg(not(windows))]
fn flags(cmd: &mut Command, _flag: u32) -> &mut Command {
    cmd
}
const CREATE_NEW_CONSOLE: u32 = 0x10;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Env vars a parent Claude Code session sets for its own children (session id, messaging
/// token, entrypoint...). If AgentPulse was started from inside Claude Code, these must not
/// leak into the processes we launch. User config (CLAUDE_CONFIG_DIR) and auth (ANTHROPIC_*) stay.
fn is_inherited_session_var(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    n == "CLAUDECODE" || n == "CLAUDE_PID" || n.starts_with("CLAUDE_CODE_") || n.starts_with("CLAUDE_AGENT_SDK")
}

fn command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    for (k, _) in std::env::vars_os() {
        if is_inherited_session_var(&k.to_string_lossy()) {
            cmd.env_remove(k);
        }
    }
    cmd
}

/// VS Code sessions: focus/open the project folder in VS Code.
/// Terminal sessions: resume in a new console window (only when not running elsewhere).
pub fn open_session(id: &str, cwd: &str, entrypoint: &str, running: bool) -> Result<(), String> {
    if !is_uuid(id) {
        return Err("Invalid session id.".into());
    }
    let dir = existing_dir(cwd)?;
    if entrypoint.contains("vscode") {
        let code = find_on_path(if cfg!(windows) { &["code.cmd", "code.exe"] } else { &["code"] })
            .ok_or("Couldn't find the `code` command on PATH.")?;
        command(&code).arg(dir).spawn().map_err(|e| e.to_string())?;
        return Ok(());
    }
    if running {
        return Err("This session is running in a terminal. Switch to that window to answer it.".into());
    }
    let mut cmd = command(&claude_exe()?);
    cmd.args(["--resume", id]).current_dir(dir);
    flags(&mut cmd, CREATE_NEW_CONSOLE).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Deserialize)]
struct StreamLine {
    #[serde(rename = "type")]
    kind: String,
    message: Option<StreamMessage>,
    result: Option<String>,
    is_error: Option<bool>,
    total_cost_usd: Option<f64>,
    permission_denials: Option<Vec<serde_json::Value>>,
}

#[derive(Deserialize)]
struct StreamMessage {
    #[serde(default)]
    content: Vec<Block>,
}

#[derive(Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
    name: Option<String>,
}

fn push_capped(s: &mut String, add: &str) {
    if s.chars().count() < MAX_OUTPUT_CHARS {
        s.push_str(add);
    }
}

/// Folds one `--output-format stream-json` line into the view. Pure, so testable.
pub fn apply_stream_line(v: &mut ReplyView, line: &str) {
    let Ok(l) = serde_json::from_str::<StreamLine>(line) else { return };
    match l.kind.as_str() {
        "assistant" => {
            for b in l.message.map(|m| m.content).unwrap_or_default() {
                match (b.kind.as_str(), b.text, b.name) {
                    ("text", Some(t), _) => {
                        if !v.text.is_empty() {
                            push_capped(&mut v.text, "\n\n");
                        }
                        push_capped(&mut v.text, &t);
                    }
                    ("tool_use", _, Some(n)) => {
                        v.activity.push(n);
                        let over = v.activity.len().saturating_sub(6);
                        v.activity.drain(..over);
                    }
                    _ => {}
                }
            }
        }
        "result" => {
            if let Some(r) = l.result.filter(|r| !r.is_empty()) {
                v.text = r.chars().take(MAX_OUTPUT_CHARS).collect();
            }
            v.cost_usd = l.total_cost_usd;
            v.denied = l
                .permission_denials
                .unwrap_or_default()
                .iter()
                .map(|d| d["tool_name"].as_str().unwrap_or("a tool").to_string())
                .collect();
            v.status = if l.is_error == Some(true) { "error".into() } else { "done".into() };
        }
        _ => {}
    }
}

/// Sends `message` to an existing session with Claude Code's headless mode and streams the
/// result into `replies`. Caller must ensure the session isn't running elsewhere.
pub fn start_reply(replies: &Replies, id: &str, cwd: &str, model: Option<&str>, message: &str, mode: &str) -> Result<(), String> {
    if !is_uuid(id) {
        return Err("Invalid session id.".into());
    }
    let message = message.trim();
    if message.is_empty() || message.chars().count() > MAX_REPLY_CHARS {
        return Err("Reply must be between 1 and 20,000 characters.".into());
    }
    if !PERMISSION_MODES.contains(&mode) {
        return Err("Unknown permission mode.".into());
    }
    if replies.is_running(id) {
        return Err("A reply to this session is already running.".into());
    }
    let dir = existing_dir(cwd)?;

    let mut cmd = command(&claude_exe()?);
    cmd.args(["-p", "--resume", id, "--output-format", "stream-json", "--verbose", "--permission-mode", mode])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Continue on the session's own model rather than the user's default.
    if let Some(m) = model.filter(|m| valid_model(m)) {
        cmd.args(["--model", m]);
    }
    let mut child = flags(&mut cmd, CREATE_NO_WINDOW).spawn().map_err(|e| format!("Couldn't start Claude Code: {e}"))?;

    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let mut stderr = child.stderr.take().ok_or("no stderr")?;
    let child = Arc::new(Mutex::new(child));
    replies.0.lock().map_err(|e| e.to_string())?.insert(
        id.to_string(),
        Running {
            view: ReplyView { session_id: id.to_string(), status: "running".into(), ..Default::default() },
            child: Some(child.clone()),
        },
    );

    let msg = message.to_string();
    std::thread::spawn(move || {
        let _ = stdin.write_all(msg.as_bytes()); // dropped → EOF tells claude the prompt is complete
    });

    let (replies, id) = (replies.clone(), id.to_string());
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            replies.update(&id, |v| apply_stream_line(v, &line));
        }
        let mut err = String::new();
        let _ = stderr.read_to_string(&mut err);
        let status = child.lock().ok().and_then(|mut c| c.wait().ok());
        replies.update(&id, |v| {
            if v.status == "running" {
                v.status = "error".into();
                let tail: String = err.trim().chars().rev().take(400).collect::<Vec<_>>().into_iter().rev().collect();
                v.error = if tail.is_empty() { format!("Claude Code exited ({status:?}) without a result.") } else { tail };
            }
        });
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_must_be_uuids() {
        assert!(is_uuid("12345678-90ab-4cde-8f01-23456789abc8"));
        assert!(!is_uuid("12345678-90ab-4cde-8f01-23456789abc"));
        assert!(!is_uuid("12345678-90ab-4cde-8f01-23456789abcg"));
        assert!(!is_uuid("--dangerously-skip-permissions-xxxxxx"));
        assert!(!is_uuid("12345678 90ab-4cde-8f01-23456789abc8"));
    }

    #[test]
    fn folds_stream_json_events() {
        let mut v = ReplyView { status: "running".into(), ..Default::default() };
        apply_stream_line(&mut v, r#"{"type":"system","subtype":"init"}"#);
        apply_stream_line(&mut v, r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":""},{"type":"text","text":"Looking."},{"type":"tool_use","name":"Read","input":{}}]}}"#);
        apply_stream_line(&mut v, "not json");
        assert_eq!((v.text.as_str(), v.activity.clone(), v.status.as_str()), ("Looking.", vec!["Read".to_string()], "running"));
        apply_stream_line(&mut v, r#"{"type":"result","subtype":"success","is_error":false,"result":"All done.","total_cost_usd":0.12,"permission_denials":[{"tool_name":"Bash","tool_input":{}}]}"#);
        assert_eq!(v.text, "All done.");
        assert_eq!(v.status, "done");
        assert_eq!(v.cost_usd, Some(0.12));
        assert_eq!(v.denied, vec!["Bash".to_string()]);
    }

    #[test]
    fn strips_parent_session_env_but_keeps_config_and_auth() {
        for strip in ["CLAUDECODE", "CLAUDE_PID", "CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_MESSAGING_TOKEN", "claude_code_entrypoint", "CLAUDE_AGENT_SDK_VERSION"] {
            assert!(is_inherited_session_var(strip), "{strip}");
        }
        for keep in ["CLAUDE_CONFIG_DIR", "ANTHROPIC_API_KEY", "PATH", "CLAUDE"] {
            assert!(!is_inherited_session_var(keep), "{keep}");
        }
    }

    /// Real end-to-end reply through Claude Code (costs a few cents). Opt-in:
    /// AGENTPULSE_TEST_SESSION=<uuid> AGENTPULSE_TEST_CWD=<folder> cargo test -- --ignored real_reply
    #[test]
    #[ignore]
    fn real_reply() {
        let (Ok(id), Ok(cwd)) = (std::env::var("AGENTPULSE_TEST_SESSION"), std::env::var("AGENTPULSE_TEST_CWD")) else {
            panic!("set AGENTPULSE_TEST_SESSION and AGENTPULSE_TEST_CWD");
        };
        let r = Replies::default();
        start_reply(&r, &id, &cwd, Some("claude-haiku-5-5"), "Reply with exactly the word: pong", "plan").unwrap();
        for _ in 0..240 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let v = r.views().pop().unwrap();
            if v.status != "running" {
                println!("{v:?}");
                assert_eq!(v.status, "done", "{}", v.error);
                assert!(v.text.to_lowercase().contains("pong"), "{}", v.text);
                return;
            }
        }
        panic!("reply timed out");
    }

    #[test]
    fn rejects_bad_reply_input_before_spawning() {
        let r = Replies::default();
        let id = "12345678-90ab-4cde-8f01-23456789abc8";
        assert!(start_reply(&r, "nope", ".", None, "hi", "default").is_err());
        assert!(start_reply(&r, id, ".", None, "   ", "default").is_err());
        assert!(start_reply(&r, id, ".", None, "hi", "bypassPermissions").is_err());
        assert!(start_reply(&r, id, "Z:\\definitely\\missing", None, "hi", "default").is_err());
        assert!(valid_model("claude-opus-5-5"));
        assert!(!valid_model("<synthetic>"));
        assert!(!valid_model("--dangerously-skip-permissions"));
        assert!(r.views().is_empty());
    }
}
