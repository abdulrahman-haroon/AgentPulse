import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { focusSession, markAllChecked, openDashboard, setSound, toggleWidget, useDashboard, type Dashboard, type Provider, type Session } from "./api";
import { money, since, tokens } from "./format";
import { BellIcon, BellOffIcon, ChevronIcon, CloseIcon, ExternalIcon } from "./components/Icons";
import { StatusDot } from "./components/StatusBadge";
import { AgentList, providerName } from "./components/Agents";
import { ApprovalCard } from "./components/ApprovalCard";
import { LimitBars } from "./components/Charts";
import { Logo } from "./components/Logo";

const WIDTH = 340;
const ROWS_PER_GROUP = 4;
const COMPACT_KEY = "agentpulse.widgetCompact";
const FOLDED_KEY = "agentpulse.widgetFolded";
const ORDER: Provider[] = ["claude", "copilot", "codex"];

function when(s: Session, now: number) {
  const t = s.state_since ?? s.last_active;
  if (s.state === "working") return s.state_since ? `running ${since(t, now)}` : "working";
  if (s.state === "needs_you") return `waiting ${since(t, now)}`;
  return `done ${since(t, now)} ago`;
}

/** localStorage can be unavailable; preferences then just don't persist. */
function load<T>(key: string, fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    return v === null ? fallback : (JSON.parse(v) as T);
  } catch {
    return fallback;
  }
}
function store(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* preference just won't persist */
  }
}

type Group = { provider: Provider; label: string; sessions: Session[]; today: string; counts: string; limits: Dashboard["limits"] };

/** Active sessions per agent. Agents that need you come first, then busy ones, then the rest. */
function groupByAgent(d: Dashboard): Group[] {
  const label = (p: Provider) => d.sources.find((x) => x.provider === p)?.label ?? providerName(p);
  const rank = (g: Group) => (g.sessions.some((s) => s.state === "needs_you") ? 0 : g.sessions.some((s) => s.state === "working") ? 1 : 2);
  return ORDER.map((p) => {
    const sessions = d.sessions.filter((s) => s.provider === p && s.state !== "idle");
    const t = d.providers_today.find((n) => n.name === label(p));
    const n = (st: Session["state"]) => sessions.filter((s) => s.state === st).length;
    const counts = [n("needs_you") && `${n("needs_you")} need you`, n("working") && `${n("working")} working`, n("done") && `${n("done")} done`]
      .filter(Boolean)
      .join(" · ");
    // Copilot bills per request, so its "today" is tokens, not dollars.
    const today = !t ? "" : p === "copilot" ? `${tokens(t.tokens)} tok` : money(t.cost);
    return { provider: p, label: label(p), sessions, today, counts, limits: d.limits.filter((l) => l.provider === p) };
  })
    .filter((g) => g.sessions.length > 0 || g.limits.length > 0)
    .sort((a, b) => rank(a) - rank(b));
}

/** Always-on-top companion: what needs you, what's running per agent (sub-agents too), today's spend. */
export default function Widget() {
  const [fast, setFast] = useState(false);
  const { data, reload } = useDashboard(fast ? 1500 : 3000);
  const [compact, setCompact] = useState(() => load(COMPACT_KEY, false));
  const [folded, setFolded] = useState<Provider[]>(() => load(FOLDED_KEY, []));
  const [failed, setFailed] = useState<{ id: string; error: string } | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const pending = data?.approvals.length ?? 0;
  useEffect(() => setFast(pending > 0), [pending]); // approvals count down: poll faster

  // The window has no frame to resize by, so it follows its content's height.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const win = getCurrentWindow();
    let last = 0;
    const ro = new ResizeObserver(() => {
      const h = Math.ceil(el.getBoundingClientRect().height) + 16; // room for the shadow
      if (h !== last) {
        last = h;
        win.setSize(new LogicalSize(WIDTH, h)).catch(() => {});
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const toggleCompact = () => {
    setCompact(!compact);
    store(COMPACT_KEY, !compact);
  };
  const toggleGroup = (p: Provider) => {
    const next = folded.includes(p) ? folded.filter((x) => x !== p) : [...folded, p];
    setFolded(next);
    store(FOLDED_KEY, next);
  };
  // A row click goes to where the session lives (its project in VS Code), not to the dashboard.
  const reveal = (id: string) => {
    setFailed(null);
    focusSession(id).then(reload).catch((e) => setFailed({ id, error: String(e) }));
  };
  const clearNew = () => markAllChecked().then(reload).catch(() => {});
  const flipSound = () => data && setSound(!data.sound_enabled).then(reload).catch(() => {});

  const now = Date.now();
  const needs = data ? Math.max(data.needs_you, pending) : 0;
  const unchecked = new Set(data?.unchecked ?? []);
  const quiet = !!data && needs + data.working + data.agents_running + unchecked.size === 0;
  const groups = data ? groupByAgent(data) : [];

  return (
    <div className="widget" ref={ref}>
      <header className="w-head" data-tauri-drag-region>
        <span className="w-logo" data-tauri-drag-region><Logo size={20} /></span>
        <span className="w-summary" data-tauri-drag-region aria-live="polite">
          {!data ? "Loading…" : quiet ? <span className="w-quiet" data-tauri-drag-region>All quiet</span> : (
            <>
              {needs > 0 && <span className="w-pill w-pill-needs">{needs} need{needs === 1 ? "s" : ""} you</span>}
              {unchecked.size > 0 && (
                <button className="w-pill w-pill-new" onClick={clearNew} title="Finished, not checked yet. Click to mark all as checked.">
                  {unchecked.size} new
                </button>
              )}
              {data.working > 0 && <span className="w-pill w-pill-working">{data.working} working</span>}
              {data.agents_running > 0 && <span className="w-pill w-pill-agents">{data.agents_running} agent{data.agents_running === 1 ? "" : "s"}</span>}
            </>
          )}
        </span>
        <button className="w-btn" aria-label={compact ? "Show sessions" : "Hide sessions"} title={compact ? "Show sessions" : "Hide sessions"} onClick={toggleCompact}>
          <ChevronIcon up={!compact} />
        </button>
        <button className="w-btn" aria-label="Open dashboard" title="Open dashboard" onClick={() => openDashboard()}>
          <ExternalIcon />
        </button>
        <button className="w-btn" aria-label="Hide widget" title="Hide widget" onClick={() => toggleWidget()}>
          <CloseIcon />
        </button>
      </header>

      {/* Approvals are urgent: shown even when collapsed. */}
      {data?.approvals.map((a) => <ApprovalCard key={a.request_id} a={a} compact />)}

      {!compact && data && (
        <div className="w-body">
          {groups.length === 0 ? (
            <p className="w-empty">No active sessions in the last 8 hours.</p>
          ) : (
            groups.map((g) => {
              const open = !folded.includes(g.provider);
              const more = g.sessions.length - ROWS_PER_GROUP;
              return (
                <section key={g.provider} className={`w-group w-group-${g.provider}`} aria-label={g.label}>
                  <button className={`w-ghead ${open ? "" : "w-ghead-folded"}`} aria-expanded={open} onClick={() => toggleGroup(g.provider)}>
                    <ChevronIcon size={12} />
                    <span className={`w-gmark w-gmark-${g.provider}`} aria-hidden="true" />
                    <span className="w-gname">{g.label}</span>
                    <span className="w-gcounts">{g.counts}</span>
                    {g.today && <span className="w-gtoday" title="Today">{g.today}</span>}
                  </button>
                  {open && (
                    <>
                      {g.limits.length > 0 && <LimitBars limits={g.limits} now={now} bare />}
                      <ul className="w-list">
                        {g.sessions.slice(0, ROWS_PER_GROUP).map((s) => (
                          <li key={s.id}>
                            <button className={`w-row w-row-${s.state}`} onClick={() => reveal(s.id)} title={`${s.title} · open ${s.project || "the project"} in VS Code`}>
                              <StatusDot state={s.state} />
                              <span className="w-main">
                                <span className="w-line">
                                  <span className="w-title">{s.title}</span>
                                  {unchecked.has(s.id) && <span className="w-new" title="Finished, not checked yet">new</span>}
                                  <span className="w-cost">{s.provider === "copilot" ? `${s.requests} req` : money(s.cost)}</span>
                                </span>
                                <span className="w-sub">{s.project || "unknown project"} · <span className={`w-when w-when-${s.state}`}>{when(s, now)}</span></span>
                                {s.state === "needs_you" && s.message && <span className="w-msg">{s.message}</span>}
                              </span>
                            </button>
                            {failed?.id === s.id && <p className="w-err" role="alert">{failed.error}</p>}
                            <AgentList agents={s.agents} total={s.agents_total} now={now} compact />
                          </li>
                        ))}
                      </ul>
                      {more > 0 && <button className="w-more" onClick={() => openDashboard()}>+{more} more in the dashboard</button>}
                    </>
                  )}
                </section>
              );
            })
          )}
        </div>
      )}

      <footer className="w-foot" data-tauri-drag-region>
        <span className="w-total" data-tauri-drag-region>{data ? money(data.today_cost) : "…"}</span>
        <span className="w-muted" data-tauri-drag-region>
          today{data && data.burn_rate_per_hour > 0 ? ` · ${money(data.burn_rate_per_hour)}/h` : ""}
        </span>
        {data && (
          <button
            className={`w-btn w-bell ${data.sound_enabled ? "" : "w-bell-off"}`}
            aria-label={data.sound_enabled ? "Mute alert sounds" : "Turn alert sounds on"}
            aria-pressed={!data.sound_enabled}
            title={data.sound_enabled ? "Alert sounds on (click to mute)" : "Alert sounds muted (click to turn on)"}
            onClick={flipSound}
          >
            {data.sound_enabled ? <BellIcon /> : <BellOffIcon />}
          </button>
        )}
      </footer>
    </div>
  );
}
