//! Reads Claude Code session logs (`<claude dir>/projects/**/*.jsonl`) read-only.
//! Files are parsed incrementally: each refresh reads only bytes appended since the last
//! one, so hundreds of MB of history are parsed once, then just tailed.
//! The format is undocumented, so every field is optional and bad lines are skipped.

use crate::pricing::{self, Tokens};
use chrono::{DateTime, Local};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One billed API request, already priced.
#[derive(Debug, Clone)]
pub struct Request {
    /// `message.id:requestId`; the same request can appear in several files (resumed sessions).
    pub key: Option<String>,
    pub ts: DateTime<Local>,
    pub session: String,
    pub project: String,
    pub model: String,
    pub tokens: u64,
    pub output: u64,
    pub cost: Option<f64>,
    pub cache_saved: f64,
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
    pub last_ts: Option<DateTime<Local>>,
    /// Model of the latest assistant message, so replies continue on the same model.
    pub model: Option<String>,
}

#[derive(Default)]
pub struct FileData {
    pub requests: Vec<Request>,
    pub titles: HashMap<String, String>,
    pub meta: HashMap<String, SessionMeta>,
    by_key: HashMap<String, usize>,
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
    #[serde(rename = "aiTitle")]
    ai_title: Option<String>,
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
        // Cheap skip: most lines are large records with neither usage nor a title.
        if !line.contains("\"usage\"") && !line.contains("\"ai-title\"") {
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
            Some("assistant") => self.ingest_assistant(l, session),
            _ => {}
        }
    }

    fn ingest_assistant(&mut self, l: LogLine, session: String) {
        let Some(ts) = l.timestamp.as_deref().and_then(|t| DateTime::parse_from_rfc3339(t).ok()) else { return };
        let ts = ts.with_timezone(&Local);
        let Some(m) = l.message else { return };

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
            key: key.clone(),
            ts,
            session,
            project: l.cwd.as_deref().map(project_name).unwrap_or_else(|| "unknown".into()),
            tokens: t.input + t.output + t.cache_read + t.cache_write_5m + t.cache_write_1h,
            output: t.output,
            cost: pricing::cost(&model, &t, fast),
            cache_saved: pricing::cache_savings(&model, &t, fast),
            model,
        };
        // Streaming writes one line per content block; keep the most complete copy.
        match key {
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

impl LogCache {
    /// Picks up new and grown files modified since `since`; forgets files that disappeared.
    pub fn refresh(&mut self, projects_dir: &Path, since: SystemTime) {
        let mut found = Vec::new();
        collect_jsonl(projects_dir, since, &mut found, 0);
        let live: HashSet<&PathBuf> = found.iter().map(|(p, _)| p).collect();
        self.files.retain(|p, _| live.contains(p));

        for (path, len) in &found {
            let entry = self.files.entry(path.clone()).or_insert_with(|| CachedFile { offset: 0, data: FileData::default() });
            if *len < entry.offset {
                *entry = CachedFile { offset: 0, data: FileData::default() }; // rewritten: start over
            }
            if *len == entry.offset {
                continue;
            }
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if let Ok(off) = tail(path, entry.offset, &mut entry.data, &stem) {
                entry.offset = off;
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
            if m.last_ts > slot.last_ts {
                *slot = m;
            }
        }
        out
    }
}

/// Parses complete lines from `offset` on; returns the new offset. A trailing partial
/// line (Claude Code mid-write) is left for the next refresh.
fn tail(path: &Path, offset: u64, data: &mut FileData, stem: &str) -> std::io::Result<u64> {
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
            data.ingest(s.trim_end(), stem);
        }
    }
}

/// Recursively finds `.jsonl` files modified since `since`, with their sizes. No symlinks.
fn collect_jsonl(dir: &Path, since: SystemTime, out: &mut Vec<(PathBuf, u64)>, depth: u8) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            collect_jsonl(&path, since, out, depth + 1);
        } else if ft.is_file() && path.extension().is_some_and(|e| e == "jsonl") {
            if let Ok(m) = entry.metadata() {
                if m.modified().is_ok_and(|t| t >= since) {
                    out.push((path, m.len()));
                }
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
        let off = tail(&p, 0, &mut d, "s").unwrap();
        assert_eq!(off, l1.len() as u64 + 1);
        assert_eq!(d.requests.len(), 1);
        let off2 = tail(&p, off, &mut d, "s").unwrap();
        assert_eq!(off2, off); // partial line not consumed
        fs::remove_dir_all(&dir).unwrap();
    }
}
