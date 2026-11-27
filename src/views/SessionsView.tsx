import { useEffect, useMemo, useState } from "react";
import { openSession, sessionHistory, type SessionRow } from "../api";
import { duration, money, tokens } from "../format";
import { SearchIcon } from "../components/Icons";
import { ProviderTag, providerName } from "../components/Agents";

const WHEN = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });

/** Wraps each occurrence of any search word in <mark>. Text stays text: no HTML is built. */
function Highlight({ text, words }: { text: string; words: string[] }) {
  if (!words.length || !text) return <>{text}</>;
  const lower = text.toLowerCase();
  const parts: React.ReactNode[] = [];
  let i = 0;
  while (i < text.length) {
    let hit: { at: number; len: number } | null = null;
    for (const w of words) {
      const at = lower.indexOf(w, i);
      if (at !== -1 && (!hit || at < hit.at)) hit = { at, len: w.length };
    }
    if (!hit) break;
    if (hit.at > i) parts.push(text.slice(i, hit.at));
    parts.push(<mark key={hit.at}>{text.slice(hit.at, hit.at + hit.len)}</mark>);
    i = hit.at + hit.len;
  }
  parts.push(text.slice(i));
  return <>{parts}</>;
}

function Row({ s, words }: { s: SessionRow; words: string[] }) {
  const [error, setError] = useState<string | null>(null);
  const vscode = s.entrypoint.includes("vscode");
  return (
    <li className="srow">
      <div className="srow-main">
        <h3 className="srow-title"><ProviderTag provider={s.provider} /> <Highlight text={s.title} words={words} /></h3>
        {s.last_prompt && s.last_prompt !== s.title && (
          <p className="srow-prompt" title={s.last_prompt}>
            <span className="muted">Last asked: </span><Highlight text={s.last_prompt} words={words} />
          </p>
        )}
        <p className="srow-meta">
          <span><Highlight text={s.project} words={words} /></span>
          <span>{WHEN.format(new Date(s.last_active))}</span>
          <span>{duration(s.active_secs * 1000)} active</span>
          {s.model && <span>{s.model.replace(/^claude-/, "")}</span>}
          <span>{s.requests} requests · {tokens(s.tokens)} tokens</span>
        </p>
        {error && <p className="reply-error" role="alert">{error}</p>}
      </div>
      <div className="srow-side">
        <span className="srow-cost">{s.provider === "copilot" ? "—" : money(s.cost)}</span>
        {s.provider === "claude" && (
          <button className="ghost" onClick={() => openSession(s.id).then(() => setError(null)).catch((e) => setError(String(e)))}>
            {vscode ? "Open in VS Code" : "Resume in terminal"}
          </button>
        )}
      </div>
    </li>
  );
}

export function SessionsView() {
  const [rows, setRows] = useState<SessionRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [project, setProject] = useState("");
  const [agent, setAgent] = useState("");
  const [sort, setSort] = useState<"recent" | "cost">("recent");
  useEffect(() => { sessionHistory().then(setRows).catch((e) => setError(String(e))); }, []);

  const projects = useMemo(() => {
    const count = new Map<string, number>();
    rows?.forEach((r) => count.set(r.project, (count.get(r.project) ?? 0) + 1));
    return [...count.entries()].sort((a, b) => b[1] - a[1]).map(([p]) => p);
  }, [rows]);

  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const shown = useMemo(() => {
    const list = (rows ?? []).filter((r) => {
      if (project && r.project !== project) return false;
      if (agent && r.provider !== agent) return false;
      const hay = `${r.title} ${r.project} ${r.last_prompt}`.toLowerCase();
      return words.every((w) => hay.includes(w)); // every word must match somewhere
    });
    return sort === "cost" ? [...list].sort((a, b) => b.cost - a.cost) : list;
  }, [rows, project, agent, sort, query]);
  const agents = useMemo(() => [...new Set((rows ?? []).map((r) => r.provider))], [rows]);

  if (error) return <p className="callout callout-error" role="alert">Couldn't load sessions: {error}</p>;
  if (!rows) return <div className="loading"><span className="loading-bar" /></div>;

  return (
    <>
      <div className="session-filters">
        <label className="search">
          <SearchIcon />
          <input
            autoFocus
            type="search"
            placeholder="Search titles, projects and what you asked…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label="Search sessions"
          />
        </label>
        <select className="select" value={project} onChange={(e) => setProject(e.target.value)} aria-label="Project">
          <option value="">All projects</option>
          {projects.map((p) => <option key={p} value={p}>{p}</option>)}
        </select>
        {agents.length > 1 && (
          <select className="select" value={agent} onChange={(e) => setAgent(e.target.value)} aria-label="Agent">
            <option value="">All agents</option>
            {agents.map((a) => <option key={a} value={a}>{providerName(a)}</option>)}
          </select>
        )}
        <div className="filters" role="group" aria-label="Sort">
          <button className={`seg ${sort === "recent" ? "seg-on" : ""}`} aria-pressed={sort === "recent"} onClick={() => setSort("recent")}>Recent</button>
          <button className={`seg ${sort === "cost" ? "seg-on" : ""}`} aria-pressed={sort === "cost"} onClick={() => setSort("cost")}>Most expensive</button>
        </div>
      </div>

      <p className="muted">
        {shown.length} of {rows.length} sessions · {money(shown.reduce((s, r) => s + r.cost, 0))} · last 30 days
      </p>

      {shown.length === 0 ? (
        <div className="empty">
          <h3>No sessions match</h3>
          <p className="muted">Try fewer words, or a different project.</p>
        </div>
      ) : (
        <ul className="srows">{shown.map((s) => <Row key={s.id} s={s} words={words} />)}</ul>
      )}
    </>
  );
}
