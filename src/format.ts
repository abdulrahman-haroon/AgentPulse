const compact = new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 });
const usd2 = new Intl.NumberFormat(undefined, { style: "currency", currency: "USD", minimumFractionDigits: 2, maximumFractionDigits: 2 });
const usd0 = new Intl.NumberFormat(undefined, { style: "currency", currency: "USD", maximumFractionDigits: 0 });

export const tokens = (n: number) => compact.format(n);
export const money = (n: number) => (Math.abs(n) >= 1000 ? usd0.format(n) : usd2.format(n));
/** Axis ticks share one format, chosen by the axis maximum: whole dollars once it reaches $10. */
export const moneyTick = (n: number, axisMax: number) => (axisMax >= 10 ? usd0.format(n) : usd2.format(n));

/** "45s", "12m", "2h 5m", "3d" */
export function duration(ms: number) {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return m % 60 ? `${h}h ${m % 60}m` : `${h}h`;
  return `${Math.floor(h / 24)}d`;
}

export const since = (iso: string, now = Date.now()) => duration(now - new Date(iso).getTime());

/** Percent change, or null when there's no baseline. */
export function change(now: number, before: number) {
  if (before <= 0.005) return null;
  return ((now - before) / before) * 100;
}

const WEEKDAY = new Intl.DateTimeFormat(undefined, { weekday: "short" });
const DAY = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });
// "2026-11-22" parsed as local midnight, not UTC.
export const dayLabel = (date: string) => DAY.format(new Date(`${date}T00:00:00`));
export const weekdayLabel = (date: string) => WEEKDAY.format(new Date(`${date}T00:00:00`));
