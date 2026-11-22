# Security Policy

AgentPulse runs entirely on your machine. It reads your local AI-agent logs **read-only**, sends nothing anywhere by default, and contains no telemetry.

## Reporting a vulnerability

Please **do not open a public issue**. Use GitHub's private reporting instead:
**Security tab → "Report a vulnerability"** on this repository.

Include what you found, how to reproduce it, and the impact. You'll get a reply as soon as possible.

## Scope

In scope: anything that lets AgentPulse leak log contents, run code it shouldn't, write outside its own data folder, or be controlled by another program or website.
