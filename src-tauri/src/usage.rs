//! Reads Claude Code session logs (`<claude dir>/projects/**/*.jsonl`) read-only and
//! sums token usage per model for one local day. The log format is undocumented, so
//! everything is optional and bad lines are skipped.

use crate::pricing::{self, Tokens};
use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Deserialize)]
struct LogLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize, Default)]
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

#[derive(Deserialize, Default)]
struct CacheCreation {
    #[serde(default)]
    ephemeral_5m_input_tokens: u64,
    #[serde(default)]
    ephemeral_1h_input_tokens: u64,
}

#[derive(Serialize, Default, Debug, PartialEq)]
pub struct ModelTotals {
    pub model: String,
    pub requests: u64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// None when the model has no known price.
    pub cost_usd: Option<f64>,
}

#[derive(Serialize, Debug)]
pub struct DaySummary {
    pub date: String,
    pub models: Vec<ModelTotals>,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
}

struct Entry {
    model: String,
    tokens: Tokens,
    fast: bool,
}

/// Parses one log line into a (dedupe key, entry) if it is an assistant message on `day`.
fn parse_line(line: &str, day: NaiveDate) -> Option<(Option<String>, Entry)> {
    if !line.contains("\"usage\"") {
        return None; // cheap skip: most lines are large non-usage records
    }
    let l: LogLine = serde_json::from_str(line).ok()?;
    if l.kind.as_deref() != Some("assistant") {
        return None;
    }
    let ts = DateTime::parse_from_rfc3339(l.timestamp.as_deref()?).ok()?;
    if ts.with_timezone(&Local).date_naive() != day {
        return None;
    }
    let m = l.message?;
    let u = m.usage?;
    // Older logs lack the TTL split; treat all cache writes as 5-minute.
    let (w5, w1h) = match u.cache_creation {
        Some(c) if c.ephemeral_5m_input_tokens + c.ephemeral_1h_input_tokens > 0 => {
            (c.ephemeral_5m_input_tokens, c.ephemeral_1h_input_tokens)
        }
        _ => (u.cache_creation_input_tokens, 0),
    };
    // Streaming writes one line per content block with the same message id and request id.
    let key = match (&m.id, &l.request_id) {
        (Some(id), Some(req)) => Some(format!("{id}:{req}")),
        (Some(id), None) => Some(id.clone()),
        _ => None,
    };
    let entry = Entry {
        model: m.model.unwrap_or_else(|| "unknown".into()),
        tokens: Tokens {
            input: u.input_tokens,
            output: u.output_tokens,
            cache_read: u.cache_read_input_tokens,
            cache_write_5m: w5,
            cache_write_1h: w1h,
        },
        fast: u.speed.as_deref() == Some("fast"),
    };
    Some((key, entry))
}

/// Sums usage for `day` from any set of log lines. Pure, so it is unit-testable.
pub fn summarize<S: AsRef<str>>(lines: impl Iterator<Item = S>, day: NaiveDate) -> DaySummary {
    let mut unique: HashMap<String, Entry> = HashMap::new();
    let mut unkeyed: Vec<Entry> = Vec::new();
    for line in lines {
        let Some((key, e)) = parse_line(line.as_ref(), day) else { continue };
        match key {
            // Keep the most complete copy of a streamed message.
            Some(k) => {
                let keep_old = unique.get(&k).is_some_and(|old| old.tokens.output >= e.tokens.output);
                if !keep_old {
                    unique.insert(k, e);
                }
            }
            None => unkeyed.push(e),
        }
    }

    let mut by_model: HashMap<String, ModelTotals> = HashMap::new();
    for e in unique.into_values().chain(unkeyed) {
        let t = by_model.entry(e.model.clone()).or_insert_with(|| ModelTotals {
            model: e.model.clone(),
            cost_usd: Some(0.0),
            ..Default::default()
        });
        t.requests += 1;
        t.input += e.tokens.input;
        t.output += e.tokens.output;
        t.cache_read += e.tokens.cache_read;
        t.cache_write += e.tokens.cache_write_5m + e.tokens.cache_write_1h;
        t.cost_usd = match (t.cost_usd, pricing::cost(&e.model, &e.tokens, e.fast)) {
            (Some(a), Some(b)) => Some(a + b),
            _ => None,
        };
    }

    let mut models: Vec<ModelTotals> = by_model.into_values().collect();
    models.sort_by(|a, b| b.cost_usd.unwrap_or(0.0).total_cmp(&a.cost_usd.unwrap_or(0.0)));
    DaySummary {
        date: day.to_string(),
        total_tokens: models.iter().map(|m| m.input + m.output + m.cache_read + m.cache_write).sum(),
        total_cost_usd: models.iter().filter_map(|m| m.cost_usd).sum(),
        models,
    }
}

/// `CLAUDE_CONFIG_DIR` if set, else `~/.claude`.
pub fn claude_dir(home: &Path) -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home.join(".claude"))
}

/// Summarizes today's usage from every log file touched since local midnight.
pub fn today(claude_dir: &Path) -> DaySummary {
    let now = Local::now();
    let midnight: SystemTime = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|m| m.and_local_timezone(Local).earliest())
        .map(Into::into)
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let mut files = Vec::new();
    collect_jsonl(&claude_dir.join("projects"), midnight, &mut files, 0);

    // Streams line by line: session logs can be hundreds of MB.
    let lines = files
        .into_iter()
        .filter_map(|f| File::open(f).ok())
        .flat_map(|file| BufReader::new(file).lines().map_while(Result::ok));
    summarize(lines, now.date_naive())
}

/// Recursively finds `.jsonl` files modified since `since`. Does not follow symlinks.
fn collect_jsonl(dir: &Path, since: SystemTime, out: &mut Vec<PathBuf>, depth: u8) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            collect_jsonl(&path, since, out, depth + 1);
        } else if ft.is_file()
            && path.extension().is_some_and(|e| e == "jsonl")
            && entry.metadata().and_then(|m| m.modified()).is_ok_and(|t| t >= since)
        {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic fixture; never use real session logs in tests.
    fn line(id: &str, req: &str, ts: &str, model: &str, input: u64, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","requestId":"{req}","message":{{"id":"{id}","model":"{model}","usage":{{"input_tokens":{input},"output_tokens":{output},"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
        )
    }

    fn day_of(ts: &str) -> NaiveDate {
        DateTime::parse_from_rfc3339(ts).unwrap().with_timezone(&Local).date_naive()
    }

    const TS: &str = "2026-06-15T12:00:00.000Z";

    #[test]
    fn dedupes_streamed_copies_keeping_the_complete_one() {
        let lines = [
            line("msg_1", "req_1", TS, "claude-opus-5-5", 1000, 10),
            line("msg_1", "req_1", TS, "claude-opus-5-5", 1000, 500),
            line("msg_2", "req_2", TS, "claude-opus-5-5", 2000, 100),
        ];
        let s = summarize(lines.iter().map(String::as_str), day_of(TS));
        assert_eq!(s.models.len(), 1);
        let m = &s.models[0];
        assert_eq!((m.requests, m.input, m.output), (2, 3000, 600));
        // 3000 * $4 + 600 * $20 per MTok
        assert!((s.total_cost_usd - 0.024).abs() < 1e-9);
    }

    #[test]
    fn skips_other_days_bad_lines_and_non_assistant_lines() {
        let lines = [
            line("a", "r", TS, "claude-opus-5-5", 1, 1),
            line("b", "r", "2026-06-10T12:00:00.000Z", "claude-opus-5-5", 999, 999),
            "{not json \"usage\"".to_string(),
            r#"{"type":"user","usage":{}}"#.to_string(),
        ];
        let s = summarize(lines.iter().map(String::as_str), day_of(TS));
        assert_eq!(s.models[0].requests, 1);
    }

    #[test]
    fn unknown_model_counts_tokens_but_has_no_cost() {
        let lines = [line("a", "r", TS, "<synthetic>", 5, 5)];
        let s = summarize(lines.iter().map(String::as_str), day_of(TS));
        assert_eq!(s.models[0].cost_usd, None);
        assert_eq!(s.total_tokens, 10);
        assert_eq!(s.total_cost_usd, 0.0);
    }

    #[test]
    fn splits_cache_writes_by_ttl() {
        let l = format!(
            r#"{{"type":"assistant","timestamp":"{TS}","requestId":"r","message":{{"id":"m","model":"claude-opus-5-5","usage":{{"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":2000000,"cache_creation":{{"ephemeral_5m_input_tokens":1000000,"ephemeral_1h_input_tokens":1000000}}}}}}}}"#
        );
        let s = summarize(std::iter::once(l.as_str()), day_of(TS));
        // 1M * $4 * 1.25 + 1M * $4 * 2 = $13
        assert!((s.total_cost_usd - 13.0).abs() < 1e-9);
    }
}
