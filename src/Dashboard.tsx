import { useCallback, useEffect, useState } from "react";
import { autoCheckEnabled, checkUpdate, hookStatus, toggleWidget, useDashboard, type HookStatus, type UpdateInfo } from "./api";
import { money, since } from "./format";
import { ChartIcon, ListIcon, PinIcon, PulseIcon, SlidersIcon, SparkIcon } from "./components/Icons";
import { Logo } from "./components/Logo";
import { SessionsView } from "./views/SessionsView";
import { LiveView } from "./views/LiveView";
import { InsightsView } from "./views/InsightsView";
import { HistoryView } from "./views/HistoryView";
import { SetupView } from "./views/SetupView";

const PAGES = [
  { id: "live", label: "Live", icon: <PulseIcon />, subtitle: "Every agent session and sub-agent, right now" },
  { id: "sessions", label: "Sessions", icon: <ListIcon />, subtitle: "Find any conversation from the last 30 days" },
  { id: "insights", label: "Insights", icon: <SparkIcon />, subtitle: "What today is costing, and where it goes" },
  { id: "history", label: "History", icon: <ChartIcon />, subtitle: "Trends, projects and your working hours" },
  { id: "setup", label: "Settings", icon: <SlidersIcon />, subtitle: "Agents, hooks, alerts, startup and updates" },
] as const;
type PageId = (typeof PAGES)[number]["id"];

export default function Dashboard() {
  const [page, setPage] = useState<PageId>("live");
  const [fast, setFast] = useState(false);
  const { data, error, updatedAt } = useDashboard(fast ? 1500 : 4000);
  // Poll faster while an approval counts down or a quick reply is streaming in.
  useEffect(() => setFast(!!data && (data.approvals.length > 0 || data.replies.some((r) => r.status === "running"))), [data]);
  const [hooks, setHooks] = useState<HookStatus | null>(null);
  const recheck = useCallback(() => { hookStatus().then(setHooks).catch(() => {}); }, []);
  useEffect(recheck, [recheck]);
  // A new approval request jumps to the Live page, where the card is.
  const approvalCount = data?.approvals.length ?? 0;
  useEffect(() => { if (approvalCount > 0) setPage("live"); }, [approvalCount]);
  // One quiet update check per launch (opt-out in Settings). Failures stay silent here.
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  useEffect(() => { if (autoCheckEnabled()) checkUpdate().then(setUpdate).catch(() => {}); }, []);

  const current = PAGES.find((p) => p.id === page)!;
  const needs = data ? Math.max(data.needs_you, data.approvals.length) : 0;
  const status = !data ? "" : [
    needs && `${needs} need${needs === 1 ? "s" : ""} you`,
    data.working && `${data.working} working`,
    data.agents_running && `${data.agents_running} sub‑agent${data.agents_running === 1 ? "" : "s"}`, // non-breaking hyphen
  ].filter(Boolean).join(" · ") || "All quiet";

  return (
    <div className="app">
      <nav className="sidebar" aria-label="Sections">
        <div className="brand">
          <Logo size={30} />
          <span className="brand-text">
            <span className="brand-name">AgentPulse</span>
            <span className="brand-sub">for AI coding agents</span>
          </span>
        </div>
        {PAGES.map((p) => (
          <button
            key={p.id}
            className={`nav ${page === p.id ? "nav-on" : ""}`}
            aria-current={page === p.id ? "page" : undefined}
            onClick={() => setPage(p.id)}
          >
            {p.icon}
            <span className="nav-label">{p.label}</span>
            {p.id === "live" && needs > 0 && <span className="nav-count">{needs}</span>}
            {p.id === "setup" && update?.available && <span className="nav-count nav-update" title={`Version ${update.available} available`}>1</span>}
          </button>
        ))}
        <div className="sidebar-foot">
          {data && (
            <div className="side-card">
              <span className="side-label">Today</span>
              <span className="side-value">{money(data.today_cost)}</span>
              <span className={`side-status ${needs ? "side-needs" : data.working + data.agents_running ? "side-busy" : ""}`}>{status}</span>
            </div>
          )}
          <button className="nav" onClick={() => toggleWidget()}>
            <PinIcon />
            <span className="nav-label">Floating widget</span>
          </button>
        </div>
      </nav>

      <main className="content">
        <header className="page-head">
          <div>
            <h1>{current.label}</h1>
            <p className="muted">{current.subtitle}</p>
          </div>
          {updatedAt && (
            <span className="live-pill" title="Refreshes every few seconds">
              <span className="live-dot" /> Live · {since(updatedAt.toISOString())} ago
            </span>
          )}
        </header>

        {error && <p className="callout callout-error" role="alert">Couldn't read usage: {error}</p>}
        {page === "setup" ? (
          <SetupView status={hooks} setStatus={setHooks} recheck={recheck} update={update} sources={data?.sources ?? []} />
        ) : page === "sessions" ? (
          <SessionsView />
        ) : !data ? (
          <div className="loading">
            <span className="loading-bar" />
            <p className="muted">Reading your agent history… the first load can take a few seconds.</p>
          </div>
        ) : page === "live" ? (
          <LiveView data={data} hooksMissing={!!hooks && hooks.missing.some((m) => m !== "StatusLine")} />
        ) : page === "insights" ? (
          <InsightsView data={data} />
        ) : (
          <HistoryView data={data} />
        )}
      </main>
    </div>
  );
}
