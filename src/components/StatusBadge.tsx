import type { SessionState } from "../api";
import { AlertIcon, CheckIcon, MoonIcon, SpinIcon } from "./Icons";

// Status is never color alone: every state has an icon and a label.
const META: Record<SessionState, { label: string; icon: React.ReactNode }> = {
  needs_you: { label: "Needs you", icon: <AlertIcon /> },
  working: { label: "Working", icon: <SpinIcon /> },
  done: { label: "Done", icon: <CheckIcon /> },
  idle: { label: "Idle", icon: <MoonIcon /> },
};

export function StatusBadge({ state }: { state: SessionState }) {
  const m = META[state];
  return (
    <span className={`badge badge-${state}`}>
      {m.icon}
      {m.label}
    </span>
  );
}

export const StatusDot = ({ state }: { state: SessionState }) => <span className={`dot dot-${state}`} aria-label={META[state].label} />;
