//! Reads coding-agent session logs read-only: Claude Code (`<claude dir>/projects/**/*.jsonl`,
//! parsed here), Codex (`codex.rs`) and Copilot CLI (`copilot.rs`).
//! Files are parsed incrementally: each refresh reads only bytes appended since the last
//! one, so hundreds of MB of history are parsed once, then just tailed.
//! The formats are undocumented, so every field is optional and bad lines are skipped.

use crate::pricing::{self, Tokens};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Which coding agent wrote a log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    #[default]
    Claude,
    Codex,
    Copilot,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Provider::Claude => "Claude Code",
            Provider::Codex => "Codex",
            Provider::Copilot => "Copilot",
        }
    }
}

/// One billed API request, already priced.
#[derive(Debug, Clone)]
pub struct Request {
    pub provider: Provider,
    /// `message.id:requestId`; the same request can appear in several files (resumed sessions).
    pub key: Option<String>,
    pub ts: Timestamp,
    pub session: String,
    pub project: String,
    pub model: String,
    pub tokens: u64,
    pub output: u64,
    pub cost: Option<f64>,
    pub cache_saved: f64,
    /// GitHub AI credits this request used (Copilot only; 0 for other agents).
    pub ai_credits: f64,
}

/// Where a session lives and how its last reply ended.
#[derive(Debug, Clone, Default)]
pub struct SessionMeta {
    /// Full folder the session started in.
    pub cwd: String,
    /// e.g. "cli" or "claude-vscode".
    pub entrypoint: String,
    /// stop_reason of the latest assistant message ("end_turn" = finished, waiting for you).
    pub last_stop: Option<String>,
    pub last_ts: Option<Timestamp>,
    /// Model of the latest assistant message, so replies continue on the same model.
    pub model: Option<String>,
    /// The user's most recent prompt (Claude Code's `last-prompt` record), capped. For search.
    pub last_prompt: String,
    pub provider: Provider,
    /// The agent says the session closed (Copilot's `session.shutdown`). Claude uses the SessionEnd hook.
    pub ended: bool,
}

/// A sub-agent (Claude Code Task/Agent tool, Copilot subagent) working for a session.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubAgent {
    pub id: String,
    /// The session that started it.
    pub session: String,
    /// e.g. "general-purpose", "Explore".
    pub kind: String,
    pub description: String,
    /// Runs on while the main session does other things (or has finished its turn).
    pub background: bool,
    pub finished: bool,
    pub model: String,
    pub started: Option<Timestamp>,
    /// Last write to its transcript.
    pub last: Option<Timestamp>,
    pub cost: f64,
}

/// Plan usage. Reported by the agent itself (Codex logs, Claude Code's status line) unless
/// `estimate` is set (Copilot: logged credits against the plan the user picked).
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct PlanLimit {
    pub provider: Provider,
    /// "5-hour", "Weekly", ...
    pub window: String,
    pub used_percent: f64,
    pub resets_at: Option<String>,
    /// e.g. "4,210 of 7,000 credits" or "$314 of $500".
    pub detail: Option<String>,
    /// When the agent last reported this number (None = computed just now).
    pub as_of: Option<String>,
    pub estimate: bool,
}

pub const MAX_PROMPT_CHARS: usize = 300;
const MAX_TITLE_CHARS: usize = 80;

/// One line, whitespace collapsed (control chars go with it), capped: a preview, not a transcript.
pub fn one_line(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = flat.chars().filter(|c| !c.is_control()).take(max).collect();
    if flat.chars().count() > max {
        out.push('…');
    }
    out
}

/// A title from the first thing the user asked (for agents that don't write titles).
pub fn title_from(prompt: &str) -> String {
    one_line(prompt, MAX_TITLE_CHARS)
}

#[derive(Default)]
pub struct FileData {
    pub requests: Vec<Request>,
    pub titles: HashMap<String, String>,
    pub meta: HashMap<String, SessionMeta>,
    pub agents: HashMap<String, SubAgent>,
    /// Newest plan limits seen in this file, with when they were reported.
    pub limits: Option<(Timestamp, Vec<PlanLimit>)>,
    by_key: HashMap<String, usize>,
    /// Parser state for formats that name the session/model once, not per line (Codex, Copilot).
    pub(crate) session: Option<String>,
    pub(crate) model: Option<String>,
    /// Codex's running token total, so repeated snapshots aren't counted twice.
    pub(crate) total: u64,
}

#[derive(Deserialize)]
struct LogLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    cwd: Option<String>,
    entrypoint: Option<String>,
    /// Sub-agent lines carry the parent's sessionId; they must not overwrite its metadata.
    #[serde(rename = "isSidechain")]
    is_sidechain: Option<bool>,
    #[serde(rename = "aiTitle")]
    ai_title: Option<String>,
    #[serde(rename = "lastPrompt")]
    last_prompt: Option<String>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    model: Option<String>,
    stop_reason: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    cache_creation: Option<CacheCreation>,
    speed: Option<String>,
}

#[derive(Deserialize)]
struct CacheCreation {
    #[serde(default)]
    ephemeral_5m_input_tokens: u64,
    #[serde(default)]
    ephemeral_1h_input_tokens: u64,
}

/// Last path component of a cwd, for both `\` and `/` separators.
pub fn project_name(cwd: &str) -> String {
    cwd.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().filter(|s| !s.is_empty()).unwrap_or("unknown").to_string()
}

impl FileData {
    /// Adds one log line. `fallback_session` is the file stem, used when a line has no sessionId.
    pub fn ingest(&mut self, line: &str, fallback_session: &str) {
        // Cheap skip: most lines are large records with no usage, title or prompt summary.
        if !line.contains("\"usage\"") && !line.contains("\"ai-title\"") && !line.contains("\"last-prompt\"") {
            return;
        }
        let Ok(l) = serde_json::from_str::<LogLine>(line) else { return };
        let session = l.session_id.clone().unwrap_or_else(|| fallback_session.to_string());
        match l.kind.as_deref() {
            Some("ai-title") => {
                if let Some(t) = l.ai_title {
                    self.titles.insert(session, t);
                }
            }
            Some("last-prompt") => {
                if let Some(p) = l.last_prompt {
                    self.meta.entry(session).or_default().last_prompt = one_line(&p, MAX_PROMPT_CHARS);
                }
            }
            Some("assistant") => self.ingest_assistant(l, session),
            _ => {}
        }
    }

    fn ingest_assistant(&mut self, l: LogLine, session: String) {
        let Some(ts) = l.timestamp.as_deref().and_then(|t| t.parse::<Timestamp>().ok()) else { return };
        let Some(m) = l.message else { return };

        if l.is_sidechain != Some(true) {
            let meta = self.meta.entry(session.clone()).or_default();
            if meta.cwd.is_empty() {
                meta.cwd = l.cwd.clone().unwrap_or_default(); // lines are chronological: first = start folder
            }
            if meta.entrypoint.is_empty() {
                meta.entrypoint = l.entrypoint.unwrap_or_default();
            }
            // Streaming copies of one message carry the final stop_reason only on some lines.
            if meta.last_ts.is_none_or(|t| ts > t) || (Some(ts) == meta.last_ts && m.stop_reason.is_some()) {
                meta.last_ts = Some(ts);
                meta.last_stop = m.stop_reason.clone();
                meta.model = m.model.clone();
            }
        }

        let Some(u) = m.usage else { return };
        // Older logs lack the TTL split; treat all cache writes as 5-minute.
        let (w5, w1h) = match u.cache_creation {
            Some(c) if c.ephemeral_5m_input_tokens + c.ephemeral_1h_input_tokens > 0 => {
                (c.ephemeral_5m_input_tokens, c.ephemeral_1h_input_tokens)
            }
            _ => (u.cache_creation_input_tokens, 0),
        };
        let t = Tokens {
            input: u.input_tokens,
            output: u.output_tokens,
            cache_read: u.cache_read_input_tokens,
            cache_write_5m: w5,
            cache_write_1h: w1h,
        };
        let model = m.model.unwrap_or_else(|| "unknown".into());
        let fast = u.speed.as_deref() == Some("fast");
        let key = match (&m.id, &l.request_id) {
            (Some(id), Some(req)) => Some(format!("{id}:{req}")),
            (Some(id), None) => Some(id.clone()),
            _ => None,
        };
        let req = Request {
            provider: Provider::Claude,
            key,
            ts,
            session,
            project: l.cwd.as_deref().map(project_name).unwrap_or_else(|| "unknown".into()),
            tokens: t.input + t.output + t.cache_read + t.cache_write_5m + t.cache_write_1h,
            output: t.output,
            cost: pricing::cost(&model, &t, fast),
            cache_saved: pricing::cache_savings(&model, &t, fast),
            ai_credits: 0.0,
            model,
        };
        self.push_request(req);
    }

    /// Adds a request; streamed copies (same key) keep the most complete one.
    pub(crate) fn push_request(&mut self, req: Request) {
        match req.key.clone() {
            Some(k) => match self.by_key.get(&k) {
                Some(&i) if req.output > self.requests[i].output => self.requests[i] = req,
                Some(_) => {}
                None => {
                    self.by_key.insert(k, self.requests.len());
                    self.requests.push(req);
                }
            },
            None => self.requests.push(req),
        }
    }
}

struct CachedFile {
    /// Bytes consumed so far (always ends after a newline).
    offset: u64,
    data: FileData,
}

#[derive(Default)]
pub struct LogCache {
    files: HashMap<PathBuf, CachedFile>,
}

/// Where each agent keeps its session logs, honoring the same env overrides the agents do.
pub fn roots(home: &Path) -> Vec<(PathBuf, Provider)> {
    let dir = |var: &str, default: &str| std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| home.join(default));
    vec![
        (claude_dir(home).join("projects"), Provider::Claude),
        (dir("CODEX_HOME", ".codex").join("sessions"), Provider::Codex),
        (dir("COPILOT_HOME", ".copilot").join("session-state"), Provider::Copilot),
    ]
}

/// Session id for lines that don't name one: the file stem, or the folder for Copilot's `<id>/events.jsonl`.
fn file_session(path: &Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if stem != "events" {
        return stem;
    }
    path.parent().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or(stem)
}

/// `<project>/<session>/subagents/agent-<id>.jsonl` → (agent id, parent session id).
fn claude_subagent(path: &Path) -> Option<(String, String)> {
    let dir = path.parent()?;
    if dir.file_name()? != "subagents" {
        return None;
    }
    let id = path.file_stem()?.to_str()?.strip_prefix("agent-")?.to_string();
    let parent = dir.parent()?.file_name()?.to_str()?.to_string();
    Some((id, parent))
}

/// The fields AgentPulse uses from Claude Code's `agent-<id>.meta.json`.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct AgentMetaFile {
    agent_type: String,
    description: String,
    /// "foreground" | "background"
    request_shape: String,
    finished: bool,
    model: String,
}

/// Sub-agent state from its transcript and `.meta.json`. The meta file is re-read until it says
/// finished (only for agents active in the last day, so a crashed one isn't polled forever).
fn update_subagent(d: &mut FileData, path: &Path, id: String, parent: String, modified: Timestamp, now: Timestamp) {
    let cost = d.requests.iter().filter_map(|r| r.cost).sum();
    let started = d.requests.iter().map(|r| r.ts).min().unwrap_or(modified);
    let a = d.agents.entry(id.clone()).or_insert_with(|| SubAgent { id, session: parent, ..Default::default() });
    a.cost = cost;
    a.started = Some(a.started.map_or(started, |s| s.min(started)));
    a.last = Some(modified);
    if a.finished || now.duration_since(modified) > jiff::SignedDuration::from_hours(24) {
        return;
    }
    let Some(m) = fs::read_to_string(path.with_extension("meta.json")).ok().and_then(|t| serde_json::from_str::<AgentMetaFile>(&t).ok()) else {
        return;
    };
    a.kind = one_line(&m.agent_type, 40);
    a.description = one_line(&m.description, 120);
    a.background = m.request_shape == "background";
    a.finished = m.finished;
    a.model = one_line(&m.model, 64);
}

impl LogCache {
    /// Picks up new and grown files modified since `since`; forgets files that disappeared.
    pub fn refresh(&mut self, roots: &[(PathBuf, Provider)], since: SystemTime) {
        let mut found = Vec::new();
        for (dir, provider) in roots {
            collect_jsonl(dir, since, *provider, &mut found, 0);
        }
        let live: HashSet<&PathBuf> = found.iter().map(|f| &f.path).collect();
        self.files.retain(|p, _| live.contains(p));

        let now = Timestamp::now();
        for f in &found {
            let entry = self.files.entry(f.path.clone()).or_insert_with(|| CachedFile { offset: 0, data: FileData::default() });
            if f.len < entry.offset {
                *entry = CachedFile { offset: 0, data: FileData::default() }; // rewritten: start over
            }
            if f.len != entry.offset {
                if let Ok(off) = tail(&f.path, entry.offset, &mut entry.data, &file_session(&f.path), f.provider) {
                    entry.offset = off;
                }
            }
            // Codex/Copilot log many events we skip (tool runs); the file's write time says the
            // session is still active. Their files hold one session each.
            if f.provider != Provider::Claude {
                for m in entry.data.meta.values_mut() {
                    m.last_ts = Some(m.last_ts.map_or(f.modified, |t| t.max(f.modified)));
                }
            }
            if f.provider == Provider::Claude {
                if let Some((id, parent)) = claude_subagent(&f.path) {
                    update_subagent(&mut entry.data, &f.path, id, parent, f.modified, now);
                }
            }
        }
    }

    pub fn requests(&self) -> impl Iterator<Item = &Request> {
        self.files.values().flat_map(|f| f.data.requests.iter())
    }

    pub fn titles(&self) -> HashMap<&str, &str> {
        self.files.values().flat_map(|f| f.data.titles.iter().map(|(k, v)| (k.as_str(), v.as_str()))).collect()
    }

    /// Per-session metadata; when a session spans files (resumed), the most recent one wins.
    pub fn meta(&self) -> HashMap<&str, &SessionMeta> {
        let mut out: HashMap<&str, &SessionMeta> = HashMap::new();
        for (id, m) in self.files.values().flat_map(|f| f.data.meta.iter()) {
            let slot = out.entry(id.as_str()).or_insert(m);
            // Prefer the copy with the latest activity; a prompt-only copy never wins over a real one.
            if m.last_ts > slot.last_ts {
                *slot = m;
            }
        }
        out
    }

    pub fn agents(&self) -> Vec<&SubAgent> {
        self.files.values().flat_map(|f| f.data.agents.values()).collect()
    }

    /// The newest plan limits any log reported, minus windows that have reset since (their usage is unknown).
    pub fn limits(&self, now: Timestamp) -> Vec<PlanLimit> {
        let newest = self.files.values().filter_map(|f| f.data.limits.as_ref()).max_by_key(|(ts, _)| *ts);
        newest
            .map(|(at, l)| {
                l.iter()
                    .filter(|l| l.resets_at.as_deref().and_then(|t| t.parse::<Timestamp>().ok()).is_none_or(|t| t > now))
                    .map(|l| PlanLimit { as_of: Some(at.to_string()), ..l.clone() })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Parses complete lines from `offset` on; returns the new offset. A trailing partial
/// line (the agent mid-write) is left for the next refresh.
fn tail(path: &Path, offset: u64, data: &mut FileData, session: &str, provider: Provider) -> std::io::Result<u64> {
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(offset))?;
    let mut r = BufReader::new(f);
    let mut buf = Vec::new();
    let mut pos = offset;
    loop {
        buf.clear();
        let n = r.read_until(b'\n', &mut buf)?;
        if n == 0 || buf.last() != Some(&b'\n') {
            return Ok(pos);
        }
        pos += n as u64;
        if let Ok(s) = std::str::from_utf8(&buf) {
            let line = s.trim_end();
            match provider {
                Provider::Claude => data.ingest(line, session),
                Provider::Codex => crate::codex::ingest(data, line, session),
                Provider::Copilot => crate::copilot::ingest(data, line, session),
            }
        }
    }
}

struct Found {
    path: PathBuf,
    len: u64,
    modified: Timestamp,
    provider: Provider,
}

/// Session logs only: Codex `rollout-*.jsonl`; Copilot `<id>/events.jsonl` or the older top-level
/// `<id>.jsonl` (its session folders hold other files too). Claude: every `.jsonl`.
fn is_session_log(provider: Provider, path: &Path, depth: u8) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    match provider {
        Provider::Claude => true,
        Provider::Codex => name.starts_with("rollout-"),
        Provider::Copilot => depth == 0 || name == "events.jsonl",
    }
}

/// Recursively finds `.jsonl` files modified since `since`, with their sizes. No symlinks.
fn collect_jsonl(dir: &Path, since: SystemTime, provider: Provider, out: &mut Vec<Found>, depth: u8) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            collect_jsonl(&path, since, provider, out, depth + 1);
        } else if ft.is_file() && path.extension().is_some_and(|e| e == "jsonl") && is_session_log(provider, &path, depth) {
            let Ok(m) = entry.metadata() else { continue };
            let Ok(t) = m.modified() else { continue };
            if t >= since {
                let modified = Timestamp::try_from(t).unwrap_or_else(|_| Timestamp::now());
                out.push(Found { path, len: m.len(), modified, provider });
            }
        }
    }
}

/// `CLAUDE_CONFIG_DIR` if set, else `~/.claude`.
pub fn claude_dir(home: &Path) -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home.join(".claude"))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    // Synthetic fixtures only; never real session logs.
    pub fn line(id: &str, req: &str, ts: &str, model: &str, input: u64, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","requestId":"{req}","sessionId":"s1","cwd":"C:\\code\\demo","message":{{"id":"{id}","model":"{model}","usage":{{"input_tokens":{input},"output_tokens":{output},"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
        )
    }

    const TS: &str = "2026-06-15T12:00:00.000Z";

    #[test]
    fn dedupes_streamed_copies_keeping_the_complete_one() {
        let mut d = FileData::default();
        d.ingest(&line("m1", "r1", TS, "claude-opus-5-5", 1000, 10), "f");
        d.ingest(&line("m1", "r1", TS, "claude-opus-5-5", 1000, 500), "f");
        d.ingest(&line("m2", "r2", TS, "claude-opus-5-5", 2000, 100), "f");
        assert_eq!(d.requests.len(), 2);
        assert_eq!(d.requests[0].output, 500);
        assert_eq!(d.requests[0].project, "demo");
        assert_eq!(d.requests[0].session, "s1");
    }

    #[test]
    fn reads_titles_and_skips_junk() {
        let mut d = FileData::default();
        d.ingest(r#"{"type":"ai-title","aiTitle":"Fix login bug","sessionId":"s9"}"#, "f");
        d.ingest("{not json \"usage\"", "f");
        d.ingest(r#"{"type":"user","usage":{}}"#, "f");
        assert_eq!(d.titles.get("s9").map(String::as_str), Some("Fix login bug"));
        assert!(d.requests.is_empty());
    }

    #[test]
    fn splits_cache_writes_by_ttl() {
        let mut d = FileData::default();
        d.ingest(&format!(r#"{{"type":"assistant","timestamp":"{TS}","requestId":"r","message":{{"id":"m","model":"claude-opus-5-5","usage":{{"cache_creation_input_tokens":2000000,"cache_creation":{{"ephemeral_5m_input_tokens":1000000,"ephemeral_1h_input_tokens":1000000}}}}}}}}"#), "f");
        // 1M * $4 * 1.25 + 1M * $4 * 2 = $13
        assert!((d.requests[0].cost.unwrap() - 13.0).abs() < 1e-9);
        assert_eq!(d.requests[0].session, "f"); // fallback to file stem
    }

    #[test]
    fn records_session_meta_from_first_and_latest_lines() {
        let mut d = FileData::default();
        let first = r#"{"type":"assistant","timestamp":"2026-06-15T12:00:00Z","sessionId":"s1","cwd":"C:\\code\\demo","entrypoint":"cli","message":{"id":"a","stop_reason":"tool_use","usage":{}}}"#;
        let later = r#"{"type":"assistant","timestamp":"2026-06-15T12:05:00Z","sessionId":"s1","cwd":"C:\\code\\demo\\src","entrypoint":"cli","message":{"id":"b","model":"claude-haiku-5-5","stop_reason":"end_turn","usage":{}}}"#;
        d.ingest(first, "f");
        d.ingest(later, "f");
        let m = &d.meta["s1"];
        assert_eq!(m.cwd, r"C:\code\demo"); // start folder, not the later subfolder
        assert_eq!(m.entrypoint, "cli");
        assert_eq!(m.last_stop.as_deref(), Some("end_turn"));
        assert_eq!(m.model.as_deref(), Some("claude-haiku-5-5"));
    }

    #[test]
    fn keeps_last_prompt_as_a_short_single_line() {
        let mut d = FileData::default();
        let long = format!("fix the\nlogin   bug {}", "x".repeat(500));
        d.ingest(&serde_json::json!({"type":"last-prompt","lastPrompt":long,"sessionId":"s1"}).to_string(), "f");
        let p = &d.meta["s1"].last_prompt;
        assert!(p.starts_with("fix the login bug x"));
        assert_eq!(p.chars().count(), MAX_PROMPT_CHARS + 1); // + "…"
    }

    #[test]
    fn subagent_lines_count_but_never_overwrite_the_parent_session() {
        let mut d = FileData::default();
        let main = r#"{"type":"assistant","timestamp":"2026-06-15T12:00:00Z","sessionId":"s1","cwd":"C:\\code\\demo","message":{"id":"a","model":"claude-opus-5-5","stop_reason":"tool_use","usage":{"output_tokens":5}}}"#;
        let side = r#"{"type":"assistant","isSidechain":true,"timestamp":"2026-06-15T12:01:00Z","sessionId":"s1","cwd":"C:\\code\\demo","message":{"id":"b","model":"claude-haiku-5-5","stop_reason":"end_turn","usage":{"output_tokens":5}}}"#;
        d.ingest(main, "f");
        d.ingest(side, "f");
        assert_eq!(d.requests.len(), 2); // the sub-agent's usage still counts for the session
        let m = &d.meta["s1"];
        assert_eq!((m.model.as_deref(), m.last_stop.as_deref()), (Some("claude-opus-5-5"), Some("tool_use")));
    }

    #[test]
    fn finds_subagents_and_session_ids_from_paths() {
        let p = Path::new("projects/C--code-demo/1234/subagents/agent-a6ae9f.jsonl");
        assert_eq!(claude_subagent(p), Some(("a6ae9f".into(), "1234".into())));
        assert_eq!(claude_subagent(Path::new("projects/C--code-demo/1234.jsonl")), None);
        assert_eq!(file_session(Path::new("session-state/abc/events.jsonl")), "abc");
        assert_eq!(file_session(Path::new("session-state/abc.jsonl")), "abc");
        assert!(is_session_log(Provider::Copilot, Path::new("abc/events.jsonl"), 1));
        assert!(is_session_log(Provider::Copilot, Path::new("abc.jsonl"), 0));
        assert!(!is_session_log(Provider::Copilot, Path::new("abc/checkpoints.jsonl"), 1));
        assert!(is_session_log(Provider::Codex, Path::new("2026/06/15/rollout-x.jsonl"), 3));
        assert!(!is_session_log(Provider::Codex, Path::new("history.jsonl"), 0));
    }

    #[test]
    fn subagent_meta_is_read_until_finished() {
        let dir = std::env::temp_dir().join(format!("agentpulse-agents-{}", std::process::id())).join("s1").join("subagents");
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("agent-x1.jsonl");
        fs::write(&p, "").unwrap();
        let meta = |finished: bool| format!(r#"{{"agentType":"general-purpose","description":"Read the docs","requestShape":"background","finished":{finished},"model":"claude-haiku-5-5"}}"#);
        fs::write(dir.join("agent-x1.meta.json"), meta(false)).unwrap();
        let now = Timestamp::now();
        let mut d = FileData::default();
        update_subagent(&mut d, &p, "x1".into(), "s1".into(), now, now);
        let a = &d.agents["x1"];
        assert_eq!((a.kind.as_str(), a.description.as_str(), a.background, a.finished), ("general-purpose", "Read the docs", true, false));
        fs::write(dir.join("agent-x1.meta.json"), meta(true)).unwrap();
        update_subagent(&mut d, &p, "x1".into(), "s1".into(), now, now);
        assert!(d.agents["x1"].finished);
        fs::remove_dir_all(dir.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn project_name_handles_both_separators() {
        assert_eq!(project_name(r"C:\code\demo\"), "demo");
        assert_eq!(project_name("/home/u/web-app"), "web-app");
        assert_eq!(project_name(""), "unknown");
    }

    #[test]
    fn tail_reads_only_complete_new_lines() {
        let dir = std::env::temp_dir().join(format!("agentpulse-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        let l1 = line("m1", "r1", TS, "claude-opus-5-5", 1, 1);
        fs::write(&p, format!("{l1}\n{{\"partial")).unwrap();
        let mut d = FileData::default();
        let off = tail(&p, 0, &mut d, "s", Provider::Claude).unwrap();
        assert_eq!(off, l1.len() as u64 + 1);
        assert_eq!(d.requests.len(), 1);
        let off2 = tail(&p, off, &mut d, "s", Provider::Claude).unwrap();
        assert_eq!(off2, off); // partial line not consumed
        fs::remove_dir_all(&dir).unwrap();
    }
}
