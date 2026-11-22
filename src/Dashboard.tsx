import { useCallback, useEffect, useState } from "react";
import { autoCheckEnabled, checkUpdate, hookStatus, toggleWidget, useDashboard, type HookStatus, type UpdateInfo } from "./api";
import { since } from "./format";
import { ChartIcon, ListIcon, PinIcon, PlugIcon, PulseIcon, SparkIcon } from "./components/Icons";
import { SessionsView } from "./views/SessionsView";
import { LiveView } from "./views/LiveView";
import { InsightsView } from "./views/InsightsView";
import { HistoryView } from "./views/HistoryView";
import { SetupView } from "./views/SetupView";

const PAGES = [
  { id: "live", label: "Live", icon: <PulseIcon />, subtitle: "Every Claude Code session, right now" },
  { id: "sessions", label: "Sessions", icon: <ListIcon />, subtitle: "Find any conversation from the last 30 days" },
  { id: "insights", label: "Insights", icon: <SparkIcon />, subtitle: "What today is costing, and where it goes" },
  { id: "history", label: "History", icon: <ChartIcon />, subtitle: "Trends, projects and your working hours" },
  { id: "setup", label: "Setup", icon: <PlugIcon />, subtitle: "Hooks, startup, updates and privacy" },
] as const;
type PageId = (typeof PAGES)[number]["id"];

export default function Dashboard() {
  const [page, setPage] = useState<PageId>("live");
  const [fast, setFast] = useState(false);
  const { data, error, updatedAt } = useDashboard(fast ? 1500 : 4000);
  // Poll faster while a quick reply is streaming in.
  useEffect(() => setFast(!!data && (data.approvals.length > 0 || data.replies.some((r) => r.status === "running"))), [data]);
  const [hooks, setHooks] = useState<HookStatus | null>(null);
  const recheck = useCallback(() => { hookStatus().then(setHooks).catch(() => {}); }, []);
  useEffect(recheck, [recheck]);
  // A new approval request jumps to the Live page, where the card is.
  const approvalCount = data?.approvals.length ?? 0;
  useEffect(() => { if (approvalCount > 0) setPage("live"); }, [approvalCount]);
  // One quiet update check per launch (opt-out in Setup). Failures stay silent here.
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  useEffect(() => { if (autoCheckEnabled()) checkUpdate().then(setUpdate).catch(() => {}); }, []);

  const current = PAGES.find((p) => p.id === page)!;
  return (
    <div className="app">
      <nav className="sidebar" aria-label="Sections">
        <div className="brand">
          <span className="brand-mark"><PulseIcon size={16} /></span>
          <span className="brand-name">AgentPulse</span>
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
            {p.id === "live" && data && data.needs_you + data.approvals.length > 0 && (
              <span className="nav-count">{Math.max(data.needs_you, data.approvals.length)}</span>
            )}
            {p.id === "setup" && update?.available && <span className="nav-count nav-update" title={`Version ${update.available} available`}>1</span>}
          </button>
        ))}
        <div className="sidebar-foot">
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
          {updatedAt && <span className="updated">Updated {since(updatedAt.toISOString())} ago</span>}
        </header>

        {error && <p className="callout callout-error" role="alert">Couldn't read usage: {error}</p>}
        {page === "setup" ? (
          <SetupView status={hooks} setStatus={setHooks} recheck={recheck} update={update} />
        ) : page === "sessions" ? (
          <SessionsView />
        ) : !data ? (
          <div className="loading">
            <span className="loading-bar" />
            <p className="muted">Reading your Claude Code history… the first load can take a few seconds.</p>
          </div>
        ) : page === "live" ? (
          <LiveView data={data} hooksMissing={!!hooks && hooks.missing.length > 0} />
        ) : page === "insights" ? (
          <InsightsView data={data} />
        ) : (
          <HistoryView data={data} />
        )}
      </main>
    </div>
  );
}
