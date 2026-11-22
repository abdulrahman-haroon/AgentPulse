import type { Dashboard } from "../api";
import { money } from "../format";
import { SessionCard } from "../components/SessionCard";

export function LiveView({ data, hooksMissing }: { data: Dashboard; hooksMissing: boolean }) {
  const now = Date.now();
  const delta = data.today_cost - data.yesterday_cost_by_now;
  const replies = new Map(data.replies.map((r) => [r.session_id, r]));
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
          Without hooks, states come from the logs: <strong>Done</strong> when Claude's last reply finished,
          otherwise <strong>Working</strong> or <strong>Idle</strong> by recent activity. Connect the hooks in
          <strong> Setup</strong> for instant, exact states and "needs you" alerts.
        </p>
      )}

      {data.sessions.length === 0 ? (
        <div className="empty">
          <h3>No sessions in the last 8 hours</h3>
          <p className="muted">Start Claude Code in any project and it will show up here.</p>
        </div>
      ) : (
        <div className="sessions">
          {data.sessions.map((s) => <SessionCard key={s.id} s={s} reply={replies.get(s.id)} now={now} />)}
        </div>
      )}
    </>
  );
}
