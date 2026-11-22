import { useEffect, useState } from "react";
import { appVersion, autoCheckEnabled, checkUpdate, installUpdate, setAutoCheck, type UpdateInfo } from "../api";

export function UpdatesCard({ initial }: { initial: UpdateInfo | null }) {
  const [version, setVersion] = useState<string>("");
  const [info, setInfo] = useState<UpdateInfo | null>(initial);
  const [state, setState] = useState<"idle" | "checking" | "installing">("idle");
  const [error, setError] = useState<string | null>(null);
  const [auto, setAuto] = useState(autoCheckEnabled());
  useEffect(() => { appVersion().then(setVersion).catch(() => {}); }, []);

  const check = async () => {
    setState("checking");
    setError(null);
    try {
      setInfo(await checkUpdate());
    } catch (e) {
      setError(String(e));
    } finally {
      setState("idle");
    }
  };
  const install = async () => {
    setState("installing");
    setError(null);
    try {
      await installUpdate(); // the app restarts on success
    } catch (e) {
      setError(String(e));
      setState("idle");
    }
  };

  return (
    <section className="card">
      <div className="card-head">
        <h2 className="card-title">Updates</h2>
        <span className="muted">Version {version || "…"}</span>
      </div>

      {info?.available ? (
        <div className="confirm">
          <p><strong>Version {info.available} is available.</strong></p>
          {info.notes && <p className="reply-text">{info.notes}</p>}
          <p className="muted">The download is checked against AgentPulse's signing key before it installs. The app restarts when it's done.</p>
          <div className="confirm-actions">
            <button className="primary" disabled={state !== "idle"} onClick={install}>
              {state === "installing" ? "Installing…" : "Install and restart"}
            </button>
          </div>
        </div>
      ) : (
        <div className="setup-actions">
          <button className="ghost" disabled={state !== "idle"} onClick={check}>
            {state === "checking" ? "Checking…" : "Check for updates"}
          </button>
          {info && !info.available && <span className="muted update-ok">You're up to date.</span>}
        </div>
      )}
      {error && <p className="reply-error" role="alert">{error}</p>}

      <label className="check-row">
        <input type="checkbox" checked={auto} onChange={(e) => { setAuto(e.target.checked); setAutoCheck(e.target.checked); }} />
        Check for updates when AgentPulse starts (contacts GitHub; nothing about you or your usage is sent)
      </label>
    </section>
  );
}
