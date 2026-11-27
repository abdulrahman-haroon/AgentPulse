import type { Agent, Provider } from "../api";
import { since } from "../format";
import { AgentsIcon, CheckIcon, SpinIcon } from "./Icons";

const PROVIDER: Record<Provider, string> = { claude: "Claude", codex: "Codex", copilot: "Copilot" };
export const providerName = (p: Provider) => PROVIDER[p] ?? p;

/** Which coding agent a session belongs to. Text label, color is only a hint. */
export const ProviderTag = ({ provider }: { provider: Provider }) => (
  <span className={`ptag ptag-${provider}`}>{providerName(provider)}</span>
);

/** "2 running" chip for a session's sub-agents. */
export function AgentChip({ running, total }: { running: number; total: number }) {
  if (total === 0) return null;
  return (
    <span className={`achip ${running ? "achip-on" : ""}`} title={`${running} sub-agent${running === 1 ? "" : "s"} running, ${total} in the last 8 hours`}>
      <AgentsIcon size={12} />
      {running ? `${running} running` : `${total} done`}
    </span>
  );
}

/** Sub-agents of one session: running ones first, each with what it was asked to do. */
export function AgentList({ agents, total, now, compact = false }: { agents: Agent[]; total: number; now: number; compact?: boolean }) {
  const shown = compact ? agents.filter((a) => a.running) : agents;
  if (shown.length === 0) return null;
  const hidden = total - shown.length;
  return (
    <ul className={`agents ${compact ? "agents-compact" : ""}`} aria-label="Sub-agents">
      {shown.map((a) => (
        <li key={a.id} className={a.running ? "agent-on" : "agent-done"}>
          {a.running ? <SpinIcon size={12} /> : <CheckIcon size={12} />}
          <span className="agent-kind">{a.kind || "agent"}</span>
          <span className="agent-desc" title={a.description}>{a.description || (a.background ? "background task" : "task")}</span>
          <span className="agent-time">
            {a.background && <span className="agent-bg">bg</span>}
            {a.running ? (a.started ? since(a.started, now) : "") : a.last_active ? `${since(a.last_active, now)} ago` : ""}
          </span>
        </li>
      ))}
      {!compact && hidden > 0 && <li className="agent-more">+{hidden} more finished</li>}
    </ul>
  );
}
