//! Turns parsed requests + live hook state into everything the UI shows. Pure, so testable.

use crate::actions::ReplyView;
use crate::events::{Live, State};
use crate::logs::{Request, SessionMeta};
use chrono::{DateTime, Datelike, Duration, Local, Timelike};
use serde::Serialize;
use std::collections::HashMap;

pub const HISTORY_DAYS: i64 = 30;
const RECENT_HOURS: i64 = 8;
const MAX_SESSIONS: usize = 15;

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
}

struct Agg<'a> {
    first: DateTime<Local>,
    last: DateTime<Local>,
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

pub fn build<'a>(
    requests: impl Iterator<Item = &'a Request>,
    titles: &HashMap<&str, &str>,
    meta: &HashMap<&str, &SessionMeta>,
    live: &HashMap<String, Live>,
    now: DateTime<Local>,
) -> Dashboard {
    // The same request can appear in several files (resumed sessions): dedupe globally.
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

    let today = now.date_naive();
    let yesterday = today - Duration::days(1);
    let hour_ago = now - Duration::hours(1);
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
            .map(|i| DayPoint { date: (today - Duration::days(i)).to_string(), cost: 0.0, tokens: 0 })
            .collect(),
        projects: vec![],
        heatmap: vec![[0; 24]; 7],
        sessions: vec![],
        needs_you: 0,
        working: 0,
        replies: vec![],
    };
    let mut models = HashMap::new();
    let mut projects = HashMap::new();
    let mut today_by_session: HashMap<&str, f64> = HashMap::new();
    let mut sessions: HashMap<&str, Agg> = HashMap::new();

    for r in &all {
        let date = r.ts.date_naive();
        let age = (today - date).num_days();
        if !(0..HISTORY_DAYS).contains(&age) {
            continue;
        }
        let cost = r.cost.unwrap_or(0.0);
        let day = &mut d.daily[(HISTORY_DAYS - 1 - age) as usize];
        day.cost += cost;
        day.tokens += r.tokens;
        d.heatmap[r.ts.weekday().num_days_from_monday() as usize][r.ts.hour() as usize] += 1;

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
        } else if date == yesterday && r.ts.time() <= now.time() {
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
    let recent = now - Duration::hours(RECENT_HOURS);
    let mut ids: Vec<&str> = sessions.iter().filter(|(_, a)| a.last >= recent).map(|(id, _)| *id).collect();
    ids.extend(live.iter().filter(|(id, l)| l.state != State::Ended && !sessions.contains_key(id.as_str())).map(|(id, _)| id.as_str()));

    for id in ids {
        let agg = sessions.get(id);
        let hook = live.get(id);
        let last = agg.map(|a| a.last).or(hook.map(|h| h.since)).unwrap_or(now);
        let state = match hook {
            Some(h) if h.state == State::Ended => continue,
            // Log moved on after the permission prompt → it was answered.
            Some(h) if h.state == State::NeedsYou && last > h.since + Duration::seconds(10) => "working",
            Some(h) => match h.state {
                State::Working => "working",
                State::NeedsYou => "needs_you",
                _ => "done",
            },
            // No hooks: the log says how the last reply ended.
            None if meta.get(id).is_some_and(|m| m.last_stop.as_deref() == Some("end_turn")) => "done",
            None if now - last < Duration::minutes(2) => "working",
            None => "idle",
        };
        d.sessions.push(SessionView {
            id: id.to_string(),
            title: title_of(id),
            project: agg.map(|a| a.project.to_string()).or(hook.map(|h| h.project.clone())).unwrap_or_default(),
            state,
            message: hook.filter(|_| state == "needs_you").map(|h| h.message.clone()).unwrap_or_default(),
            started: agg.map(|a| a.first).unwrap_or(last).to_rfc3339(),
            last_active: last.to_rfc3339(),
            // When the current state began; only known when hooks report it.
            state_since: hook.map(|h| h.since.to_rfc3339()),
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

    fn req(key: &str, session: &str, ts: DateTime<Local>, cost: f64, output: u64) -> Request {
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

    fn noon() -> DateTime<Local> {
        use chrono::TimeZone;
        Local.with_ymd_and_hms(2026, 6, 15, 12, 0, 0).unwrap()
    }

    fn live(state: State, since: DateTime<Local>) -> Live {
        Live { state, since, project: "demo".into(), message: "Allow Bash?".into() }
    }

    #[test]
    fn totals_dedupe_across_files_and_history() {
        let now = noon();
        let reqs = [
            req("a", "s1", now - Duration::minutes(5), 2.0, 10),
            req("a", "s1", now - Duration::minutes(5), 2.0, 10), // same request in a resumed file
            req("b", "s1", now - Duration::days(3), 5.0, 10),
            req("c", "s2", now - Duration::days(40), 9.0, 10), // outside history
        ];
        let d = build(reqs.iter(), &HashMap::new(), &HashMap::new(), &HashMap::new(), now);
        assert_eq!(d.today_requests, 1);
        assert_eq!(d.today_cost, 2.0);
        assert_eq!(d.burn_rate_per_hour, 2.0);
        assert_eq!(d.cache_saved_today, 1.0);
        assert_eq!(d.daily.len(), HISTORY_DAYS as usize);
        assert_eq!(d.daily.last().unwrap().cost, 2.0);
        assert_eq!(d.projects[0].cost, 7.0);
        assert_eq!(d.heatmap.iter().flatten().sum::<u32>(), 2);
    }

    #[test]
    fn session_counts_under_the_folder_it_started_in() {
        let now = noon();
        let mut later = req("2", "s", now - Duration::minutes(1), 3.0, 1);
        later.project = "src".into(); // Claude cd'd into a subfolder
        let reqs = [later, req("1", "s", now - Duration::minutes(9), 1.0, 1)];
        let d = build(reqs.iter(), &HashMap::new(), &HashMap::new(), &HashMap::new(), now);
        assert_eq!(d.projects.len(), 1);
        assert_eq!((d.projects[0].name.as_str(), d.projects[0].cost), ("demo", 4.0));
        assert_eq!(d.sessions[0].project, "demo");
    }

    #[test]
    fn session_states_from_hooks_and_logs() {
        let now = noon();
        let reqs = [
            req("1", "waiting", now - Duration::minutes(3), 1.0, 1),
            req("2", "answered", now - Duration::seconds(5), 1.0, 1),
            req("3", "quiet", now - Duration::hours(1), 1.0, 1),
            req("4", "busy", now - Duration::seconds(30), 1.0, 1),
            req("5", "closed", now - Duration::minutes(1), 1.0, 1),
            req("6", "finished", now - Duration::hours(1), 1.0, 1),
        ];
        let finished = SessionMeta { last_stop: Some("end_turn".into()), entrypoint: "cli".into(), ..Default::default() };
        let meta = HashMap::from([("finished", &finished)]);
        let mut l = HashMap::new();
        l.insert("waiting".to_string(), live(State::NeedsYou, now - Duration::minutes(2)));
        l.insert("answered".to_string(), live(State::NeedsYou, now - Duration::minutes(2)));
        l.insert("closed".to_string(), live(State::Ended, now));
        l.insert("fresh".to_string(), live(State::Working, now)); // hook seen, no log yet
        let titles = HashMap::from([("waiting", "Fix login bug")]);
        let d = build(reqs.iter(), &titles, &meta, &l, now);

        let state = |id: &str| d.sessions.iter().find(|s| s.id == id).map(|s| s.state);
        assert_eq!(state("waiting"), Some("needs_you"));
        assert_eq!(state("answered"), Some("working"));
        assert_eq!(state("quiet"), Some("idle"));
        assert_eq!(state("busy"), Some("working")); // inferred without hooks
        assert_eq!(state("finished"), Some("done")); // last reply ended with end_turn
        assert_eq!(state("fresh"), Some("working"));
        assert_eq!(state("closed"), None);
        assert_eq!(d.sessions[0].title, "Fix login bug"); // needs_you sorts first
        assert_eq!(d.sessions[0].message, "Allow Bash?");
        assert_eq!((d.needs_you, d.working), (1, 3));
    }
}
