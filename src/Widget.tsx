import { toggleWidget, useDashboard } from "./api";
import { money } from "./format";
import { CloseIcon } from "./components/Icons";
import { StatusDot } from "./components/StatusBadge";

/** Always-on-top floating pill: live states and today's cost at a glance. */
export default function Widget() {
  const { data } = useDashboard(4000);
  const sessions = (data?.sessions ?? []).filter((s) => s.state !== "idle").slice(0, 3);

  return (
    <div className="widget">
      <header className="widget-head" data-tauri-drag-region>
        <span className="widget-cost" data-tauri-drag-region>{data ? money(data.today_cost) : "…"}</span>
        <span className="widget-sub" data-tauri-drag-region>
          today{data && data.burn_rate_per_hour > 0 ? ` · ${money(data.burn_rate_per_hour)}/h` : ""}
        </span>
        <button className="widget-close" aria-label="Hide widget" onClick={() => toggleWidget()}>
          <CloseIcon />
        </button>
      </header>
      {sessions.length === 0 ? (
        <p className="widget-empty" data-tauri-drag-region>No active sessions</p>
      ) : (
        <ul className="widget-list">
          {sessions.map((s) => (
            <li key={s.id}>
              <StatusDot state={s.state} />
              <span className="widget-title">{s.title}</span>
              <span className="widget-project">{s.project}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
