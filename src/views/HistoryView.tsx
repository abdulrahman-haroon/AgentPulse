import { useState } from "react";
import type { Dashboard } from "../api";
import { money, tokens } from "../format";
import { DailyBars, Heatmap, RankedBars } from "../components/Charts";

const RANGES = [7, 30] as const;

export function HistoryView({ data }: { data: Dashboard }) {
  const [range, setRange] = useState<(typeof RANGES)[number]>(30);
  const days = data.daily.slice(-range);
  const total = days.reduce((s, d) => s + d.cost, 0);
  const totalTokens = days.reduce((s, d) => s + d.tokens, 0);
  const active = days.filter((d) => d.cost > 0).length;

  return (
    <>
      <div className="filters" role="group" aria-label="Time range">
        {RANGES.map((r) => (
          <button key={r} className={`seg ${range === r ? "seg-on" : ""}`} aria-pressed={range === r} onClick={() => setRange(r)}>
            Last {r} days
          </button>
        ))}
      </div>

      <section className="card">
        <div className="card-head">
          <h2 className="card-title">Estimated cost per day</h2>
          <span className="muted">
            {money(total)} · {tokens(totalTokens)} tokens · {active} active days · {money(active ? total / active : 0)}/active day
          </span>
        </div>
        <DailyBars days={days} />
      </section>

      <div className="two-col">
        <section className="card">
          <h2 className="card-title">Top projects · 30 days</h2>
          <RankedBars items={data.projects} empty="No project activity yet." />
        </section>
        <section className="card">
          <h2 className="card-title">When you work · 30 days</h2>
          <Heatmap grid={data.heatmap} />
        </section>
      </div>
    </>
  );
}
