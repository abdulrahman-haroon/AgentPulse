// Writes realistic *fake* Claude Code logs so AgentPulse can be tried (or screenshotted)
// without anyone's real sessions. Deterministic: same output every run.
//
//   node scripts/demo-logs.mjs <folder>
//   then start AgentPulse with CLAUDE_CONFIG_DIR=<folder>
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
  return t;
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
for (const s of today) {
  const lastAt = writeSession({
    project: s.project, title: s.task[0], prompt: s.task[1], model: "claude-opus-5-5",
    entrypoint: s.project === "mobile-app" ? "cli" : "claude-vscode",
    start: now - s.ago, requests: s.requests, gapMin: s.gapMin, lastStop: s.lastStop,
  });
  if (lastAt > now) throw new Error("demo session ends in the future; lower its requests or gap");
}

console.log(`Wrote ${n} demo requests to ${projectsDir}`);
