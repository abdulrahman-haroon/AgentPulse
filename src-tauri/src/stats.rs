//! Turns parsed requests + live hook state into everything the UI shows. Pure, so testable.

use crate::actions::ReplyView;
use crate::events::{Live, State};
use crate::logs::{Request, SessionMeta};
use jiff::{SignedDuration, Timestamp, ToSpan, ZonedDateTime};
use serde::Serialize;
use std::collections::HashMap;

pub const HISTORY_DAYS: i64 = 30;
const RECENT_HOURS: i64 = 8;
const MAX_SESSIONS: usize = 15;
const UNFINISHED_TURN_GRACE: SignedDuration = SignedDuration::from_mins(15);

#[derive(Serialize, Debug, Default, Clone)]
pub struct NameValue {
    pub name: String,
    pub cost: f64,
    pub tokens: u64,
}

#[derive(Serialize, Debug)]
pub struct DayPoint {
    pub date: String,
    pub cost: f64,
    pub tokens: u64,
}

#[derive(Serialize, Debug)]
pub struct SessionView {
    pub id: String,
    pub title: String,
    pub project: String,
    /// working | needs_you | done | idle
    pub state: &'static str,
    pub message: String,
    pub started: String,
    pub last_active: String,
    pub state_since: Option<String>,
    pub cost: f64,
    pub requests: u64,
    pub model: String,
    /// "cli", "claude-vscode", ... decides how "Open" works.
    pub entrypoint: String,
}

#[derive(Serialize, Debug)]
pub struct Dashboard {
    pub today_cost: f64,
    pub today_tokens: u64,
    pub today_requests: u64,
    pub today_sessions: usize,
    pub yesterday_cost_by_now: f64,
    pub burn_rate_per_hour: f64,
    pub cache_saved_today: f64,
    pub top_session_today: Option<NameValue>,
    pub models_today: Vec<NameValue>,
    pub daily: Vec<DayPoint>,
    pub projects: Vec<NameValue>,
    /// [weekday Mon=0][hour] request counts over the history window.
    pub heatmap: Vec<[u32; 24]>,
    pub sessions: Vec<SessionView>,
    pub needs_you: usize,
    pub working: usize,
    /// Quick replies sent from AgentPulse (filled in by lib.rs, not computed here).
    pub replies: Vec<ReplyView>,
    /// Permission prompts waiting for an answer here (filled in by lib.rs).
    pub approvals: Vec<ApprovalView>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct ApprovalView {
    pub request_id: String,
    pub session_title: String,
    pub project: String,
    pub tool_name: String,
    /// The part people scan first: the shell command or file path.
    pub summary: String,
    /// The complete tool input, pretty-printed. Always shown in full.
    pub detail: String,
    pub expires_at: String,
}

pub fn approval_views(approvals: &HashMap<String, crate::events::Approval>, titles: &HashMap<&str, &str>) -> Vec<ApprovalView> {
    let mut v: Vec<_> = approvals
        .values()
        .map(|a| {
            let input: serde_json::Value = serde_json::from_str(&a.tool_input).unwrap_or(serde_json::Value::String(a.tool_input.clone()));
            let summary = ["command", "file_path", "url", "pattern", "path"]
                .iter()
                .find_map(|k| input[k].as_str())
                .unwrap_or("")
                .to_string();
            ApprovalView {
                request_id: a.request_id.clone(),
                session_title: titles.get(a.session_id.as_str()).map(|t| t.to_string()).unwrap_or_else(|| "Untitled session".into()),
                project: a.project.clone(),
                tool_name: a.tool_name.clone(),
                summary,
                detail: serde_json::to_string_pretty(&input).unwrap_or_else(|_| a.tool_input.clone()),
                expires_at: a.expires_at.to_string(),
            }
        })
        .collect();
    v.sort_by(|a, b| a.expires_at.cmp(&b.expires_at));
    v
}

struct Agg<'a> {
    first: Timestamp,
    last: Timestamp,
    cost: f64,
    tokens: u64,
    requests: u64,
    project: &'a str,
    model: &'a str,
}

fn add(map: &mut HashMap<String, NameValue>, name: &str, r: &Request) {
    let e = map.entry(name.to_string()).or_insert_with(|| NameValue { name: name.to_string(), ..Default::default() });
    e.cost += r.cost.unwrap_or(0.0);
    e.tokens += r.tokens;
}

fn ranked(map: HashMap<String, NameValue>, n: usize) -> Vec<NameValue> {
    let mut v: Vec<_> = map.into_values().collect();
    v.sort_by(|a, b| b.cost.total_cmp(&a.cost).then(b.tokens.cmp(&a.tokens)));
    v.truncate(n);
    v
}

/// The same request can appear in several files (resumed sessions): keep one copy, the most complete.
fn dedupe<'a>(requests: impl Iterator<Item = &'a Request>) -> Vec<&'a Request> {
    let mut unique: HashMap<&str, &Request> = HashMap::new();
    let mut all: Vec<&Request> = Vec::new();
    for r in requests {
        match r.key.as_deref() {
            Some(k) => {
                let slot = unique.entry(k).or_insert(r);
                if r.output > slot.output {
                    *slot = r;
                }
            }
            None => all.push(r),
        }
    }
    all.extend(unique.into_values());
    all
}

/// One row of the session browser.
#[derive(Serialize, Debug)]
pub struct SessionRow {
    pub id: String,
    pub title: String,
    pub project: String,
    pub started: String,
    pub last_active: String,
    pub active_secs: i64,
    pub requests: u64,
    pub cost: f64,
    pub tokens: u64,
    pub model: String,
    pub entrypoint: String,
    pub last_prompt: String,
}

/// Every session with activity in the history window, newest first.
pub fn history<'a>(
    requests: impl Iterator<Item = &'a Request>,
    titles: &HashMap<&str, &str>,
    meta: &HashMap<&str, &SessionMeta>,
    now: &ZonedDateTime,
) -> Vec<SessionRow> {
    let cutoff = now.timestamp() - SignedDuration::from_hours(24 * HISTORY_DAYS);
    let mut by_session: HashMap<&str, Agg> = HashMap::new();
    for r in dedupe(requests).into_iter().filter(|r| r.ts >= cutoff) {
        let s = by_session.entry(&r.session).or_insert(Agg {
            first: r.ts,
            last: r.ts,
            cost: 0.0,
            tokens: 0,
            requests: 0,
            project: &r.project,
            model: &r.model,
        });
        if r.ts < s.first {
            s.first = r.ts;
            s.project = &r.project;
        }
        if r.ts >= s.last {
            s.last = r.ts;
            s.model = &r.model;
        }
        s.cost += r.cost.unwrap_or(0.0);
        s.tokens += r.tokens;
        s.requests += 1;
    }
    let mut rows: Vec<SessionRow> = by_session
        .into_iter()
        .map(|(id, a)| {
            let m = meta.get(id);
            SessionRow {
                id: id.to_string(),
                title: titles.get(id).map(|t| t.to_string()).unwrap_or_else(|| "Untitled session".into()),
                project: a.project.to_string(),
                started: a.first.to_string(),
                last_active: a.last.to_string(),
                active_secs: a.last.duration_since(a.first).as_secs(),
                requests: a.requests,
                cost: a.cost,
                tokens: a.tokens,
                model: a.model.to_string(),
                entrypoint: m.map(|m| m.entrypoint.clone()).unwrap_or_default(),
                last_prompt: m.map(|m| m.last_prompt.clone()).unwrap_or_default(),
            }
        })
        .collect();
    rows.sort_by(|a, b| b.last_active.cmp(&a.last_active));
    rows
}

pub fn build<'a>(
    requests: impl Iterator<Item = &'a Request>,
    titles: &HashMap<&str, &str>,
    meta: &HashMap<&str, &SessionMeta>,
    live: &HashMap<String, Live>,
    // In the user's time zone: decides what "today", weekdays and hours mean.
    now: &ZonedDateTime,
) -> Dashboard {
    let all = dedupe(requests);

    let tz = now.time_zone().clone();
    let now_ts = now.timestamp();
    let today = now.date();
    let yesterday = today.yesterday().ok();
    let hour_ago = now_ts - SignedDuration::from_hours(1);
    let mut d = Dashboard {
        today_cost: 0.0,
        today_tokens: 0,
        today_requests: 0,
        today_sessions: 0,
        yesterday_cost_by_now: 0.0,
        burn_rate_per_hour: 0.0,
        cache_saved_today: 0.0,
        top_session_today: None,
        models_today: vec![],
        daily: (0..HISTORY_DAYS)
            .rev()
            .map(|i| DayPoint { date: today.checked_sub(i.days()).map(|d| d.to_string()).unwrap_or_default(), cost: 0.0, tokens: 0 })
            .collect(),
        projects: vec![],
        heatmap: vec![[0; 24]; 7],
        sessions: vec![],
        needs_you: 0,
        working: 0,
        replies: vec![],
        approvals: vec![],
    };
    let mut models = HashMap::new();
    let mut projects = HashMap::new();
    let mut today_by_session: HashMap<&str, f64> = HashMap::new();
    let mut sessions: HashMap<&str, Agg> = HashMap::new();

    for r in &all {
        let local = r.ts.to_zoned_datetime(tz.clone());
        let date = local.date();
        let age = date.until(today).map(|s| i64::from(s.get_days())).unwrap_or(-1);
        if !(0..HISTORY_DAYS).contains(&age) {
            continue;
        }
        let cost = r.cost.unwrap_or(0.0);
        let day = &mut d.daily[(HISTORY_DAYS - 1 - age) as usize];
        day.cost += cost;
        day.tokens += r.tokens;
        d.heatmap[local.weekday().to_monday_zero_offset() as usize][local.hour() as usize] += 1;

        let s = sessions.entry(&r.session).or_insert(Agg {
            first: r.ts,
            last: r.ts,
            cost: 0.0,
            tokens: 0,
            requests: 0,
            project: &r.project,
            model: &r.model,
        });
        // A session belongs to the folder it started in, not subfolders it later cd'd into.
        if r.ts < s.first {
            s.first = r.ts;
            s.project = &r.project;
        }
        if r.ts >= s.last {
            s.last = r.ts;
            s.model = &r.model;
        }
        s.cost += cost;
        s.tokens += r.tokens;
        s.requests += 1;

        if date == today {
            d.today_cost += cost;
            d.today_tokens += r.tokens;
            d.today_requests += 1;
            d.cache_saved_today += r.cache_saved;
            *today_by_session.entry(&r.session).or_default() += cost;
            add(&mut models, &r.model, r);
            if r.ts > hour_ago {
                d.burn_rate_per_hour += cost;
            }
        } else if Some(date) == yesterday && local.time() <= now.time() {
            d.yesterday_cost_by_now += cost;
        }
    }

    let title_of = |id: &str| titles.get(id).map(|t| t.to_string()).unwrap_or_else(|| "Untitled session".into());
    d.today_sessions = today_by_session.len();
    d.top_session_today = today_by_session
        .iter()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(id, cost)| NameValue { name: title_of(id), cost: *cost, tokens: 0 });
    d.models_today = ranked(models, 8);
    for s in sessions.values() {
        let p = projects.entry(s.project.to_string()).or_insert_with(|| NameValue { name: s.project.to_string(), ..Default::default() });
        p.cost += s.cost;
        p.tokens += s.tokens;
    }
    d.projects = ranked(projects, 8);

    // Live board: recently active sessions plus any the hooks say are open.
    let recent = now_ts - SignedDuration::from_hours(RECENT_HOURS);
    let mut ids: Vec<&str> = sessions.iter().filter(|(_, a)| a.last >= recent).map(|(id, _)| *id).collect();
    ids.extend(live.iter().filter(|(id, l)| l.state != State::Ended && !sessions.contains_key(id.as_str())).map(|(id, _)| id.as_str()));

    for id in ids {
        let agg = sessions.get(id);
        let hook = live.get(id);
        let last = agg.map(|a| a.last).or(hook.map(|h| h.since)).unwrap_or(now_ts);
        let state = match hook {
            Some(h) if h.state == State::Ended => continue,
            // Log moved on after the permission prompt → it was answered.
            Some(h) if h.state == State::NeedsYou && last > h.since + SignedDuration::from_secs(10) => "working",
            Some(h) => match h.state {
                State::Working => "working",
                State::NeedsYou => "needs_you",
                _ => "done",
            },
            // No hooks: the log says how the last reply ended.
            None if meta.get(id).is_some_and(|m| m.last_stop.as_deref() == Some("end_turn")) => "done",
            // A turn that didn't finish may be a long tool call (a build, tests) that writes nothing
            // while it runs. Call it Working for a while so Reply/Resume can't start a second
            // Claude on a live session.
            None if now_ts.duration_since(last) < UNFINISHED_TURN_GRACE => "working",
            None => "idle",
        };
        d.sessions.push(SessionView {
            id: id.to_string(),
            title: title_of(id),
            project: agg.map(|a| a.project.to_string()).or(hook.map(|h| h.project.clone())).unwrap_or_default(),
            state,
            message: hook.filter(|_| state == "needs_you").map(|h| h.message.clone()).unwrap_or_default(),
            started: agg.map(|a| a.first).unwrap_or(last).to_string(),
            last_active: last.to_string(),
            // When the current state began; only known when hooks report it.
            state_since: hook.map(|h| h.since.to_string()),
            cost: agg.map_or(0.0, |a| a.cost),
            requests: agg.map_or(0, |a| a.requests),
            model: agg.map(|a| a.model.to_string()).unwrap_or_default(),
            entrypoint: meta.get(id).map(|m| m.entrypoint.clone()).unwrap_or_default(),
        });
    }
    let rank = |s: &str| match s {
        "needs_you" => 0,
        "working" => 1,
        "done" => 2,
        _ => 3,
    };
    d.sessions.sort_by(|a, b| rank(a.state).cmp(&rank(b.state)).then(b.last_active.cmp(&a.last_active)));
    d.sessions.truncate(MAX_SESSIONS);
    d.needs_you = d.sessions.iter().filter(|s| s.state == "needs_you").count();
    d.working = d.sessions.iter().filter(|s| s.state == "working").count();
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(key: &str, session: &str, ts: Timestamp, cost: f64, output: u64) -> Request {
        Request {
            key: Some(key.into()),
            ts,
            session: session.into(),
            project: "demo".into(),
            model: "claude-opus-5-5".into(),
            tokens: 100,
            output,
            cost: Some(cost),
            cache_saved: 1.0,
        }
    }

    /// Fixed local noon, so tests never straddle midnight.
    fn noon() -> ZonedDateTime {
        jiff::civil::date(2026, 6, 15).at(12, 0, 0, 0).to_zoned_datetime(jiff::tz::TimeZone::system()).unwrap()
    }

    fn ago(now: &ZonedDateTime, secs: i64) -> Timestamp {
        now.timestamp() - SignedDuration::from_secs(secs)
    }

    fn live(state: State, since: Timestamp) -> Live {
        Live { state, since, project: "demo".into(), message: "Allow Bash?".into() }
    }

    #[test]
    fn totals_dedupe_across_files_and_history() {
        let now = noon();
        let reqs = [
            req("a", "s1", ago(&now, 5 * 60), 2.0, 10),
            req("a", "s1", ago(&now, 5 * 60), 2.0, 10), // same request in a resumed file
            req("b", "s1", ago(&now, 3 * 86_400), 5.0, 10),
            req("c", "s2", ago(&now, 40 * 86_400), 9.0, 10), // outside history
        ];
        let d = build(reqs.iter(), &HashMap::new(), &HashMap::new(), &HashMap::new(), &now);
        assert_eq!(d.today_requests, 1);
        assert_eq!(d.today_cost, 2.0);
        assert_eq!(d.burn_rate_per_hour, 2.0);
        assert_eq!(d.cache_saved_today, 1.0);
        assert_eq!(d.daily.len(), HISTORY_DAYS as usize);
        assert_eq!(d.daily.last().unwrap().cost, 2.0);
        assert_eq!(d.projects[0].cost, 7.0);
        assert_eq!(d.heatmap.iter().flatten().sum::<u32>(), 2);
    }

    /// Opt-in smoke test on the developer's own logs; prints counts only, never content.
    /// cargo test -- --ignored real_history
    #[test]
    #[ignore]
    fn real_history() {
        let home = std::path::PathBuf::from(std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap());
        let mut cache = crate::logs::LogCache::default();
        let since = std::time::SystemTime::now() - std::time::Duration::from_secs(31 * 86_400);
        let t = std::time::Instant::now();
        cache.refresh(&crate::logs::claude_dir(&home).join("projects"), since);
        let rows = history(cache.requests(), &cache.titles(), &cache.meta(), &ZonedDateTime::now());
        let with_prompt = rows.iter().filter(|r| !r.last_prompt.is_empty()).count();
        let titled = rows.iter().filter(|r| r.title != "Untitled session").count();
        println!("sessions={} titled={titled} with_last_prompt={with_prompt} total_cost={:.2} took={:?}", rows.len(), rows.iter().map(|r| r.cost).sum::<f64>(), t.elapsed());
        assert!(rows.windows(2).all(|w| w[0].last_active >= w[1].last_active), "newest first");
    }

    #[test]
    fn history_lists_sessions_newest_first_with_prompt() {
        let now = noon();
        let reqs = [
            req("1", "old", ago(&now, 5 * 86_400), 2.0, 1),
            req("2", "old", ago(&now, 5 * 86_400 - 600), 3.0, 1),
            req("3", "new", ago(&now, 60), 1.0, 1),
            req("3", "new", ago(&now, 60), 1.0, 1), // duplicate copy
            req("4", "ancient", ago(&now, 40 * 86_400), 9.0, 1), // outside window
        ];
        let m = SessionMeta { last_prompt: "fix the login bug".into(), entrypoint: "cli".into(), ..Default::default() };
        let meta = HashMap::from([("old", &m)]);
        let titles = HashMap::from([("old", "Login fix")]);
        let rows = history(reqs.iter(), &titles, &meta, &now);
        assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["new", "old"]);
        let old = &rows[1];
        assert_eq!((old.title.as_str(), old.cost, old.requests, old.active_secs), ("Login fix", 5.0, 2, 600));
        assert_eq!((old.last_prompt.as_str(), old.entrypoint.as_str()), ("fix the login bug", "cli"));
        assert_eq!(rows[0].requests, 1);
    }

    #[test]
    fn session_counts_under_the_folder_it_started_in() {
        let now = noon();
        let mut later = req("2", "s", ago(&now, 1 * 60), 3.0, 1);
        later.project = "src".into(); // Claude cd'd into a subfolder
        let reqs = [later, req("1", "s", ago(&now, 9 * 60), 1.0, 1)];
        let d = build(reqs.iter(), &HashMap::new(), &HashMap::new(), &HashMap::new(), &now);
        assert_eq!(d.projects.len(), 1);
        assert_eq!((d.projects[0].name.as_str(), d.projects[0].cost), ("demo", 4.0));
        assert_eq!(d.sessions[0].project, "demo");
    }

    #[test]
    fn session_states_from_hooks_and_logs() {
        let now = noon();
        let reqs = [
            req("1", "waiting", ago(&now, 3 * 60), 1.0, 1),
            req("2", "answered", ago(&now, 5), 1.0, 1),
            req("3", "quiet", ago(&now, 1 * 3600), 1.0, 1),
            req("4", "busy", ago(&now, 30), 1.0, 1),
            req("7", "long-build", ago(&now, 5 * 60), 1.0, 1), // quiet 5 min mid-turn: a long tool call
            req("5", "closed", ago(&now, 1 * 60), 1.0, 1),
            req("6", "finished", ago(&now, 1 * 3600), 1.0, 1),
        ];
        let finished = SessionMeta { last_stop: Some("end_turn".into()), entrypoint: "cli".into(), ..Default::default() };
        let meta = HashMap::from([("finished", &finished)]);
        let mut l = HashMap::new();
        l.insert("waiting".to_string(), live(State::NeedsYou, ago(&now, 2 * 60)));
        l.insert("answered".to_string(), live(State::NeedsYou, ago(&now, 2 * 60)));
        l.insert("closed".to_string(), live(State::Ended, now.timestamp()));
        l.insert("fresh".to_string(), live(State::Working, now.timestamp())); // hook seen, no log yet
        let titles = HashMap::from([("waiting", "Fix login bug")]);
        let d = build(reqs.iter(), &titles, &meta, &l, &now);

        let state = |id: &str| d.sessions.iter().find(|s| s.id == id).map(|s| s.state);
        assert_eq!(state("waiting"), Some("needs_you"));
        assert_eq!(state("answered"), Some("working"));
        assert_eq!(state("quiet"), Some("idle"));
        assert_eq!(state("busy"), Some("working")); // inferred without hooks
        assert_eq!(state("long-build"), Some("working")); // not Idle, so no Reply/Resume on a live session
        assert_eq!(state("finished"), Some("done")); // last reply ended with end_turn
        assert_eq!(state("fresh"), Some("working"));
        assert_eq!(state("closed"), None);
        assert_eq!(d.sessions[0].title, "Fix login bug"); // needs_you sorts first
        assert_eq!(d.sessions[0].message, "Allow Bash?");
        assert_eq!((d.needs_you, d.working), (1, 4));
    }
}
