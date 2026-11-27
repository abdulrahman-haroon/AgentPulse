# Changelog

## 0.3.0

AgentPulse now watches more than Claude Code, shows sub-agents, and has a calmer, redesigned look.

**New**
- **GitHub Copilot CLI and OpenAI Codex CLI.** Their sessions appear next to Claude Code's, read from their local logs. Nothing to set up. Copilot sessions get a *Needs you* alert when they wait for a permission. Codex shows your real 5-hour and weekly plan usage.
- **Sub-agents.** See the helpers each session started, what they were asked to do, whether they run in the background, and when they finish.
- **Alert sound.** A quiet, soft chime with every alert: two notes falling when a task finishes, rising when an agent needs you. Each one can be turned off, and the widget has a mute bell.
- **"Not checked yet" badge.** When a task finishes, AgentPulse's taskbar button shows a green badge with the count (like a chat app), the tray icon gets a green dot, and the widget shows "1 new", until you look: open the dashboard, click the session, pick it back up, or click the pill.
- **Floating widget, rebuilt.** Sessions grouped by agent with each agent's spend today, approvals you can answer right there, running sub-agents, plan usage, and a collapsed mode. It sizes itself to its content and remembers where you put it. Click a session to jump to its project in VS Code.
- **Open in VS Code** for any session, including Copilot and Codex.
- **New look and logo.** Redesigned pages, a status-colored tray icon, per-agent cost in Insights and an agent filter in Sessions.

**Fixed**
- *Start with Windows* could point at a developer build that opens blank. Turn it off and on once in Settings to fix an existing entry.
- A session could show its sub-agent's model instead of its own, and Quick reply could continue on that model.
- App notifications were silent.
- Empty "Untitled session" cards for Claude Code windows that were opened and closed without a prompt.

Copilot's and Codex's log formats aren't documented by their makers. Copilot support was checked against real sessions; Codex support follows the community-documented format and hasn't been tested on a real install yet. Please [open an issue](../../issues/new/choose) if something looks off.

## 0.2.0

First public build: Live board, session browser, Insights, History, floating widget, approvals from the app, Quick reply, phone notifications (ntfy), signed in-app updates.
