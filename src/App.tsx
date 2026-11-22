import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

// Mirrors usage::DaySummary in src-tauri/src/usage.rs.
type ModelTotals = {
  model: string;
  requests: number;
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
  cost_usd: number | null;
};
type DaySummary = {
  date: string;
  models: ModelTotals[];
  total_tokens: number;
  total_cost_usd: number;
};

const REFRESH_MS = 30_000;
const tokens = new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 });
const usd = new Intl.NumberFormat(undefined, { style: "currency", currency: "USD" });

export default function App() {
  const [data, setData] = useState<DaySummary | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = () =>
    invoke<DaySummary>("today_usage")
      .then((d) => {
        setData(d);
        setError(null);
      })
      .catch((e) => setError(String(e)));

  useEffect(() => {
    load();
    const timer = setInterval(load, REFRESH_MS);
    return () => clearInterval(timer);
  }, []);

  return (
    <main>
      <header>
        <h1>AgentPulse</h1>
        <button onClick={load} aria-label="Refresh">↻</button>
      </header>

      {error && <p className="error" role="alert">Couldn't read usage: {error}</p>}
      {!data && !error && <p className="muted">Loading…</p>}

      {data && (
        <>
          <section className="tiles" aria-label={`Today, ${data.date}`}>
            <div>
              <span className="label">Tokens today</span>
              <span className="value">{tokens.format(data.total_tokens)}</span>
            </div>
            <div>
              <span className="label">Est. API cost</span>
              <span className="value">{usd.format(data.total_cost_usd)}</span>
            </div>
          </section>

          {data.models.length === 0 ? (
            <p className="muted">No Claude Code activity yet today.</p>
          ) : (
            <table>
              <thead>
                <tr>
                  <th scope="col">Model</th>
                  <th scope="col">Requests</th>
                  <th scope="col">Tokens</th>
                  <th scope="col">Est. cost</th>
                </tr>
              </thead>
              <tbody>
                {data.models.map((m) => (
                  <tr key={m.model}>
                    <td>{m.model}</td>
                    <td>{m.requests}</td>
                    <td>{tokens.format(m.input + m.output + m.cache_read + m.cache_write)}</td>
                    <td>{m.cost_usd === null ? "—" : usd.format(m.cost_usd)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}

          <p className="note">
            Cost is what these tokens would cost at API prices. Pro/Max subscribers aren't billed per token.
          </p>
        </>
      )}
    </main>
  );
}
