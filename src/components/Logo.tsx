import { useId } from "react";

/** The AgentPulse mark (same drawing as src-tauri/icons/icon-source.svg, without the blur). */
export function Logo({ size = 28 }: { size?: number }) {
  const id = useId();
  return (
    <svg width={size} height={size} viewBox="64 64 896 896" aria-hidden="true" className="logo">
      <defs>
        <linearGradient id={`${id}t`} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#232850" />
          <stop offset="1" stopColor="#0b0d1c" />
        </linearGradient>
        <linearGradient id={`${id}l`} x1="176" y1="0" x2="848" y2="0" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="#22d3ee" />
          <stop offset="0.5" stopColor="#818cf8" />
          <stop offset="1" stopColor="#e879f9" />
        </linearGradient>
      </defs>
      <rect x="64" y="64" width="896" height="896" rx="224" fill={`url(#${id}t)`} />
      <rect x="67" y="67" width="890" height="890" rx="221" fill="none" stroke="#fff" strokeOpacity="0.1" strokeWidth="6" />
      <path d="M176 560 H352 L432 288 L592 768 L672 560 H800" fill="none" stroke={`url(#${id}l)`} strokeWidth="92" strokeLinecap="round" strokeLinejoin="round" />
      <circle cx="824" cy="560" r="64" fill="#fdf4ff" />
    </svg>
  );
}
