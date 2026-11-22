import { useState } from "react";
import { randomTopic, testPush, type PushSettings } from "../api";

type Props = { push: PushSettings; save: (push: PushSettings) => Promise<void>; hooksConnected: boolean };

export function PhoneCard({ push: p, save, hooksConnected }: Props) {
  const [server, setServer] = useState(p.server);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [testing, setTesting] = useState(false);

  // Every change is validated and saved by the backend; on error the old settings stay.
  const update = async (patch: Partial<PushSettings>) => {
    setError(null);
    setNote(null);
    const next = { ...p, ...patch };
    if (next.enabled && !next.topic) next.topic = randomTopic();
    try {
      await save(next);
    } catch (e) {
      setError(String(e));
    }
  };
  const test = async () => {
    setTesting(true);
    setError(null);
    setNote(null);
    try {
      await testPush(p);
      setNote("Sent. It should appear on your phone within a few seconds.");
    } catch (e) {
      setError(String(e));
    } finally {
      setTesting(false);
    }
  };
  const copy = () => navigator.clipboard.writeText(p.topic).then(() => setNote("Topic copied.")).catch(() => {});

  return (
    <section className="card">
      <div className="card-head">
        <h2 className="card-title">Phone notifications</h2>
        <label className="switch">
          <input type="checkbox" role="switch" aria-label="Phone notifications" checked={p.enabled} onChange={(e) => update({ enabled: e.target.checked })} />
          <span aria-hidden="true" />
        </label>
      </div>
      <p className="muted">
        Get "finished" and "needs you" alerts on your phone through <strong>ntfy</strong>, a free open-source push
        service, so you can walk away while Claude works. Off by default.
      </p>

      {p.enabled && (
        <div className="phone">
          {!hooksConnected && <p className="callout">Phone alerts come from the Claude Code hooks. Connect them above first.</p>}
          <ol className="steps">
            <li>Install the <strong>ntfy</strong> app (Android, iOS) or open <code>{p.server.replace(/^https:\/\//, "")}</code> in a browser.</li>
            <li>
              Subscribe to this topic:
              <span className="topic">
                <code>{p.topic}</code>
                <button className="ghost" onClick={copy}>Copy</button>
                <button className="ghost" title="Anyone who knows the old topic stops receiving alerts" onClick={() => update({ topic: randomTopic() })}>New topic</button>
              </span>
            </li>
            <li><button className="ghost" disabled={testing} onClick={test}>{testing ? "Sending…" : "Send a test notification"}</button></li>
          </ol>

          <div className="checks">
            <label className="check-row"><input type="checkbox" checked={p.on_done} onChange={(e) => update({ on_done: e.target.checked })} /> When a task finishes</label>
            <label className="check-row"><input type="checkbox" checked={p.on_needs_you} onChange={(e) => update({ on_needs_you: e.target.checked })} /> When Claude needs your approval</label>
            <label className="check-row">
              <input type="checkbox" checked={p.include_details} onChange={(e) => update({ include_details: e.target.checked })} />
              Include project names and Claude's question (otherwise messages are generic, so nothing about your work leaves this computer)
            </label>
          </div>

          <details className="snippet">
            <summary>Advanced: self-hosted ntfy server</summary>
            <p>Use your own server instead of ntfy.sh. Must be https.</p>
            <input className="text" value={server} onChange={(e) => setServer(e.target.value)} onBlur={() => server !== p.server && update({ server })} spellCheck={false} />
          </details>

          <p className="muted small">
            The topic name is the only key on ntfy.sh, which is why it's long and random. Don't share it.
          </p>
        </div>
      )}
      {note && <p className="muted small" role="status">{note}</p>}
      {error && <p className="reply-error" role="alert">{error}</p>}
    </section>
  );
}
