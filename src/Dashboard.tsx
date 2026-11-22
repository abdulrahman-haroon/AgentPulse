import { useCallback, useEffect, useState } from "react";
import { hookStatus, toggleWidget, useDashboard, type HookStatus } from "./api";
import { since } from "./format";
import { ChartIcon, PinIcon, PlugIcon, PulseIcon, SparkIcon } from "./components/Icons";
import { LiveView } from "./views/LiveView";
import { InsightsView } from "./views/InsightsView";
import { HistoryView } from "./views/HistoryView";
import { SetupView } from "./views/SetupView";

const PAGES = [
  { id: "live", label: "Live", icon: <PulseIcon />, subtitle: "Every Claude Code session, right now" },
  { id: "insights", label: "Insights", icon: <SparkIcon />, subtitle: "What today is costing, and where it goes" },
  { id: "history", label: "History", icon: <ChartIcon />, subtitle: "Trends, projects and your working hours" },
  { id: "setup", label: "Setup", icon: <PlugIcon />, subtitle: "Hooks and privacy" },
] as const;
type PageId = (typeof PAGES)[number]["id"];

export default function Dashboard() {
  const [page, setPage] = useState<PageId>("live");
  const { data, error, updatedAt } = useDashboard(4000);
  const [hooks, setHooks] = useState<HookStatus | null>(null);
  const recheck = useCallback(() => { hookStatus().then(setHooks).catch(() => {}); }, []);
  useEffect(recheck, [recheck]);

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
            {p.id === "live" && data && data.needs_you > 0 && <span className="nav-count">{data.needs_you}</span>}
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
          <SetupView status={hooks} recheck={recheck} />
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
