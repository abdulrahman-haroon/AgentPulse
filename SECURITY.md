# Security Policy

AgentPulse runs entirely on your machine. It reads your local Claude Code logs **read-only**, contains no telemetry, and uses the network only for an optional update check to this repository's GitHub Releases and, if the user turns it on, phone notifications via ntfy (generic text by default). Updates are signed, and the app refuses any update whose signature doesn't match the public key built into it.

## Reporting a vulnerability

Please **do not open a public issue**. Use GitHub's private reporting instead:
**Security tab → "Report a vulnerability"** on this repository.

Include what you found, how to reproduce it, and the impact. You'll get a reply as soon as possible.

## Scope

In scope: anything that lets AgentPulse leak log contents, run code it shouldn't, write outside its own data folder (other than the user-approved hook entries in `~/.claude/settings.json`), install an unsigned or tampered update, approve a Claude Code permission request the user didn't approve (or approve a different request than the one shown), or be controlled by another program or website.
