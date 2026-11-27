import { useLayoutEffect, useRef, useState } from "react";
import type { DayPoint, NameValue, PlanLimit } from "../api";
import { dayLabel, duration, money, moneyTick, tokens, weekdayLabel } from "../format";
import { providerName } from "./Agents";

/** Plan usage an agent reports about itself (Codex logs it), so these are real numbers. */
export function LimitBars({ limits, now }: { limits: PlanLimit[]; now: number }) {
  return (
    <ul className="limits">
      {limits.map((l) => {
        const level = l.used_percent >= 90 ? "limit-high" : l.used_percent >= 70 ? "limit-mid" : "";
        return (
          <li key={`${l.provider}-${l.window}`}>
            <div className="limit-row">
              <span>{providerName(l.provider)} · {l.window}</span>
              <span className="limit-value">{Math.round(l.used_percent)}%</span>
            </div>
            <div className="track" role="meter" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(l.used_percent)} aria-label={`${providerName(l.provider)} ${l.window} limit used`}>
              <div className={`track-fill ${level}`} style={{ width: `${Math.max(1, l.used_percent)}%` }} />
            </div>
            {l.resets_at && <span className="limit-reset">resets in {duration(new Date(l.resets_at).getTime() - now)}</span>}
          </li>
        );
      })}
    </ul>
  );
}

function useWidth<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    if (!ref.current) return;
    const ro = new ResizeObserver(([e]) => setWidth(e.contentRect.width));
    ro.observe(ref.current);
    return () => ro.disconnect();
  }, []);
  return [ref, width] as const;
}

/** Rounded 4px top corners, square at the baseline. */
function barPath(x: number, y: number, w: number, h: number) {
  const r = Math.min(4, w / 2, h);
  return `M${x},${y + h} V${y + r} Q${x},${y} ${x + r},${y} H${x + w - r} Q${x + w},${y} ${x + w},${y + r} V${y + h} Z`;
}

/** "Nice" axis maximum: 1, 2, 2.5 or 5 × 10^n. */
function niceMax(v: number) {
  if (v <= 0) return 1;
  const p = 10 ** Math.floor(Math.log10(v));
  return ([1, 2, 2.5, 5, 10].find((m) => m * p >= v) ?? 10) * p;
}

/** Daily cost columns with a hover tooltip and a screen-reader table. */
export function DailyBars({ days }: { days: DayPoint[] }) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const [hover, setHover] = useState<number | null>(null);
  const H = 200, top = 8, bottom = 24, left = 48;
  const plotW = Math.max(0, width - left);
  const plotH = H - top - bottom;
  const max = niceMax(Math.max(...days.map((d) => d.cost)));
  const slot = plotW / days.length;
  const barW = Math.max(2, slot - 2); // 2px surface gap between bars
  const y = (v: number) => top + plotH - (v / max) * plotH;
  const labelEvery = days.length > 14 ? 5 : 1;
  const h = hover === null ? null : days[hover];

  return (
    <div className="chart" ref={ref} onMouseLeave={() => setHover(null)}>
      {width > 0 && (
        <svg width={width} height={H} role="img" aria-label="Estimated cost per day">
          {[0, 0.5, 1].map((f) => (
            <g key={f}>
              <line className="grid" x1={left} x2={width} y1={y(max * f)} y2={y(max * f)} />
              <text className="tick" x={left - 8} y={y(max * f) + 4} textAnchor="end">{moneyTick(max * f, max)}</text>
            </g>
          ))}
          {days.map((d, i) => {
            const x = left + i * slot + (slot - barW) / 2;
            const bh = Math.max(d.cost > 0 ? 2 : 0, (d.cost / max) * plotH);
            return (
              <g key={d.date}>
                <path className={`bar ${hover !== null && hover !== i ? "bar-dim" : ""}`} d={barPath(x, top + plotH - bh, barW, bh)} />
                {i % labelEvery === (days.length - 1) % labelEvery && (
                  <text className="tick" x={i === days.length - 1 ? x + barW : x + barW / 2} y={H - 6} textAnchor={i === days.length - 1 ? "end" : "middle"}>
                    {days.length > 14 ? dayLabel(d.date) : weekdayLabel(d.date)}
                  </text>
                )}
                {/* hit target: the whole column, wider than the bar */}
                <rect x={left + i * slot} y={top} width={slot} height={plotH} fill="transparent" onMouseEnter={() => setHover(i)} />
              </g>
            );
          })}
          <line className="baseline" x1={left} x2={width} y1={top + plotH} y2={top + plotH} />
        </svg>
      )}
      {h && hover !== null && (
        <div className="tooltip" style={{ left: Math.min(left + hover * slot + slot / 2, width - 90), top: y(h.cost) - 8 }}>
          <strong>{dayLabel(h.date)}</strong>
          <span>{money(h.cost)}</span>
          <span className="muted">{tokens(h.tokens)} tokens</span>
        </div>
      )}
      <table className="sr-only">
        <caption>Estimated cost per day</caption>
        <tbody>
          {days.map((d) => (
            <tr key={d.date}><th scope="row">{d.date}</th><td>{money(d.cost)}</td><td>{d.tokens} tokens</td></tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Ranked horizontal bars (HTML, so labels never collide). One series, one color. */
export function RankedBars({ items, empty }: { items: NameValue[]; empty: string }) {
  if (items.length === 0) return <p className="muted">{empty}</p>;
  const max = Math.max(...items.map((i) => i.cost), 0.0001);
  return (
    <ul className="ranked">
      {items.map((i) => (
        <li key={i.name} title={`${i.name}: ${money(i.cost)} · ${tokens(i.tokens)} tokens`}>
          <div className="ranked-row">
            <span className="ranked-name">{i.name}</span>
            <span className="ranked-value">{money(i.cost)}</span>
          </div>
          <div className="track">
            <div className="track-fill" style={{ width: `${Math.max(1, (i.cost / max) * 100)}%` }} />
          </div>
        </li>
      ))}
    </ul>
  );
}

const DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/** Requests by weekday × hour. One hue; intensity = volume; empty cells recede. */
export function Heatmap({ grid }: { grid: number[][] }) {
  const [hover, setHover] = useState<{ d: number; h: number } | null>(null);
  const max = Math.max(1, ...grid.flat());
  const hv = hover ? grid[hover.d][hover.h] : 0;
  return (
    <div className="heatmap-wrap" onMouseLeave={() => setHover(null)}>
      <div className="heatmap" role="img" aria-label="Requests by weekday and hour, last 30 days">
        {grid.map((row, d) => (
          <div className="heatmap-row" key={d}>
            <span className="tick heatmap-day">{DAYS[d]}</span>
            {row.map((v, h) => (
              <span
                key={h}
                className={`cell ${v === 0 ? "cell-empty" : ""}`}
                style={v ? { opacity: 0.25 + 0.75 * (v / max) } : undefined}
                onMouseEnter={() => setHover({ d, h })}
              />
            ))}
          </div>
        ))}
        <div className="heatmap-row">
          <span className="heatmap-day" />
          {Array.from({ length: 24 }, (_, h) => (
            <span key={h} className="tick heatmap-hour">{h % 6 === 0 ? `${h}:00` : ""}</span>
          ))}
        </div>
      </div>
      <p className="heatmap-readout" aria-live="polite">
        {hover ? `${DAYS[hover.d]} ${hover.h}:00–${hover.h + 1}:00 · ${hv} request${hv === 1 ? "" : "s"}` : "Hover a cell for details"}
      </p>
    </div>
  );
}
