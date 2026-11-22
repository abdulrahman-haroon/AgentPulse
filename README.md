# AgentPulse

**Mission control for your AI coding agents.**

Running Claude Code in three terminals at once? AgentPulse shows every session on one screen: which ones are working, which one is stuck waiting for your approval, and what each is costing. It notifies you the moment a task finishes, and shows trends your editor can't. Everything stays on your computer.

> **Status:** early. Works from source on Windows. No installer yet.

---

## 1. What it does

| Page | What you get |
|---|---|
| **Live** | A card per Claude Code session across all projects and terminals: *Working*, *Needs you* (with the permission question), *Done*, or *Idle*, with its auto-generated title, project, model and cost so far. |
| **Open** | Jump straight to a session: VS Code sessions focus their VS Code window; finished terminal sessions reopen in a new terminal with `claude --resume`. |
| **Quick reply** | Type a follow-up on any finished session and send it without leaving AgentPulse. It runs in the background on the session's own model, streams the answer into the card, and you choose the permissions: *Ask first*, *Allow edits*, or *Plan only*. |
| **Insights** | Today's cost vs. yesterday at the same time, burn rate ($/hour right now), money saved by prompt caching, most expensive session, average per session and per request, cost by model. |
| **History** | 7 or 30-day cost chart, top projects, and a weekday × hour heatmap of when you actually work with Claude. |
| **Setup** | Shows which Claude Code hooks are connected, with copy-ready config. |
| **Floating widget** | A small always-on-top card with today's cost and active sessions, for a second monitor. |
| **Tray icon** | A status light: grey = quiet, green = something's working, amber = a session needs you. |
| **Notifications** | Native popups when a task finishes or needs your approval, even when the app is closed. |

Planned: Codex support, phone push via ntfy (opt-in), approve/deny permission prompts from the app, installer, macOS/Linux.

## 2. Quick start (Windows)

1. Clone this repo, e.g. to `C:\path\to\AgentPulse`.
2. Install the requirements and start the app (see [Development](#6-development)): `npm install`, then `npm run tauri dev`.
3. Connect Claude Code. Open `%USERPROFILE%\.claude\settings.json` and add this `hooks` block (merge it if you already have one). Replace the path with yours, using forward slashes:

```json
{
  "hooks": {
    "SessionStart":     [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -ExecutionPolicy Bypass -File \"C:/path/to/AgentPulse/hooks/notify.ps1\"" }] }],
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -ExecutionPolicy Bypass -File \"C:/path/to/AgentPulse/hooks/notify.ps1\"" }] }],
    "Notification":     [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -ExecutionPolicy Bypass -File \"C:/path/to/AgentPulse/hooks/notify.ps1\"" }] }],
    "Stop":             [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -ExecutionPolicy Bypass -File \"C:/path/to/AgentPulse/hooks/notify.ps1\"" }] }],
    "SessionEnd":       [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -ExecutionPolicy Bypass -File \"C:/path/to/AgentPulse/hooks/notify.ps1\"" }] }]
  }
}
```

4. Start a new Claude Code session. The **Setup** page should show all five hooks as connected.

Without hooks, everything still works, but Live states come from the logs (*Done* when Claude's last reply finished, otherwise *Working* or *Idle* by recent activity) and there are no "needs you" alerts.

`-ExecutionPolicy Bypass` applies only to this one script. Read [hooks/notify.ps1](hooks/notify.ps1) first; it's about 50 lines. Each hook adds roughly a quarter of a second while PowerShell starts.

## 3. How it works

```
Claude Code ──(5 hooks)──► hooks/notify.ps1 ──append──► %LOCALAPPDATA%\dev.agentpulse.app\events.jsonl
                                 │                                   │ (tailed every 750 ms)
                                 └─ app closed? shows a toast itself  ▼
                                                              AgentPulse (Rust) ──► live state, tray light, notifications
Claude Code ──(writes)──► ~/.claude/projects/**/*.jsonl ──read-only──┘          └──► dashboard (React)
```

- **Hooks** are Claude Code's official way to run a command when something happens. They tell AgentPulse exactly when a session starts, gets a prompt, needs permission, finishes or closes.
- **Events file**: the hook writes one line per event to a local file and the app tails it. A file is used instead of a network port, so no website or other program can talk to the app.
- **Logs**: Claude Code saves every session as JSONL. AgentPulse reads token counts and session titles from them, read-only. It parses history once, then only reads new bytes, so even large log folders stay fast.
- **Cost** is what the tokens would cost at Anthropic API prices, including cache read and write rates. Pro/Max subscribers aren't billed per token; the number shows what the usage is worth.
- **Quick reply** uses Claude Code's official headless mode: `claude -p --resume <session> --output-format stream-json`, with your message sent on stdin. It's only offered for sessions that are *Done* or *Idle*, so it never competes with a session that's running in an editor or terminal. VS Code shows the new messages next time you open that session.

### Project layout

```
hooks/notify.ps1          Claude Code hook (works with or without the app)
src/
  Dashboard.tsx           sidebar shell and page switching
  Widget.tsx              floating widget window
  views/                  Live, Insights, History, Setup pages
  components/             charts (SVG), status badges, icons
  api.ts, format.ts       typed bridge to Rust, number/time formatting
src-tauri/src/
  lib.rs                  app setup: tray light, windows, commands
  logs.rs                 incremental, de-duplicating log reader
  stats.rs                turns requests + live state into the dashboard
  events.rs               tails the events file, session state, notifications
  actions.rs              Open / Quick reply: the only code that starts processes
  pricing.rs              model prices and cost math
src-tauri/capabilities/   exactly which commands each window may call
```

## 4. Privacy and security

- **100% local.** No accounts, no telemetry, no servers.
- **Read-only** access to Claude Code logs and settings.
- **No open ports.** Nothing on your network or in your browser can talk to AgentPulse.
- **Locked-down UI.** Each window can call only the app functions it needs; no file, shell or network access. Strict Content Security Policy.
- **Launching Claude Code / VS Code safely.** The UI only sends a session ID (and your reply text). Folders come from AgentPulse's own log index, session IDs must be UUIDs, programs are found on PATH by fixed name, arguments are passed without a shell, reply text goes through stdin, and Claude Code session variables inherited from a parent session are stripped. Replies never use "bypass permissions".
- Hook input is treated as untrusted data: it's re-serialized as JSON, cleaned of control characters, length-capped, and only ever shown as plain text.
- Found a problem? See [SECURITY.md](SECURITY.md).

## 5. Roadmap

1. ✅ Notifications via hooks
2. ✅ Tray app: Live board, Insights, History, floating widget, Open and Quick reply
3. Installer, autostart, single-instance, one-click hook setup
4. Codex support, ntfy phone push, approve/deny permission prompts from the app
5. macOS / Linux builds

## 6. Development

Requirements: [Rust](https://rustup.rs) (stable, MSVC on Windows), Node.js 20+, and on Windows the Visual Studio C++ Build Tools.

```bash
npm install          # frontend dependencies
npm run tauri dev    # run the app with hot reload
npm test             # Rust unit tests (log parsing, pricing, stats, events, actions)
# optional real end-to-end reply test (costs a few cents, use a throwaway session):
# AGENTPULSE_TEST_SESSION=<uuid> AGENTPULSE_TEST_CWD=<its folder> cargo test --manifest-path src-tauri/Cargo.toml -- --ignored real_reply
npm run tauri build  # build an installer into src-tauri/target/release/bundle
```

Closing a window hides it to the tray. Use the tray menu's **Quit** to exit.

## License

[MIT](LICENSE)
