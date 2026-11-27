import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { openDashboard, toggleWidget, useDashboard, type Session } from "./api";
import { money, since } from "./format";
import { ChevronIcon, CloseIcon, ExternalIcon } from "./components/Icons";
import { StatusDot } from "./components/StatusBadge";
import { AgentList, ProviderTag } from "./components/Agents";
import { ApprovalCard } from "./components/ApprovalCard";
import { LimitBars } from "./components/Charts";
import { Logo } from "./components/Logo";

const WIDTH = 340;
const MAX_ROWS = 6;
const COMPACT_KEY = "agentpulse.widgetCompact";

function when(s: Session, now: number) {
  const t = s.state_since ?? s.last_active;
  if (s.state === "working") return s.state_since ? `running ${since(t, now)}` : "working";
  if (s.state === "needs_you") return `waiting ${since(t, now)}`;
  return `done ${since(t, now)} ago`;
}

function readCompact() {
  try {
    return localStorage.getItem(COMPACT_KEY) === "on";
  } catch {
    return false;
  }
}

/** Always-on-top companion: what needs you, what's running (sub-agents too), today's spend. */
export default function Widget() {
  const [fast, setFast] = useState(false);
  const { data } = useDashboard(fast ? 1500 : 3000);
  const [compact, setCompact] = useState(readCompact);
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
    try {
      localStorage.setItem(COMPACT_KEY, compact ? "off" : "on");
    } catch {
      /* preference just won't persist */
    }
  };

  const now = Date.now();
  const sessions = (data?.sessions ?? []).filter((s) => s.state !== "idle").slice(0, MAX_ROWS);
  const needs = data ? Math.max(data.needs_you, pending) : 0;
  const quiet = !!data && needs + data.working + data.agents_running === 0;

  return (
    <div className="widget" ref={ref}>
      <header className="w-head" data-tauri-drag-region>
        <span className="w-logo" data-tauri-drag-region><Logo size={20} /></span>
        <span className="w-summary" data-tauri-drag-region aria-live="polite">
          {!data ? "Loading…" : quiet ? <span className="w-quiet" data-tauri-drag-region>All quiet</span> : (
            <>
              {needs > 0 && <span className="w-pill w-pill-needs">{needs} need{needs === 1 ? "s" : ""} you</span>}
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
          {sessions.length === 0 ? (
            <p className="w-empty">No active sessions in the last 8 hours.</p>
          ) : (
            <ul className="w-list">
              {sessions.map((s) => (
                <li key={s.id}>
                  <button className={`w-row w-row-${s.state}`} onClick={() => openDashboard()} title={`${s.title} · open the dashboard`}>
                    <StatusDot state={s.state} />
                    <span className="w-main">
                      <span className="w-line">
                        <span className="w-title">{s.title}</span>
                        <ProviderTag provider={s.provider} />
                      </span>
                      <span className="w-sub">{s.project || "unknown project"} · {when(s, now)}</span>
                      {s.state === "needs_you" && s.message && <span className="w-msg">{s.message}</span>}
                    </span>
                  </button>
                  <AgentList agents={s.agents} total={s.agents_total} now={now} compact />
                </li>
              ))}
            </ul>
          )}
          {data.limits.length > 0 && <LimitBars limits={data.limits} now={now} />}
        </div>
      )}

      <footer className="w-foot" data-tauri-drag-region>
        <span className="w-cost" data-tauri-drag-region>{data ? money(data.today_cost) : "…"}</span>
        <span className="w-muted" data-tauri-drag-region>
          today{data && data.burn_rate_per_hour > 0 ? ` · ${money(data.burn_rate_per_hour)}/h` : ""}
        </span>
      </footer>
    </div>
  );
}
