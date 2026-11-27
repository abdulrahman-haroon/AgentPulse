import { useState } from "react";
import { testSound, type SoundSettings } from "../api";
import { PlayIcon } from "./Icons";

type Props = { sound: SoundSettings; save: (s: SoundSettings) => Promise<void> };

export function SoundCard({ sound: s, save }: Props) {
  const [error, setError] = useState<string | null>(null);
  const update = (patch: Partial<SoundSettings>) => {
    setError(null);
    save({ ...s, ...patch }).catch((e) => setError(String(e)));
  };
  const play = (kind: "done" | "needs_you") => testSound(kind).catch((e) => setError(String(e)));

  return (
    <section className="card">
      <div className="card-head">
        <h2 className="card-title">Alert sound</h2>
        <label className="switch">
          <input type="checkbox" role="switch" aria-label="Alert sound" checked={s.enabled} onChange={(e) => update({ enabled: e.target.checked })} />
          <span aria-hidden="true" />
        </label>
      </div>
      <p className="muted">
        A short chime with every alert, so you hear it even when the notification is hidden. Two different
        chimes: a soft one when a task finishes, a brighter one when an agent needs you. Follows your system volume.
      </p>
      {s.enabled && (
        <div className="checks">
          <div className="check-line">
            <label className="check-row"><input type="checkbox" checked={s.on_needs_you} onChange={(e) => update({ on_needs_you: e.target.checked })} /> When an agent needs you</label>
            <button className="ghost icon-btn" onClick={() => play("needs_you")}><PlayIcon /> Play</button>
          </div>
          <div className="check-line">
            <label className="check-row"><input type="checkbox" checked={s.on_done} onChange={(e) => update({ on_done: e.target.checked })} /> When a task finishes</label>
            <button className="ghost icon-btn" onClick={() => play("done")}><PlayIcon /> Play</button>
          </div>
        </div>
      )}
      {error && <p className="reply-error" role="alert">{error}</p>}
    </section>
  );
}
