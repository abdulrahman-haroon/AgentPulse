import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

// Mirrors stats::Dashboard in src-tauri/src/stats.rs.
export type NameValue = { name: string; cost: number; tokens: number };
export type DayPoint = { date: string; cost: number; tokens: number };
export type SessionState = "working" | "needs_you" | "done" | "idle";
export type Session = {
  id: string;
  title: string;
  project: string;
  state: SessionState;
  message: string;
  started: string;
  last_active: string;
  /** When the current state began; null when hooks aren't connected. */
  state_since: string | null;
  cost: number;
  requests: number;
  model: string;
};
export type Dashboard = {
  today_cost: number;
  today_tokens: number;
  today_requests: number;
  today_sessions: number;
  yesterday_cost_by_now: number;
  burn_rate_per_hour: number;
  cache_saved_today: number;
  top_session_today: NameValue | null;
  models_today: NameValue[];
  daily: DayPoint[];
  projects: NameValue[];
  heatmap: number[][];
  sessions: Session[];
  needs_you: number;
  working: number;
};
export type HookStatus = { installed: string[]; missing: string[] };

/** Polls the dashboard command; skips a tick if the previous call is still running. */
export function useDashboard(intervalMs: number) {
  const [data, setData] = useState<Dashboard | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [updatedAt, setUpdatedAt] = useState<Date | null>(null);
  const busy = useRef(false);

  const load = useCallback(async () => {
    if (busy.current) return;
    busy.current = true;
    try {
      setData(await invoke<Dashboard>("dashboard"));
      setError(null);
      setUpdatedAt(new Date());
    } catch (e) {
      setError(String(e));
    } finally {
      busy.current = false;
    }
  }, []);

  useEffect(() => {
    load();
    const t = setInterval(load, intervalMs);
    return () => clearInterval(t);
  }, [load, intervalMs]);

  return { data, error, updatedAt, reload: load };
}

export const toggleWidget = () => invoke("toggle_widget");
export const hookStatus = () => invoke<HookStatus>("hook_status");
