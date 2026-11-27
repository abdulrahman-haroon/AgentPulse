import { useEffect, useState } from "react";
import type { CopilotPlan, LimitSettings } from "../api";

// Included monthly AI credits per GitHub's individual plans (docs.github.com, checked 2026-11-27).
const PLANS: { value: CopilotPlan; label: string }[] = [
  { value: "", label: "Don't show" },
  { value: "pro", label: "Copilot Pro (1,500 credits)" },
  { value: "pro_plus", label: "Copilot Pro+ (7,000 credits)" },
  { value: "max", label: "Copilot Max (20,000 credits)" },
  { value: "custom", label: "Other (enter credits)" },
];

type Props = { limits: LimitSettings; save: (l: LimitSettings) => Promise<void>; statusLineReady: boolean };

export function PlanLimitsCard({ limits: l, save, statusLineReady }: Props) {
  const [error, setError] = useState<string | null>(null);
  const [custom, setCustom] = useState(l.copilot_custom_credits ? String(l.copilot_custom_credits) : "");
  useEffect(() => setCustom(l.copilot_custom_credits ? String(l.copilot_custom_credits) : ""), [l.copilot_custom_credits]);
  const update = (patch: Partial<LimitSettings>) => {
    setError(null);
    save({ ...l, ...patch }).catch((e) => setError(String(e)));
  };
  const pickPlan = (plan: CopilotPlan) => {
    // "Other" is saved once a number is entered, so validation never trips on an empty field.
    if (plan === "custom" && !l.copilot_custom_credits) { update({ copilot_plan: plan, copilot_custom_credits: 1500 }); return; }
    update({ copilot_plan: plan });
  };

  return (
    <section className="card">
      <div className="card-head">
        <h2 className="card-title">Plan limits</h2>
      </div>
      <ul className="plain">
        <li>
          <strong>Claude Code</strong>: your real 5-hour and weekly usage (and spend limit, if your plan has one),
          as Claude Code reports it to its status line. Pro and Max plans only; it updates whenever a session replies.{" "}
          {statusLineReady ? <span className="muted">Connected.</span> : <span>Needs the <code>StatusLine</code> step: click <strong>Connect hooks…</strong> or <strong>Repair connection…</strong> above.</span>}
        </li>
        <li><strong>Codex</strong>: your real 5-hour and weekly usage from its own logs. Nothing to set up.</li>
        <li>
          <strong>Copilot</strong>: an <em>estimate</em> of this month's AI credits, counted from this computer's
          Copilot CLI logs (VS Code chat and other devices aren't included). Resets on the 1st (UTC). Pick your plan:
        </li>
      </ul>
      <label className="check-row wait-row">
        Copilot plan
        <select value={l.copilot_plan} onChange={(e) => pickPlan(e.target.value as CopilotPlan)}>
          {PLANS.map((p) => <option key={p.value} value={p.value}>{p.label}</option>)}
        </select>
        {l.copilot_plan === "custom" && (
          <>
            <input
              type="number"
              min={1}
              step={100}
              inputMode="numeric"
              aria-label="Monthly AI credits"
              value={custom}
              onChange={(e) => setCustom(e.target.value)}
              onBlur={() => { const n = Math.round(Number(custom)); if (n > 0 && n !== l.copilot_custom_credits) update({ copilot_custom_credits: n }); }}
            />
            credits a month
          </>
        )}
      </label>
      <p className="muted small">Check the figure against github.com › Settings › Billing › Usage once; plans and seats can differ.</p>
      {error && <p className="reply-error" role="alert">{error}</p>}
    </section>
  );
}
