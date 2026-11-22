import type { HookStatus } from "../api";
import { AlertIcon, CheckIcon } from "../components/Icons";

const WHAT: Record<string, string> = {
  SessionStart: "a session opens",
  UserPromptSubmit: "you send a prompt → Working",
  Notification: "Claude asks for permission → Needs you",
  Stop: "a task finishes → Done + notification",
  SessionEnd: "a session closes",
};

const SNIPPET = `"hooks": {
  "SessionStart":     [{ "hooks": [{ "type": "command", "command": "<CMD>" }] }],
  "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "<CMD>" }] }],
  "Notification":     [{ "hooks": [{ "type": "command", "command": "<CMD>" }] }],
  "Stop":             [{ "hooks": [{ "type": "command", "command": "<CMD>" }] }],
  "SessionEnd":       [{ "hooks": [{ "type": "command", "command": "<CMD>" }] }]
}`.split("<CMD>").join(`powershell -NoProfile -ExecutionPolicy Bypass -File \\"C:/path/to/AgentPulse/hooks/notify.ps1\\"`);

export function SetupView({ status, recheck }: { status: HookStatus | null; recheck: () => void }) {
  const check = recheck;
  return (
    <>
      <section className="card">
        <div className="card-head">
          <h2 className="card-title">Claude Code hooks</h2>
          <button className="ghost" onClick={check}>Check again</button>
        </div>
        <p className="muted">
          Hooks tell AgentPulse the moment a session changes state. Without them, the Live board guesses from log activity.
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
        <details className="snippet">
          <summary>How to connect</summary>
          <p>
            Add this to <code>%USERPROFILE%\.claude\settings.json</code> (merge with any existing <code>hooks</code>),
            replacing the path with where you cloned AgentPulse. Then start a new Claude Code session.
          </p>
          <pre>{SNIPPET}</pre>
        </details>
      </section>

      <section className="card">
        <h2 className="card-title">Privacy</h2>
        <ul className="plain">
          <li>Everything stays on this computer. No accounts, servers, or telemetry.</li>
          <li>Claude Code logs are read, never modified.</li>
          <li>No network ports are opened; hooks talk to the app through a local file.</li>
        </ul>
      </section>
    </>
  );
}
