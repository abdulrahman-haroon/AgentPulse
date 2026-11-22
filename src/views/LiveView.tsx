import type { Dashboard, Session } from "../api";
import { money, since } from "../format";
import { StatusBadge } from "../components/StatusBadge";

function timeLabel(s: Session, now: number) {
  if (s.state_since && s.state === "working") return `running ${since(s.state_since, now)}`;
  if (s.state_since && s.state === "needs_you") return `waiting ${since(s.state_since, now)}`;
  return `active ${since(s.last_active, now)} ago`;
}

function SessionCard({ s, now }: { s: Session; now: number }) {
  return (
    <article className={`session session-${s.state}`}>
      <header className="session-head">
        <StatusBadge state={s.state} />
        <span className="session-time">{timeLabel(s, now)}</span>
      </header>
      <h3 className="session-title" title={s.title}>{s.title}</h3>
      <p className="session-meta">
        <span>{s.project || "unknown project"}</span>
        {s.model && <span>{s.model.replace(/^claude-/, "")}</span>}
      </p>
      {s.state === "needs_you" && s.message && <p className="session-message">{s.message}</p>}
      <footer className="session-foot">
        <span className="session-cost">{money(s.cost)}</span>
        <span className="muted">{s.requests} requests</span>
      </footer>
    </article>
  );
}

export function LiveView({ data, hooksMissing }: { data: Dashboard; hooksMissing: boolean }) {
  const now = Date.now();
  const delta = data.today_cost - data.yesterday_cost_by_now;
  return (
    <>
      <section className="strip" aria-label="Right now">
        <div className={`strip-item ${data.needs_you ? "strip-alert" : ""}`}>
          <span className="strip-value">{data.needs_you}</span>
          <span className="strip-label">need you</span>
        </div>
        <div className="strip-item">
          <span className="strip-value">{data.working}</span>
          <span className="strip-label">working</span>
        </div>
        <div className="strip-item">
          <span className="strip-value">{money(data.today_cost)}</span>
          <span className="strip-label">
            today ·{" "}
            <span className={delta > 0 ? "up" : "down"}>
              {delta >= 0 ? "+" : "−"}{money(Math.abs(delta))}
            </span>{" "}
            vs yesterday by now
          </span>
        </div>
        <div className="strip-item">
          <span className="strip-value">{money(data.burn_rate_per_hour)}<small>/h</small></span>
          <span className="strip-label">burn rate, last hour</span>
        </div>
      </section>

      {hooksMissing && (
        <p className="callout">
          Live states are guessed from log activity. Connect the hooks in <strong>Setup</strong> for exact
          "working / needs you / done" states.
        </p>
      )}

      {data.sessions.length === 0 ? (
        <div className="empty">
          <h3>No sessions in the last 8 hours</h3>
          <p className="muted">Start Claude Code in any project and it will show up here.</p>
        </div>
      ) : (
        <div className="sessions">
          {data.sessions.map((s) => <SessionCard key={s.id} s={s} now={now} />)}
        </div>
      )}
    </>
  );
}
