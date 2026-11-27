//! Turns parsed requests + live hook state into everything the UI shows. Pure, so testable.

use crate::actions::ReplyView;
use crate::events::{AlertKind, Live, State};
use crate::logs::{project_name, PlanLimit, Provider, Request, SessionMeta, SubAgent};
use jiff::{SignedDuration, Timestamp, ToSpan, ZonedDateTime};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

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
    pub provider: Provider,
    /// Running sub-agents first, then the most recent finished ones.
    pub agents: Vec<AgentView>,
    pub agents_running: usize,
    /// Sub-agents active in the live window, including ones not listed.
    pub agents_total: usize,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct AgentView {
    pub id: String,
    pub kind: String,
    pub description: String,
    pub background: bool,
    pub running: bool,
    pub started: Option<String>,
    pub last_active: Option<String>,
    pub cost: f64,
    pub model: String,
}

/// Where AgentPulse looks for one agent's logs, and whether anything is there (filled by lib.rs).
#[derive(Serialize, Debug, Clone)]
pub struct SourceView {
    pub provider: Provider,
    pub label: &'static str,
    pub path: String,
    pub found: bool,
}

/// Running until it says it finished, unless its transcript has been silent longer than a long
/// tool call could explain (crashed or killed).
pub fn agent_running(a: &SubAgent, now: Timestamp) -> bool {
    !a.finished && a.last.is_some_and(|l| now.duration_since(l) < UNFINISHED_TURN_GRACE)
}

fn agent_view(a: &SubAgent, now: Timestamp) -> AgentView {
    AgentView {
        id: a.id.clone(),
        kind: a.kind.clone(),
        description: a.description.clone(),
        background: a.background,
        running: agent_running(a, now),
        started: a.started.map(|t| t.to_string()),
        last_active: a.last.map(|t| t.to_string()),
        cost: a.cost,
        model: a.model.clone(),
    }
}

/// Live state of a session from an agent without hooks (Codex, Copilot), from its logs alone.
#[derive(Debug, Clone, PartialEq)]
pub struct LogState {
    pub state: &'static str,
    pub provider: Provider,
    pub project: String,
}

/// The session's own model from its main thread. The newest request may be a sub-agent's (often a
/// smaller model). Placeholders like "<synthetic>" (error stand-ins) don't count.
fn main_model(m: Option<&SessionMeta>) -> Option<String> {
    m.and_then(|m| m.model.clone()).filter(|x| !x.starts_with('<'))
}

/// done when the last turn ended; working while the log moves (or a long tool call could still
/// be running); idle after that.
/// "needs_input" is a pending permission prompt in an agent's own log (Copilot).
fn inferred_state(m: Option<&SessionMeta>, last: Timestamp, now: Timestamp) -> &'static str {
    match m.and_then(|m| m.last_stop.as_deref()) {
        Some("end_turn") => "done",
        Some("needs_input") => "needs_you",
        _ if now.duration_since(last) < UNFINISHED_TURN_GRACE => "working",
        _ => "idle",
    }
}

pub fn log_states(meta: &HashMap<&str, &SessionMeta>, now: Timestamp) -> HashMap<String, LogState> {
    let recent = now - SignedDuration::from_hours(RECENT_HOURS);
    meta.iter()
        .filter(|(_, m)| m.provider != Provider::Claude && !m.ended)
        .filter_map(|(id, m)| {
            let last = m.last_ts.filter(|t| *t >= recent)?;
            Some((id.to_string(), LogState { state: inferred_state(Some(m), last, now), provider: m.provider, project: project_name(&m.cwd) }))
        })
        .collect()
}

/// What changed since the last check that deserves an alert: working → done ("finished") and
/// anything → needs_you. Sessions seen for the first time don't alert.
/// shortcut: a whole turn shorter than one check (2 s) is missed; a per-turn counter would catch it.
pub fn log_alerts<'a>(before: &HashMap<String, LogState>, now: &'a HashMap<String, LogState>) -> Vec<(AlertKind, &'a str, &'a LogState)> {
    now.iter()
        .filter_map(|(id, s)| match (before.get(id)?.state, s.state) {
            ("working", "done") => Some((AlertKind::Done, id.as_str(), s)),
            (b, "needs_you") if b != "needs_you" => Some((AlertKind::NeedsYou, id.as_str(), s)),
            _ => None,
        })
        .collect()
}

/// A finished session stays "not checked yet" until the user picks it back up (it works or asks
/// again) or closes it. Opening it from AgentPulse or opening the dashboard clears it too (lib.rs).
pub fn still_unchecked(hook: Option<State>, log: Option<&str>) -> bool {
    !matches!(hook, Some(State::Working | State::NeedsYou | State::Ended)) && !matches!(log, Some("working" | "needs_you"))
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
    /// Sub-agents running across the sessions on the board.
    pub agents_running: usize,
    /// Plan usage the agents report about themselves (Codex), newest first.
    pub limits: Vec<PlanLimit>,
    /// Today's cost per agent (Claude Code, Codex, Copilot).
    pub providers_today: Vec<NameValue>,
    /// Filled in by lib.rs.
    pub sources: Vec<SourceView>,
    /// Alert chimes on (from settings; the widget's bell shows and flips it). Filled in by lib.rs.
    pub sound_enabled: bool,
    /// Sessions that finished since the user last looked ("not checked yet"). Filled in by lib.rs.
    pub unchecked: Vec<String>,
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
    provider: Provider,
}

impl<'a> Agg<'a> {
    fn new(r: &'a Request) -> Self {
        Agg { first: r.ts, last: r.ts, cost: 0.0, tokens: 0, requests: 0, project: &r.project, model: &r.model, provider: r.provider }
    }
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
    pub provider: Provider,
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
        by_session.entry(&r.session).or_insert_with(|| Agg::new(r)).add(r);
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
                model: main_model(m.copied()).unwrap_or_else(|| a.model.to_string()),
                entrypoint: m.map(|m| m.entrypoint.clone()).unwrap_or_default(),
                last_prompt: m.map(|m| m.last_prompt.clone()).unwrap_or_default(),
                provider: a.provider,
            }
        })
        .collect();
    rows.sort_by(|a, b| b.last_active.cmp(&a.last_active));
    rows
}

impl<'a> Agg<'a> {
    fn add(&mut self, r: &'a Request) {
        // A session belongs to the folder it started in, not subfolders it later cd'd into.
        if r.ts < self.first {
            self.first = r.ts;
            self.project = &r.project;
        }
        if r.ts >= self.last {
            self.last = r.ts;
            self.model = &r.model;
        }
        self.cost += r.cost.unwrap_or(0.0);
        self.tokens += r.tokens;
        self.requests += 1;
    }
}

pub fn build<'a>(
    requests: impl Iterator<Item = &'a Request>,
    titles: &HashMap<&str, &str>,
    meta: &HashMap<&str, &SessionMeta>,
    agents: &[&SubAgent],
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
        agents_running: 0,
        limits: vec![],
        providers_today: vec![],
        sources: vec![],
        sound_enabled: true,
        unchecked: vec![],
    };
    let mut models = HashMap::new();
    let mut projects = HashMap::new();
    let mut providers = HashMap::new();
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
        sessions.entry(&r.session).or_insert_with(|| Agg::new(r)).add(r);

        if date == today {
            d.today_cost += cost;
            d.today_tokens += r.tokens;
            d.today_requests += 1;
            d.cache_saved_today += r.cache_saved;
            *today_by_session.entry(&r.session).or_default() += cost;
            add(&mut models, &r.model, r);
            add(&mut providers, r.provider.label(), r);
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
    d.providers_today = ranked(providers, 3);
    for s in sessions.values() {
        let p = projects.entry(s.project.to_string()).or_insert_with(|| NameValue { name: s.project.to_string(), ..Default::default() });
        p.cost += s.cost;
        p.tokens += s.tokens;
    }
    d.projects = ranked(projects, 8);

    // Live board: recently active sessions (by usage or by log events, e.g. a Codex turn that
    // hasn't reported usage yet) plus any the hooks say are open.
    let recent = now_ts - SignedDuration::from_hours(RECENT_HOURS);
    let mut ids: HashSet<&str> = sessions.iter().filter(|(_, a)| a.last >= recent).map(|(id, _)| *id).collect();
    ids.extend(meta.iter().filter(|(_, m)| m.last_ts.is_some_and(|t| t >= recent)).map(|(id, _)| *id));
    ids.extend(live.iter().filter(|(_, l)| l.state != State::Ended).map(|(id, _)| id.as_str()));
    let mut agents_of: HashMap<&str, Vec<&SubAgent>> = HashMap::new();
    for a in agents.iter().filter(|a| a.last.is_some_and(|l| l >= recent)) {
        agents_of.entry(a.session.as_str()).or_default().push(a);
    }

    for id in ids {
        let agg = sessions.get(id);
        let hook = live.get(id);
        let m = meta.get(id).copied();
        if m.is_some_and(|m| m.ended) {
            continue;
        }
        // Hooks saw it open but it never wrote a log (opened, nothing asked): noise, and nothing
        // to open or reply to. Shown only while the hooks say it's busy (its first reply is coming).
        if agg.is_none() && m.is_none() && !hook.is_some_and(|h| matches!(h.state, State::Working | State::NeedsYou)) {
            continue;
        }
        // Main-thread activity only (sub-agents keep writing while the session waits for you).
        let main_last = m.and_then(|m| m.last_ts).or(agg.map(|a| a.last));
        let last = agg.map(|a| a.last).max(main_last).or(hook.map(|h| h.since)).unwrap_or(now_ts);
        let state = match hook {
            Some(h) if h.state == State::Ended => continue,
            // Log moved on after the permission prompt → it was answered.
            Some(h) if h.state == State::NeedsYou && main_last.is_some_and(|t| t > h.since + SignedDuration::from_secs(10)) => "working",
            Some(h) => match h.state {
                State::Working => "working",
                State::NeedsYou => "needs_you",
                _ => "done",
            },
            // No hooks (or an agent without them): the log says how the last turn ended. A turn
            // that didn't finish may be a long tool call (a build, tests) that writes nothing while
            // it runs, so it stays Working for a while; that also keeps Reply/Resume off live sessions.
            None => inferred_state(m, last, now_ts),
        };
        let mut agents: Vec<AgentView> = agents_of.get(id).into_iter().flatten().map(|a| agent_view(a, now_ts)).collect();
        agents.sort_by(|a, b| b.running.cmp(&a.running).then(b.last_active.cmp(&a.last_active)));
        let (agents_total, agents_running) = (agents.len(), agents.iter().filter(|a| a.running).count());
        agents.truncate(agents_running + 5);
        d.sessions.push(SessionView {
            id: id.to_string(),
            title: title_of(id),
            project: agg
                .map(|a| a.project.to_string())
                .or(m.filter(|m| !m.cwd.is_empty()).map(|m| project_name(&m.cwd)))
                .or(hook.map(|h| h.project.clone()))
                .unwrap_or_default(),
            state,
            message: hook.filter(|_| state == "needs_you").map(|h| h.message.clone()).unwrap_or_default(),
            started: agg.map(|a| a.first).unwrap_or(last).to_string(),
            last_active: last.to_string(),
            // When the current state began; only known when hooks report it.
            state_since: hook.map(|h| h.since.to_string()),
            cost: agg.map_or(0.0, |a| a.cost),
            requests: agg.map_or(0, |a| a.requests),
            model: main_model(m).or(agg.map(|a| a.model.to_string())).unwrap_or_default(),
            entrypoint: m.map(|m| m.entrypoint.clone()).unwrap_or_default(),
            provider: agg.map(|a| a.provider).or(m.map(|m| m.provider)).unwrap_or_default(),
            agents,
            agents_running,
            agents_total,
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
    d.agents_running = d.sessions.iter().map(|s| s.agents_running).sum();
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(key: &str, session: &str, ts: Timestamp, cost: f64, output: u64) -> Request {
        Request {
            provider: Provider::Claude,
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
        let d = build(reqs.iter(), &HashMap::new(), &HashMap::new(), &[], &HashMap::new(), &now);
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
        cache.refresh(&crate::logs::roots(&home), since);
        let rows = history(cache.requests(), &cache.titles(), &cache.meta(), &ZonedDateTime::now());
        let with_prompt = rows.iter().filter(|r| !r.last_prompt.is_empty()).count();
        let titled = rows.iter().filter(|r| r.title != "Untitled session").count();
        println!("sessions={} titled={titled} with_last_prompt={with_prompt} total_cost={:.2} took={:?}", rows.len(), rows.iter().map(|r| r.cost).sum::<f64>(), t.elapsed());
        for p in [Provider::Claude, Provider::Codex, Provider::Copilot] {
            let mine: Vec<_> = rows.iter().filter(|r| r.provider == p).collect();
            let tokens: u64 = mine.iter().map(|r| r.tokens).sum();
            let models: std::collections::HashSet<_> = mine.iter().map(|r| r.model.as_str()).collect();
            println!("  {:?}: sessions={} requests={} tokens={tokens} models={}", p, mine.len(), mine.iter().map(|r| r.requests).sum::<u64>(), models.len());
        }
        let now = Timestamp::now();
        let states = log_states(&cache.meta(), now);
        let agents = cache.agents();
        println!("  live log states={:?} agents={} running={}", states.values().map(|s| s.state).collect::<Vec<_>>(), agents.len(), agents.iter().filter(|a| agent_running(a, now)).count());
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
        let d = build(reqs.iter(), &HashMap::new(), &HashMap::new(), &[], &HashMap::new(), &now);
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
        let d = build(reqs.iter(), &titles, &meta, &[], &l, &now);

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

    #[test]
    fn waiting_session_stays_needs_you_while_a_subagent_writes() {
        let now = noon();
        let mut bg = req("2", "s", ago(&now, 5), 1.0, 1); // a background agent's request, just now
        bg.model = "claude-haiku-5-5".into();
        let reqs = [req("1", "s", ago(&now, 3 * 60), 1.0, 1), bg];
        let m = SessionMeta { last_ts: Some(ago(&now, 3 * 60)), model: Some("claude-opus-5-5".into()), ..Default::default() }; // main thread: quiet
        let meta = HashMap::from([("s", &m)]);
        let l = HashMap::from([("s".to_string(), live(State::NeedsYou, ago(&now, 2 * 60)))]);
        let d = build(reqs.iter(), &HashMap::new(), &meta, &[], &l, &now);
        assert_eq!(d.sessions[0].state, "needs_you");
        assert_eq!(d.sessions[0].model, "claude-opus-5-5"); // not the sub-agent's haiku
        assert_eq!(history(reqs.iter(), &HashMap::new(), &meta, &now)[0].model, "claude-opus-5-5");
    }

    fn agent(id: &str, finished: bool, last_secs_ago: i64, now: &ZonedDateTime) -> SubAgent {
        SubAgent { id: id.into(), session: "s".into(), kind: "general-purpose".into(), background: true, finished, last: Some(ago(now, last_secs_ago)), ..Default::default() }
    }

    #[test]
    fn lists_subagents_running_first() {
        let now = noon();
        let reqs = [req("1", "s", ago(&now, 30), 1.0, 1)];
        let agents = [
            agent("done", true, 60, &now),
            agent("busy", false, 20, &now),
            agent("crashed", false, 3600, &now), // unfinished but silent for an hour
            agent("old", true, 9 * 3600, &now),  // outside the live window
        ];
        let refs: Vec<&SubAgent> = agents.iter().collect();
        let d = build(reqs.iter(), &HashMap::new(), &HashMap::new(), &refs, &HashMap::new(), &now);
        let s = &d.sessions[0];
        assert_eq!(s.agents.iter().map(|a| (a.id.as_str(), a.running)).collect::<Vec<_>>(), [("busy", true), ("done", false), ("crashed", false)]);
        assert_eq!((s.agents_running, s.agents_total, d.agents_running), (1, 3, 1));
    }

    #[test]
    fn codex_sessions_show_from_log_events_and_alert_when_done() {
        let now = noon();
        let mut m = SessionMeta { provider: Provider::Codex, cwd: r"C:\code\web-shop".into(), last_ts: Some(ago(&now, 20)), ..Default::default() };
        let mut r = req("1", "cx", ago(&now, 60), 0.5, 1);
        r.provider = Provider::Codex;
        r.key = None;
        let titles = HashMap::from([("cx", "Add dark mode")]);
        {
            // mid-turn, no usage reported yet for this turn: still on the board, Working
            let meta = HashMap::from([("cx", &m)]);
            let d = build([r.clone()].iter(), &titles, &meta, &[], &HashMap::new(), &now);
            let s = &d.sessions[0];
            assert_eq!((s.state, s.provider, s.project.as_str(), s.title.as_str()), ("working", Provider::Codex, "demo", "Add dark mode"));
            assert_eq!(d.providers_today[0].name, "Codex");
        }
        let before = log_states(&HashMap::from([("cx", &m)]), now.timestamp());
        assert_eq!(before["cx"].state, "working");
        m.last_stop = Some("end_turn".into());
        let after = log_states(&HashMap::from([("cx", &m)]), now.timestamp());
        let done = log_alerts(&before, &after);
        assert_eq!(done.len(), 1);
        assert_eq!((done[0].0, done[0].1, done[0].2.provider, done[0].2.project.as_str()), (AlertKind::Done, "cx", Provider::Codex, "web-shop"));
        assert!(log_alerts(&after, &after).is_empty()); // only once
        // a pending permission in the log (Copilot) is "needs you", and alerts once
        m.last_stop = Some("needs_input".into());
        let asking = log_states(&HashMap::from([("cx", &m)]), now.timestamp());
        assert_eq!(asking["cx"].state, "needs_you");
        assert_eq!(log_alerts(&before, &asking)[0].0, AlertKind::NeedsYou);
        assert!(log_alerts(&asking, &asking).is_empty());
        assert!(log_alerts(&HashMap::new(), &asking).is_empty()); // first sighting: no alert
        // Claude sessions are left to the hooks
        let claude = SessionMeta { last_ts: Some(now.timestamp()), ..Default::default() };
        assert!(log_states(&HashMap::from([("c", &claude)]), now.timestamp()).is_empty());
    }

    #[test]
    fn finished_sessions_stay_unchecked_until_picked_up_or_closed() {
        assert!(still_unchecked(Some(State::Done), None));
        assert!(still_unchecked(None, Some("done")));
        assert!(still_unchecked(None, None)); // log rotated away: keep until the dashboard is opened
        assert!(!still_unchecked(Some(State::Working), None)); // the user typed again
        assert!(!still_unchecked(Some(State::NeedsYou), None));
        assert!(!still_unchecked(Some(State::Ended), None)); // closed
        assert!(!still_unchecked(None, Some("working")));
    }

    #[test]
    fn hook_only_sessions_show_only_while_busy() {
        let now = noon();
        let l = HashMap::from([
            ("opened-and-left".to_string(), live(State::Done, ago(&now, 600))),
            ("first-prompt".to_string(), live(State::Working, ago(&now, 5))),
        ]);
        let d = build([].iter(), &HashMap::new(), &HashMap::new(), &[], &l, &now);
        assert_eq!(d.sessions.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["first-prompt"]);
    }
}
