import type { Dashboard } from "../api";
import { money } from "../format";
import { SessionCard } from "../components/SessionCard";
import { ApprovalCard } from "../components/ApprovalCard";
import { LimitBars } from "../components/Charts";
import { AgentsIcon, AlertIcon, PulseIcon, SpinIcon, SparkIcon } from "../components/Icons";

function Kpi({ label, value, note, tone, icon }: { label: string; value: React.ReactNode; note?: React.ReactNode; tone?: string; icon: React.ReactNode }) {
  return (
    <div className={`kpi ${tone ? `kpi-${tone}` : ""}`}>
      <span className="kpi-label">{icon}{label}</span>
      <span className="kpi-value">{value}</span>
      {note && <span className="kpi-note">{note}</span>}
    </div>
  );
}

export function LiveView({ data, hooksMissing }: { data: Dashboard; hooksMissing: boolean }) {
  const now = Date.now();
  const delta = data.today_cost - data.yesterday_cost_by_now;
  const replies = new Map(data.replies.map((r) => [r.session_id, r]));
  const needs = Math.max(data.needs_you, data.approvals.length);
  return (
    <>
      {data.approvals.length > 0 && (
        <section className="approvals" aria-label="Waiting for your approval">
          {data.approvals.map((a) => <ApprovalCard key={a.request_id} a={a} />)}
        </section>
      )}

      <section className="kpis" aria-label="Right now">
        <Kpi label="Need you" icon={<AlertIcon />} value={needs} tone={needs ? "needs" : undefined} note={needs ? "waiting on you" : "nothing waiting"} />
        <Kpi label="Working" icon={<SpinIcon />} value={data.working} tone={data.working ? "working" : undefined} note="mid-task" />
        <Kpi label="Sub-agents" icon={<AgentsIcon />} value={data.agents_running} tone={data.agents_running ? "agents" : undefined} note="helping sessions" />
        <Kpi
          label="Today"
          icon={<SparkIcon size={14} />}
          value={money(data.today_cost)}
          note={<><span className={delta > 0 ? "up" : "down"}>{delta >= 0 ? "+" : "−"}{money(Math.abs(delta))}</span> vs yesterday</>}
        />
        <Kpi label="Burn rate" icon={<PulseIcon size={14} />} value={<>{money(data.burn_rate_per_hour)}<small>/h</small></>} note="last 60 minutes" />
      </section>

      {data.limits.length > 0 && (
        <section className="card">
          <div className="card-head">
            <h2 className="card-title">Plan usage</h2>
            <span className="muted small-inline">reported by the agent's own logs</span>
          </div>
          <LimitBars limits={data.limits} now={now} />
        </section>
      )}

      {hooksMissing && (
        <p className="callout">
          Claude Code hooks aren't connected, so its states come from the logs: <strong>Done</strong> when the last
          reply finished, otherwise <strong>Working</strong> or <strong>Idle</strong> by recent activity. Connect them in
          <strong> Settings</strong> for instant, exact states and "needs you" alerts.
        </p>
      )}

      <div className="section-head">
        <h2>Sessions</h2>
        <span className="muted">{data.sessions.length} in the last 8 hours</span>
      </div>
      {data.sessions.length === 0 ? (
        <div className="empty">
          <h3>No sessions in the last 8 hours</h3>
          <p className="muted">Start Claude Code, Codex or Copilot CLI in any project and it shows up here.</p>
        </div>
      ) : (
        <div className="sessions">
          {data.sessions.map((s) => <SessionCard key={s.id} s={s} reply={replies.get(s.id)} now={now} />)}
        </div>
      )}
    </>
  );
}
