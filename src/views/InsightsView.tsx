import type { Dashboard } from "../api";
import { change, money, tokens } from "../format";
import { RankedBars } from "../components/Charts";

function Tile({ label, value, note }: { label: string; value: React.ReactNode; note?: React.ReactNode }) {
  return (
    <div className="tile">
      <span className="tile-label">{label}</span>
      <span className="tile-value">{value}</span>
      {note && <span className="tile-note">{note}</span>}
    </div>
  );
}

export function InsightsView({ data }: { data: Dashboard }) {
  const pct = change(data.today_cost, data.yesterday_cost_by_now);
  const perSession = data.today_sessions ? data.today_cost / data.today_sessions : 0;
  const cacheShare = data.today_cost + data.cache_saved_today > 0
    ? (data.cache_saved_today / (data.today_cost + data.cache_saved_today)) * 100
    : 0;

  return (
    <>
      <section className="hero card">
        <span className="tile-label">Estimated cost today</span>
        <span className="hero-value">{money(data.today_cost)}</span>
        <span className="hero-note">
          {pct === null ? "No usage yesterday by this time" : (
            <>
              <span className={pct > 0 ? "up" : "down"}>{pct > 0 ? "▲" : "▼"} {Math.abs(pct).toFixed(0)}%</span>
              {" "}vs yesterday at this time ({money(data.yesterday_cost_by_now)})
            </>
          )}
        </span>
      </section>

      <section className="tiles">
        <Tile label="Burn rate" value={<>{money(data.burn_rate_per_hour)}<small>/h</small></>} note="spent in the last 60 minutes" />
        <Tile
          label="Saved by prompt caching"
          value={money(data.cache_saved_today)}
          note={`${cacheShare.toFixed(0)}% off what today would have cost`}
        />
        <Tile
          label="Most expensive session"
          value={data.top_session_today ? money(data.top_session_today.cost) : "—"}
          note={data.top_session_today?.name ?? "no sessions yet"}
        />
        <Tile label="Average per session" value={money(perSession)} note={`${data.today_sessions} sessions today`} />
        <Tile label="Tokens today" value={tokens(data.today_tokens)} note={`${data.today_requests} API requests`} />
        <Tile
          label="Cost per request"
          value={money(data.today_requests ? data.today_cost / data.today_requests : 0)}
          note="average today"
        />
      </section>

      <section className="card">
        <h2 className="card-title">Models today</h2>
        <RankedBars items={data.models_today.map((m) => ({ ...m, name: m.name.replace(/^claude-/, "") }))} empty="No usage yet today." />
      </section>

      <p className="footnote">
        Costs are what these tokens would cost at Anthropic API prices. Pro and Max plans aren't billed per token.
      </p>
    </>
  );
}
