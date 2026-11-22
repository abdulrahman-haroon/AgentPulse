import { useState } from "react";
import type { ApprovalSettings } from "../api";

const WAITS = [30, 60, 120, 300];

type Props = { approvals: ApprovalSettings; save: (a: ApprovalSettings) => Promise<void>; hookReady: boolean };

export function ApprovalsSettingsCard({ approvals: a, save, hookReady }: Props) {
  const [error, setError] = useState<string | null>(null);
  const update = (patch: Partial<ApprovalSettings>) => {
    setError(null);
    save({ ...a, ...patch }).catch((e) => setError(String(e)));
  };

  return (
    <section className="card">
      <div className="card-head">
        <h2 className="card-title">Approve in AgentPulse</h2>
        <label className="switch">
          <input type="checkbox" role="switch" aria-label="Approve in AgentPulse" checked={a.enabled} onChange={(e) => update({ enabled: e.target.checked })} />
          <span aria-hidden="true" />
        </label>
      </div>
      <p className="muted">
        When Claude Code asks for permission (run a command, edit a file…), the request shows up on the Live page
        first, with the full command, so you can <strong>Allow once</strong> or <strong>Deny</strong> without
        switching windows. If you don't answer in time, Claude Code shows its normal prompt. Nothing is ever
        approved automatically.
      </p>
      {a.enabled && (
        <>
          {!hookReady && <p className="callout">Needs the <code>PermissionRequest</code> hook: click <strong>Repair connection…</strong> above.</p>}
          <label className="check-row wait-row">
            Wait for my answer here for
            <select value={a.wait_seconds} onChange={(e) => update({ wait_seconds: Number(e.target.value) })}>
              {WAITS.map((w) => <option key={w} value={w}>{w < 60 ? `${w} seconds` : `${w / 60} minute${w > 60 ? "s" : ""}`}</option>)}
            </select>
            before falling back to Claude Code's prompt.
          </label>
          <p className="muted small">
            While it waits, the prompt doesn't appear in your terminal or VS Code. Only works while AgentPulse is
            running; otherwise Claude Code asks as usual.
          </p>
        </>
      )}
      {error && <p className="reply-error" role="alert">{error}</p>}
    </section>
  );
}
