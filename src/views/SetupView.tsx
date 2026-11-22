import { useEffect, useState } from "react";
import {
  connectHooks, disconnectHooks, getAutostart, getSettings, saveSettings, setAutostart,
  type HookStatus, type Settings, type UpdateInfo,
} from "../api";
import { AlertIcon, CheckIcon } from "../components/Icons";
import { UpdatesCard } from "../components/UpdatesCard";
import { PhoneCard } from "../components/PhoneCard";
import { ApprovalsSettingsCard } from "../components/ApprovalsSettingsCard";

const WHAT: Record<string, string> = {
  SessionStart: "a session opens",
  UserPromptSubmit: "you send a prompt → Working",
  Notification: "Claude is waiting for you → Needs you",
  PermissionRequest: "Claude asks to use a tool → approve here (if on)",
  Stop: "a task finishes → Done + notification",
  SessionEnd: "a session closes",
};

type Props = { status: HookStatus | null; setStatus: (s: HookStatus) => void; recheck: () => void; update: UpdateInfo | null };

export function SetupView({ status, setStatus, recheck, update }: Props) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [autostart, setAuto] = useState<boolean | null>(null);
  useEffect(() => { getAutostart().then(setAuto).catch(() => setAuto(null)); }, []);
  // One owner for app settings, so the cards never overwrite each other's changes.
  const [settings, setSettings] = useState<Settings | null>(null);
  useEffect(() => { getSettings().then(setSettings).catch((e) => setError(String(e))); }, []);
  const savePart = async (patch: Partial<Settings>) => {
    if (!settings) return;
    setSettings(await saveSettings({ ...settings, ...patch }));
  };

  const connected = !!status && status.missing.length === 0;
  const partial = !!status && status.installed.length > 0 && !connected;

  const run = async (f: () => Promise<HookStatus>) => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await f());
      setConfirming(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <section className="card">
        <div className="card-head">
          <h2 className="card-title">Claude Code hooks</h2>
          <span className={`pill ${connected ? "pill-ok" : "pill-warn"}`}>
            {connected ? <><CheckIcon /> Connected</> : partial ? <><AlertIcon /> Partly connected</> : <><AlertIcon /> Not connected</>}
          </span>
        </div>
        <p className="muted">
          Hooks tell AgentPulse the moment a session changes state: instant Working / Needs you / Done, and
          notifications. Without them, states are inferred from the logs.
        </p>

        <ul className="checklist">
          {Object.keys(WHAT).map((e) => {
            const ok = status?.installed.includes(e);
            return (
              <li key={e} className={ok ? "ok" : "missing"}>
                {ok ? <CheckIcon /> : <AlertIcon />}
                <code>{e}</code>
                <span className="muted">{WHAT[e]}</span>
                <span className="checklist-state">{ok ? "Connected" : "Not set up"}</span>
              </li>
            );
          })}
        </ul>

        {confirming ? (
          <div className="confirm" role="dialog" aria-label="Confirm connecting hooks">
            <p><strong>AgentPulse will:</strong></p>
            <ul className="plain">
              <li>Back up <code>{status?.settings_path ?? "settings.json"}</code> next to it (timestamped copy).</li>
              <li>Add one AgentPulse hook to each of the 6 events above. Your other settings and hooks stay exactly as they are.</li>
              <li>Install the hook script into AgentPulse's own data folder, so it keeps working wherever this app lives.</li>
            </ul>
            <p className="muted">New Claude Code sessions pick it up. Already-open sessions need a restart. You can disconnect at any time.</p>
            <div className="confirm-actions">
              <button className="ghost" onClick={() => setConfirming(false)}>Cancel</button>
              <button className="primary" disabled={busy} onClick={() => run(connectHooks)}>Connect hooks</button>
            </div>
          </div>
        ) : (
          <div className="setup-actions">
            {!connected && <button className="primary" onClick={() => setConfirming(true)}>{partial ? "Repair connection…" : "Connect hooks…"}</button>}
            {(connected || partial) && <button className="ghost" disabled={busy} onClick={() => run(disconnectHooks)}>Disconnect</button>}
            <button className="ghost" onClick={recheck}>Check again</button>
          </div>
        )}
        {status?.backup_path && <p className="muted small">Backup saved: <code>{status.backup_path}</code></p>}
        {error && <p className="reply-error" role="alert">{error}</p>}
      </section>

      <section className="card">
        <div className="card-head">
          <h2 className="card-title">Start with Windows</h2>
          <label className="switch">
            <input
              type="checkbox"
              role="switch"
              checked={!!autostart}
              disabled={autostart === null}
              onChange={(e) => setAutostart(e.target.checked).then(setAuto).catch((err) => setError(String(err)))}
            />
            <span aria-hidden="true" />
          </label>
        </div>
        <p className="muted">Starts quietly in the tray when you sign in, so notifications and the Live board are always ready.</p>
      </section>

      {settings && (
        <>
          <ApprovalsSettingsCard
            approvals={settings.approvals}
            save={(approvals) => savePart({ approvals })}
            hookReady={!!status?.installed.includes("PermissionRequest")}
          />
          <PhoneCard push={settings.push} save={(push) => savePart({ push })} hooksConnected={connected} />
        </>
      )}

      <UpdatesCard initial={update} />

      <section className="card">
        <h2 className="card-title">Privacy</h2>
        <ul className="plain">
          <li>Everything stays on this computer. No accounts, servers, or telemetry. Network requests happen only for the update check to GitHub (optional) and phone notifications (off unless you turn them on).</li>
          <li>Claude Code logs are only read. The only file AgentPulse ever changes is settings.json, when you click Connect or Disconnect.</li>
          <li>No network ports are opened; hooks talk to the app through a local file.</li>
        </ul>
      </section>
    </>
  );
}
