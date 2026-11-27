// Writes realistic *fake* Claude Code logs so AgentPulse can be tried (or screenshotted)
// without anyone's real sessions. Deterministic: same output every run.
//
//   node scripts/demo-logs.mjs <folder>
//   then start AgentPulse with CLAUDE_CONFIG_DIR=<folder> CODEX_HOME=<folder>/codex COPILOT_HOME=<folder>/copilot
//
// Only the fields AgentPulse reads are generated; all names, prompts and numbers are made up.
import { mkdirSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const out = process.argv[2];
if (!out) {
  console.error("usage: node scripts/demo-logs.mjs <folder>");
  process.exit(1);
}

// Small seeded PRNG (mulberry32) so screenshots are reproducible.
let seed = 42;
const rand = () => {
  seed = (seed + 0x6d2b79f5) | 0;
  let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};
const pick = (a) => a[Math.floor(rand() * a.length)];
const int = (lo, hi) => lo + Math.floor(rand() * (hi - lo + 1));
const uuid = () =>
  "xxxxxxxx-xxxx-4xxx-8xxx-xxxxxxxxxxxx".replace(/x/g, () => Math.floor(rand() * 16).toString(16));

const PROJECTS = ["web-shop", "api-gateway", "mobile-app", "data-pipeline", "docs-site", "design-system"];
const TASKS = [
  ["Add dark mode toggle", "add a dark mode toggle to the settings page and remember the choice"],
  ["Fix flaky checkout test", "the checkout e2e test fails about 1 in 5 runs, find out why"],
  ["Migrate auth to OAuth 2.1", "move login from sessions to OAuth 2.1 with PKCE"],
  ["Speed up product search", "search takes 2s on large catalogs, profile it and make it faster"],
  ["Write API rate limiting", "add per-key rate limiting to the public API, 100 req/min"],
  ["Refactor payment service", "split PaymentService into smaller pieces without changing behavior"],
  ["Add CSV export", "let admins export the orders table as CSV"],
  ["Upgrade to React 19", "upgrade the app to React 19 and fix the breaking changes"],
  ["Fix memory leak in worker", "the background worker grows to 2 GB after a day, find the leak"],
  ["Document the deploy process", "write a short guide for deploying to staging and production"],
  ["Add retry to webhook sender", "webhooks fail when the receiver is slow, add retries with backoff"],
  ["Improve onboarding emails", "rewrite the 3 onboarding emails to be shorter and clearer"],
  ["Set up CI caching", "CI takes 14 minutes, add caching for dependencies and builds"],
  ["Accessibility pass on forms", "check all forms for labels, focus order and error messages"],
];
const MODELS = ["claude-opus-5-5", "claude-opus-5-5", "claude-opus-5-5", "claude-sonnet-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"];

rmSync(out, { recursive: true, force: true });
const projectsDir = join(out, "projects");
const now = Date.now();
const MIN = 60_000;
const DAY = 24 * 60 * MIN;
let n = 0;

function writeSession({ project, title, prompt, model, entrypoint, start, requests, gapMin, lastStop }) {
  const id = uuid();
  const cwd = `C:\\Users\\dev\\code\\${project}`;
  const dir = join(projectsDir, `C--Users-dev-code-${project}`);
  mkdirSync(dir, { recursive: true });
  const lines = [];
  let t = start;
  for (let i = 0; i < requests; i++) {
    const last = i === requests - 1;
    lines.push(
      JSON.stringify({
        type: "assistant",
        timestamp: new Date(t).toISOString(),
        requestId: `req_demo${++n}`,
        sessionId: id,
        cwd,
        entrypoint,
        message: {
          id: `msg_demo${n}`,
          model,
          stop_reason: last ? lastStop : "tool_use",
          usage: {
            input_tokens: int(5, 80),
            output_tokens: int(150, 1600),
            cache_read_input_tokens: int(20_000, 90_000),
            cache_creation_input_tokens: int(500, 6_000),
            cache_creation: { ephemeral_5m_input_tokens: int(500, 6_000), ephemeral_1h_input_tokens: 0 },
          },
        },
      }),
    );
    if (!last) t += int(10_000, gapMin * MIN);
  }
  lines.push(JSON.stringify({ type: "ai-title", aiTitle: title, sessionId: id }));
  lines.push(JSON.stringify({ type: "last-prompt", lastPrompt: prompt, sessionId: id }));
  const file = join(dir, `${id}.jsonl`);
  writeFileSync(file, lines.join("\n") + "\n");
  utimesSync(file, new Date(t), new Date(t)); // the app only reads files touched in the last 31 days
  return { t, id, dir };
}

// A Claude Code sub-agent: its own transcript under <session>/subagents, plus the meta file.
function writeSubagent({ dir, session, type, description, background, finished, start, end }) {
  const agentId = "a" + uuid().replace(/-/g, "").slice(0, 16);
  const sub = join(dir, session, "subagents");
  mkdirSync(sub, { recursive: true });
  const line = (ts, stop) =>
    JSON.stringify({
      type: "assistant", isSidechain: true, agentId, timestamp: new Date(ts).toISOString(), requestId: `req_demo${++n}`,
      sessionId: session, cwd: "C:\\Users\\dev\\code\\web-shop",
      message: {
        id: `msg_demo${n}`, model: "claude-haiku-5-5", stop_reason: stop,
        usage: { input_tokens: 40, output_tokens: int(200, 900), cache_read_input_tokens: int(5_000, 30_000), cache_creation_input_tokens: 800 },
      },
    });
  const file = join(sub, `agent-${agentId}.jsonl`);
  writeFileSync(file, [line(start, "tool_use"), line(end, finished ? "end_turn" : "tool_use")].join("\n") + "\n");
  const meta = { agentType: type, description, requestShape: background ? "background" : "foreground", finished, model: "claude-haiku-5-5" };
  writeFileSync(join(sub, `agent-${agentId}.meta.json`), JSON.stringify(meta));
  utimesSync(file, new Date(end), new Date(end));
}

// A Codex CLI rollout (~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl), shaped like the real format.
function writeCodex({ project, prompt, start, turns, done, limits }) {
  const id = uuid();
  const d = new Date(start);
  const pad = (v) => String(v).padStart(2, "0");
  const dir = join(out, "codex", "sessions", String(d.getFullYear()), pad(d.getMonth() + 1), pad(d.getDate()));
  mkdirSync(dir, { recursive: true });
  const at = (ms) => new Date(ms).toISOString();
  const cwd = `C:\\Users\\dev\\code\\${project}`;
  const lines = [
    { timestamp: at(start), type: "session_meta", payload: { id, cwd, originator: "codex_cli_rs", cli_version: "0.63.0" } },
    { timestamp: at(start), type: "turn_context", payload: { cwd, model: "gpt-5.5", effort: "medium" } },
    { timestamp: at(start + 1000), type: "event_msg", payload: { type: "user_message", message: prompt, images: [] } },
    { timestamp: at(start + 1500), type: "event_msg", payload: { type: "task_started", turn_id: "1" } },
  ];
  let t = start + 2000;
  let total = 0;
  for (let i = 0; i < turns; i++) {
    t += int(20_000, 60_000);
    const input = int(8_000, 40_000);
    const cached = Math.floor(input * 0.7);
    const output = int(300, 2_000);
    total += input + output;
    const rate_limits = limits && {
      primary: { used_percent: limits[0], window_minutes: 300, resets_at: Math.floor((now + 2.4 * 3600_000) / 1000) },
      secondary: { used_percent: limits[1], window_minutes: 10080, resets_at: Math.floor((now + 4 * DAY) / 1000) },
      plan_type: null,
    };
    const last_token_usage = { input_tokens: input, cached_input_tokens: cached, output_tokens: output, reasoning_output_tokens: 100, total_tokens: input + output };
    lines.push({ timestamp: at(t), type: "event_msg", payload: { type: "token_count", info: { total_token_usage: { total_tokens: total }, last_token_usage }, rate_limits } });
  }
  if (done) lines.push({ timestamp: at(t + 1000), type: "event_msg", payload: { type: "task_complete", last_agent_message: "Done." } });
  const file = join(dir, `rollout-${at(start).slice(0, 19).replace(/:/g, "-")}-${id}.jsonl`);
  writeFileSync(file, lines.map((l) => JSON.stringify(l)).join("\n") + "\n");
  utimesSync(file, new Date(t), new Date(t));
}

// A Copilot CLI session (~/.copilot/session-state/<id>/events.jsonl). Undocumented format; this
// follows the shape of real sessions: a model turn per call, usage per call, a checkpoint when done.
function writeCopilot({ project, prompt, start, messages }) {
  const id = uuid();
  const dir = join(out, "copilot", "session-state", id);
  mkdirSync(dir, { recursive: true });
  const at = (ms) => new Date(ms).toISOString();
  let e = 0;
  const ev = (type, ms, data) => ({ type, data, id: `ev${++e}`, timestamp: at(ms), parentId: null });
  const lines = [
    ev("session.start", start, { sessionId: id, selectedModel: "claude-sonnet-4.5", context: { cwd: `/home/dev/code/${project}` } }),
    ev("user.message", start + 2000, { content: prompt }),
  ];
  let t = start + 3000;
  for (let i = 0; i < messages; i++) {
    lines.push(ev("assistant.turn_start", t, { turnId: String(i) }));
    t += int(5_000, 30_000);
    lines.push(ev("session.usage_record", t, { usage: { model: "claude-sonnet-4.5", inputTokens: int(20_000, 60_000), outputTokens: int(100, 900) } }));
    lines.push(ev("assistant.turn_end", t + 200, { turnId: String(i) }));
  }
  lines.push(ev("session.usage_checkpoint", t + 500, { totalPremiumRequests: 1 }));
  const file = join(dir, "events.jsonl");
  writeFileSync(file, lines.map((l) => JSON.stringify(l)).join("\n") + "\n");
  utimesSync(file, new Date(t), new Date(t));
}

// History: ~30 sessions over the last 30 days, mostly weekday working hours.
for (let d = 29; d >= 1; d--) {
  const day = new Date(now - d * DAY);
  const weekend = day.getDay() === 0 || day.getDay() === 6;
  for (let s = 0; s < (weekend ? int(0, 1) : int(0, 2)); s++) {
    const [title, prompt] = pick(TASKS);
    const start = new Date(day);
    start.setHours(int(9, 18), int(0, 59), 0, 0);
    writeSession({
      project: pick(PROJECTS), title, prompt, model: pick(MODELS),
      entrypoint: rand() < 0.6 ? "claude-vscode" : "cli",
      start: start.getTime(), requests: int(15, 140), gapMin: 2, lastStop: "end_turn",
    });
  }
}

// Today: a few sessions, including ones that look live on the board.
const today = [
  { project: "web-shop", task: TASKS[0], ago: 25 * MIN, requests: 60, lastStop: "tool_use", gapMin: 0.4 }, // working
  { project: "api-gateway", task: TASKS[4], ago: 50 * MIN, requests: 45, lastStop: "end_turn", gapMin: 1 }, // done
  { project: "mobile-app", task: TASKS[8], ago: 3 * 60 * MIN, requests: 90, lastStop: "end_turn", gapMin: 1.5 }, // done
  { project: "docs-site", task: TASKS[9], ago: 5 * 60 * MIN, requests: 20, lastStop: "end_turn", gapMin: 3 },
];
const written = [];
for (const s of today) {
  const { t: lastAt, id, dir } = writeSession({
    project: s.project, title: s.task[0], prompt: s.task[1], model: "claude-opus-5-5",
    entrypoint: s.project === "mobile-app" ? "cli" : "claude-vscode",
    start: now - s.ago, requests: s.requests, gapMin: s.gapMin, lastStop: s.lastStop,
  });
  if (lastAt > now) throw new Error("demo session ends in the future; lower its requests or gap");
  written.push({ id, dir });
}

// Sub-agents of the live web-shop session: two still running in the background, one finished.
const live = { dir: written[0].dir, session: written[0].id };
writeSubagent({ ...live, type: "Explore", description: "Find every place the theme is read", background: true, finished: false, start: now - 4 * MIN, end: now - 20_000 });
writeSubagent({ ...live, type: "general-purpose", description: "Run the visual regression tests", background: true, finished: false, start: now - 2 * MIN, end: now - 8_000 });
writeSubagent({ ...live, type: "general-purpose", description: "Audit color contrast in dark mode", background: false, finished: true, start: now - 15 * MIN, end: now - 9 * MIN });

// Codex: some history, one finished today, one mid-turn (with the plan limits Codex logs).
for (let d = 12; d >= 2; d -= 3) writeCodex({ project: pick(PROJECTS), prompt: pick(TASKS)[1], start: now - d * DAY, turns: int(4, 12), done: true });
writeCodex({ project: "data-pipeline", prompt: "backfill the events table for October and verify row counts", start: now - 2 * 60 * MIN, turns: 8, done: true });
writeCodex({ project: "api-gateway", prompt: "add OpenTelemetry tracing to the request pipeline", start: now - 6 * MIN, turns: 4, done: false, limits: [46, 23] });

// Copilot CLI: one finished session today.
writeCopilot({ project: "design-system", prompt: "rename the Button size props to sm, md and lg everywhere", start: now - 70 * MIN, messages: 6 });

// Claude Code settings with AgentPulse's hooks registered, so the demo shows them connected.
// The script path is fake; nothing runs it.
const hook = { type: "command", command: 'powershell -NoProfile -ExecutionPolicy Bypass -File "C:/Users/dev/AppData/Local/dev.agentpulse.app/hooks/notify.ps1"' };
const events = ["SessionStart", "UserPromptSubmit", "Notification", "PermissionRequest", "Stop", "SessionEnd"];
writeFileSync(join(out, "settings.json"), JSON.stringify({ hooks: Object.fromEntries(events.map((e) => [e, [{ hooks: [hook] }]])) }, null, 2));

console.log(`Wrote ${n} demo requests to ${projectsDir}`);
