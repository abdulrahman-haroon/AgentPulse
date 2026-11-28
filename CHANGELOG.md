# Changelog

## 0.3.5

- **Fixed: a working Claude Code session could show as Done.** When a session was resumed, Claude Code reported "session started" right after your prompt, and AgentPulse took that as "waiting for you".
- **Fixed: your own Claude Code status line could show blank values** after connecting AgentPulse. On computers whose console uses UTF-8 (or without Git Bash), the input passed to it started with an invisible byte-order mark it couldn't read.

## 0.3.3

- **Claude Code plan limits.** See how much of your 5-hour and weekly limits you've used (and your spend limit, if your plan has one), with the reset time, in the widget and on the Live page. The numbers are Claude Code's own, read from its status line (Pro and Max plans). Existing installs: click **Repair connection…** in Settings once; your own status line keeps working.
- **Copilot AI credits.** Pick your Copilot plan in Settings to see an estimate of this month's AI credits used, counted from this computer's Copilot CLI logs.

## 0.3.1

- **Click a Claude Code session to open its chat.** In the widget (and the dashboard's *Open in VS Code*), Claude Code sessions that live in VS Code now open the conversation itself, not just the project. Terminal sessions (Copilot CLI, Codex, Claude Code CLI) still open their project in VS Code.
- **Richer phone notifications.** With *Include details* on, ntfy messages now say which session, what you asked (or the agent's question), how long it worked, the cost so far and the model. Generic messages now name the right app (Copilot and Codex alerts no longer say "Claude"). *Send a test notification* shows an example of exactly what you'll get.
- **Easier to scan widget.** Each agent's sessions sit in their own tinted card (orange Claude Code, violet Copilot, silver Codex), and running or waiting sessions are highlighted.

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
