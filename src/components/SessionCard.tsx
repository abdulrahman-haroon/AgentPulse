import { useState } from "react";
import { cancelReply, dismissReply, focusSession, openSession, sendReply, type PermissionMode, type Reply, type Session } from "../api";
import { money, since } from "../format";
import { StatusBadge } from "./StatusBadge";
import { SpinIcon } from "./Icons";
import { AgentChip, AgentList, ProviderTag } from "./Agents";

const MODES: { id: PermissionMode; label: string; hint: string }[] = [
  { id: "default", label: "Ask first", hint: "Uses your Claude Code permission rules; anything that would need approval is skipped." },
  { id: "acceptEdits", label: "Allow edits", hint: "Claude may edit files in this project without asking." },
  { id: "plan", label: "Plan only", hint: "Claude only reads and plans; it changes nothing." },
];

function timeLabel(s: Session, now: number) {
  if (s.state_since && s.state === "working") return `running ${since(s.state_since, now)}`;
  if (s.state_since && s.state === "needs_you") return `waiting ${since(s.state_since, now)}`;
  return `active ${since(s.last_active, now)} ago`;
}

function openLabel(s: Session) {
  if (s.provider !== "claude") return "Open in VS Code"; // other agents: their project; Resume/Reply drive `claude` only
  if (s.entrypoint.includes("vscode")) return "Open in VS Code";
  return s.state === "working" || s.state === "needs_you" ? null : "Resume in terminal";
}

function ReplyPanel({ r, onAgain }: { r: Reply; onAgain: () => void }) {
  const running = r.status === "running";
  return (
    <div className={`reply reply-${r.status}`} aria-live="polite">
      <div className="reply-head">
        {running ? <><SpinIcon /> Claude is replying…</> : r.status === "done" ? "Reply finished" : r.status === "cancelled" ? "Stopped" : "Reply failed"}
        {r.activity.length > 0 && running && <span className="reply-activity">{r.activity.join(" · ")}</span>}
        {r.cost_usd !== null && <span className="reply-cost">{money(r.cost_usd)}</span>}
      </div>
      {r.text && <p className="reply-text">{r.text}</p>}
      {r.error && <p className="reply-error">{r.error}</p>}
      {r.denied.length > 0 && (
        <p className="reply-denied">
          Skipped because it needed approval: {[...new Set(r.denied)].join(", ")}. Open the session to approve, or reply with "Allow edits".
        </p>
      )}
      <div className="reply-actions">
        {running ? (
          <button className="ghost" onClick={() => cancelReply(r.session_id)}>Stop</button>
        ) : (
          <>
            <button className="ghost" onClick={() => dismissReply(r.session_id)}>Dismiss</button>
            <button className="ghost" onClick={onAgain}>Reply again</button>
          </>
        )}
      </div>
    </div>
  );
}

export function SessionCard({ s, reply, now }: { s: Session; reply?: Reply; now: number }) {
  const [composing, setComposing] = useState(false);
  const [draft, setDraft] = useState("");
  const [mode, setMode] = useState<PermissionMode>("default");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const canReply = s.provider === "claude" && (s.state === "done" || s.state === "idle") && reply?.status !== "running";
  const open = openLabel(s);

  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await f();
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    } finally {
      setBusy(false);
    }
  };
  const send = async () => {
    if (!draft.trim()) return;
    if (await run(() => sendReply(s.id, draft, mode))) {
      setDraft("");
      setComposing(false);
    }
  };

  return (
    <article className={`session session-${s.state}`}>
      <header className="session-head">
        <StatusBadge state={s.state} />
        <ProviderTag provider={s.provider} />
        <AgentChip running={s.agents_running} total={s.agents_total} />
        <span className="session-time">{timeLabel(s, now)}</span>
      </header>
      <h3 className="session-title" title={s.title}>{s.title}</h3>
      <p className="session-meta">
        <span>{s.project || "unknown project"}</span>
        {s.model && <span>{s.model.replace(/^claude-/, "")}</span>}
        {s.provider === "claude" && <span>{s.entrypoint.includes("vscode") ? "VS Code" : "terminal"}</span>}
      </p>
      {s.state === "needs_you" && s.message && <p className="session-message">{s.message}</p>}
      <AgentList agents={s.agents} total={s.agents_total} now={now} />

      <footer className="session-foot">
        <span>
          <span className="session-cost">{s.provider === "copilot" ? "—" : money(s.cost)}</span>
          <span className="muted"> · {s.requests} {s.requests === 1 ? "request" : "requests"}</span>
        </span>
        <span className="session-actions">
          {open && <button className="ghost" disabled={busy} onClick={() => run(() => (s.provider === "claude" ? openSession(s.id) : focusSession(s.id)))}>{open}</button>}
          {canReply && !composing && <button className="primary" onClick={() => setComposing(true)}>Reply</button>}
        </span>
      </footer>

      {composing && canReply && (
        <div className="composer">
          <textarea
            autoFocus
            rows={3}
            value={draft}
            placeholder={`Continue "${s.title}"…   Enter to send · Shift+Enter for a new line`}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                send();
              }
              if (e.key === "Escape") setComposing(false);
            }}
          />
          <div className="composer-bar">
            <div className="filters" role="radiogroup" aria-label="Permissions for this reply">
              {MODES.map((m) => (
                <button key={m.id} role="radio" aria-checked={mode === m.id} title={m.hint} className={`seg ${mode === m.id ? "seg-on" : ""}`} onClick={() => setMode(m.id)}>
                  {m.label}
                </button>
              ))}
            </div>
            <span className="composer-buttons">
              <button className="ghost" onClick={() => setComposing(false)}>Cancel</button>
              <button className="primary" disabled={busy || !draft.trim()} onClick={send}>Send</button>
            </span>
          </div>
          <p className="composer-hint">{MODES.find((m) => m.id === mode)!.hint} Runs in the background; the result appears here.</p>
        </div>
      )}

      {error && <p className="reply-error" role="alert">{error}</p>}
      {reply && <ReplyPanel r={reply} onAgain={() => setComposing(true)} />}
    </article>
  );
}
