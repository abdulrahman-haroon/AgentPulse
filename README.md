# AgentPulse

**Know what your AI coding agents are doing without watching them.**

AgentPulse is a small, open-source tray app for Claude Code (Codex next). It pops up a notification the moment a task finishes or needs your approval, and shows how many tokens you've used and what they'd cost. Everything stays on your computer.

> **Status:** early. Notifications work today on Windows (v0). The tray app is being built.

---

## 1. What it does

| Feature | Status |
|---|---|
| Notification when Claude Code **finishes** a task | ✅ Works (Windows) |
| Notification when Claude Code **needs your input** | ✅ Works (Windows) |
| Tray app with today's tokens and estimated cost | 🧪 Works from source (no installer yet) |
| Stats per day / project / model | 📋 Planned |
| Codex support | 📋 Planned |
| Phone notifications (ntfy, opt-in) | 📋 Planned |
| macOS / Linux | 📋 Planned |

## 2. Quick start: notifications (Windows, 2 minutes)

1. Clone or download this repo, e.g. to `C:\path\to\AgentPulse`.
2. Open `%USERPROFILE%\.claude\settings.json` and add this `hooks` block (merge it if you already have one). Replace the path with yours and use forward slashes:

```json
{
  "hooks": {
    "Stop": [
      { "hooks": [ { "type": "command", "command": "powershell -NoProfile -ExecutionPolicy Bypass -File \"C:/path/to/AgentPulse/hooks/notify.ps1\"" } ] }
    ],
    "Notification": [
      { "hooks": [ { "type": "command", "command": "powershell -NoProfile -ExecutionPolicy Bypass -File \"C:/path/to/AgentPulse/hooks/notify.ps1\"" } ] }
    ]
  }
}
```

3. Start a new Claude Code session and give it a task. You'll get a toast when it's done.

`-ExecutionPolicy Bypass` applies only to this one script. Read [hooks/notify.ps1](hooks/notify.ps1) first; it's about 45 lines.

## 3. How it works

```
                                          ┌─ app running? ─► events.jsonl ─► AgentPulse ─► notification
Claude Code ──(hook: Stop / Notification)─┤
                                          └─ app closed?  ─► hooks/notify.ps1 shows a toast itself

Claude Code ──(writes session logs)──► ~/.claude/projects/**/*.jsonl ──(read-only)──► AgentPulse ──► today's stats
```

- **Hooks** are Claude Code's official way to run a command when something happens. AgentPulse uses them for instant notifications, so there's no polling of logs.
- **Events file**: when the app is running, the hook appends one line to `%LOCALAPPDATA%\dev.agentpulse.app\events.jsonl` and the app shows the notification. A file is used instead of a network port so no website or other program can talk to the app.
- **Logs**: Claude Code saves every session as a JSONL file. The app reads token counts from them (read-only) and estimates cost at API prices. Pro/Max subscribers aren't actually billed per token; the number shows what the usage is worth.

### Project layout

```
hooks/notify.ps1        Claude Code hook (works with or without the app)
src/                    React UI (one window: today's tokens and cost)
src-tauri/src/
  lib.rs                app setup: tray icon, window, command
  usage.rs              reads Claude Code logs, sums tokens per model
  pricing.rs            model prices and cost math
  events.rs             watches the events file, shows notifications
src-tauri/capabilities/ what the UI is allowed to call (only the usage summary)
```

## 4. Privacy and security

- **100% local.** No accounts, no telemetry, no servers. Nothing leaves your machine unless you turn on an opt-in feature such as phone notifications.
- **Read-only** access to agent logs.
- **No open ports.** Nothing on your network or in your browser can talk to AgentPulse.
- Hook input is treated as untrusted data and is only ever displayed as plain text.
- Found a problem? See [SECURITY.md](SECURITY.md).

## 5. Roadmap

1. **v0:** Windows notifications via hooks ✅
2. **v1:** Tray app (Tauri 2 + React): notifications, today's usage, estimated cost
3. **v2:** Stats dashboard, Codex support, ntfy phone push
4. **v3:** macOS / Linux builds, session browser

## 6. Development

Requirements: [Rust](https://rustup.rs) (stable, MSVC on Windows), Node.js 20+, and on Windows the Visual Studio C++ Build Tools.

```bash
npm install          # frontend dependencies
npm run tauri dev    # run the app with hot reload
npm test             # Rust unit tests (parser, pricing, events)
npm run tauri build  # build an installer into src-tauri/target/release/bundle
```

Closing the window hides it to the tray. Use the tray menu's **Quit** to exit.

## License

[MIT](LICENSE)
