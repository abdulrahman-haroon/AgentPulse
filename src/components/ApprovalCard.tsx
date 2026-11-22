import { useEffect, useState } from "react";
import { decidePermission, type Approval } from "../api";
import { AlertIcon } from "./Icons";

/** A permission prompt held open by the hook. Shows the full tool input; nothing is pre-selected. */
export function ApprovalCard({ a }: { a: Approval }) {
  const [now, setNow] = useState(Date.now());
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);

  const left = Math.max(0, Math.round((new Date(a.expires_at).getTime() - now) / 1000));
  const decide = async (allow: boolean) => {
    setBusy(true);
    setError(null);
    try {
      await decidePermission(a.request_id, allow, reason);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  return (
    <article className="approval" aria-label={`Approval needed: ${a.tool_name}`}>
      <header className="approval-head">
        <span className="badge badge-needs_you"><AlertIcon /> Approval needed</span>
        <span className="session-time">{left > 0 ? `falls back to Claude Code in ${left}s` : "answer it in Claude Code"}</span>
      </header>
      <p className="approval-who">
        <strong>{a.session_title}</strong> <span className="muted">· {a.project}</span>
      </p>
      <p className="approval-what">
        Claude wants to use <code className="tool">{a.tool_name}</code>
      </p>
      {a.summary && <pre className="approval-summary">{a.summary}</pre>}
      <details className="approval-detail" open={!a.summary}>
        <summary>Full request</summary>
        <pre>{a.detail}</pre>
      </details>
      <div className="approval-actions">
        <input
          className="text"
          placeholder="Reason for denying (optional, Claude sees it)"
          value={reason}
          maxLength={500}
          onChange={(e) => setReason(e.target.value)}
          disabled={busy || left === 0}
        />
        <button className="danger" disabled={busy || left === 0} onClick={() => decide(false)}>Deny</button>
        <button className="primary" disabled={busy || left === 0} onClick={() => decide(true)}>Allow once</button>
      </div>
      {error && <p className="reply-error" role="alert">{error}</p>}
    </article>
  );
}
