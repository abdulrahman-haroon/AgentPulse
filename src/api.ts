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
  /** "cli", "claude-vscode", ... */
  entrypoint: string;
};
export type Reply = {
  session_id: string;
  status: "running" | "done" | "error" | "cancelled";
  text: string;
  activity: string[];
  cost_usd: number | null;
  denied: string[];
  error: string;
};
export type PermissionMode = "default" | "acceptEdits" | "plan";
export type Approval = {
  request_id: string;
  session_title: string;
  project: string;
  tool_name: string;
  summary: string;
  detail: string;
  expires_at: string;
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
  replies: Reply[];
  approvals: Approval[];
};
export type HookStatus = {
  installed: string[];
  missing: string[];
  settings_path: string;
  /** Present right after connect/disconnect when settings.json was backed up. */
  backup_path: string | null;
};

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
export const openSession = (id: string) => invoke("open_session", { id });
export const sendReply = (id: string, message: string, mode: PermissionMode) => invoke("send_reply", { id, message, mode });
export const cancelReply = (id: string) => invoke("cancel_reply", { id });
export const dismissReply = (id: string) => invoke("dismiss_reply", { id });
export const hookStatus = () => invoke<HookStatus>("hook_status");
export const connectHooks = () => invoke<HookStatus>("connect_hooks");
export const disconnectHooks = () => invoke<HookStatus>("disconnect_hooks");
export type UpdateInfo = { current: string; available: string | null; notes: string | null };
export const appVersion = () => invoke<string>("app_version");
export const checkUpdate = () => invoke<UpdateInfo>("check_update");
export const installUpdate = () => invoke("install_update");

const AUTO_CHECK_KEY = "agentpulse.autoUpdateCheck";
/** Per-machine preference; defaults to on. Storage can be unavailable, so never throw. */
export function autoCheckEnabled() {
  try {
    return localStorage.getItem(AUTO_CHECK_KEY) !== "off";
  } catch {
    return true;
  }
}
export function setAutoCheck(on: boolean) {
  try {
    localStorage.setItem(AUTO_CHECK_KEY, on ? "on" : "off");
  } catch {
    /* preference just won't persist */
  }
}

// Mirrors settings::Settings in src-tauri/src/settings.rs.
export type PushSettings = {
  enabled: boolean;
  server: string;
  topic: string;
  on_done: boolean;
  on_needs_you: boolean;
  include_details: boolean;
};
export type ApprovalSettings = { enabled: boolean; wait_seconds: number };
export type Settings = { push: PushSettings; approvals: ApprovalSettings };
export const decidePermission = (requestId: string, allow: boolean, reason: string) =>
  invoke("decide_permission", { requestId, allow, reason });
export const getSettings = () => invoke<Settings>("get_settings");
export const saveSettings = (settings: Settings) => invoke<Settings>("save_settings", { settings });
export const testPush = (push: PushSettings) => invoke("test_push", { push });

/** Unguessable ntfy topic from the OS CSPRNG (on ntfy.sh the topic name is the only access control). */
export function randomTopic() {
  const chars = "ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
  const bytes = crypto.getRandomValues(new Uint8Array(24));
  // 256 % 57 bias is negligible for a 24-char topic (~140 bits).
  return "agentpulse-" + Array.from(bytes, (b) => chars[b % chars.length]).join("");
}

export const getAutostart = () => invoke<boolean>("get_autostart");
export const setAutostart = (enabled: boolean) => invoke<boolean>("set_autostart", { enabled });
