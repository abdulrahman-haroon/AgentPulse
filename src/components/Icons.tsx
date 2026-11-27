// Small inline stroke icons (24×24 grid), so the app ships no icon library.
type P = { size?: number };
const svg = (size: number, children: React.ReactNode) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    {children}
  </svg>
);

export const PulseIcon = ({ size = 18 }: P) => svg(size, <path d="M3 12h4l2.5-6 5 12 2.5-6h4" />);
export const SparkIcon = ({ size = 18 }: P) => svg(size, <path d="M12 3l1.9 5.6L19.5 10l-5.6 1.9L12 17.5l-1.9-5.6L4.5 10l5.6-1.4z" />);
export const ChartIcon = ({ size = 18 }: P) => svg(size, <><path d="M4 20V10" /><path d="M10 20V4" /><path d="M16 20v-7" /><path d="M22 20H2" /></>);
export const ListIcon = ({ size = 18 }: P) => svg(size, <><path d="M9 6h11" /><path d="M9 12h11" /><path d="M9 18h11" /><circle cx="4.5" cy="6" r="1" /><circle cx="4.5" cy="12" r="1" /><circle cx="4.5" cy="18" r="1" /></>);
export const SearchIcon = ({ size = 16 }: P) => svg(size, <><circle cx="11" cy="11" r="7" /><path d="M20 20l-3.5-3.5" /></>);
export const PlugIcon = ({ size = 18 }: P) => svg(size, <><path d="M9 7V3" /><path d="M15 7V3" /><path d="M6 7h12v4a6 6 0 0 1-12 0z" /><path d="M12 17v4" /></>);
export const PinIcon = ({ size = 18 }: P) => svg(size, <><rect x="3" y="5" width="18" height="12" rx="3" /><path d="M8 21h8" /></>);
export const CloseIcon = ({ size = 14 }: P) => svg(size, <><path d="M6 6l12 12" /><path d="M18 6L6 18" /></>);
export const CheckIcon = ({ size = 14 }: P) => svg(size, <path d="M5 12.5l4.5 4.5L19 7" />);
export const AlertIcon = ({ size = 14 }: P) => svg(size, <><path d="M12 8v5" /><path d="M12 16.5v.5" /><circle cx="12" cy="12" r="9" /></>);
export const SpinIcon = ({ size = 14 }: P) => svg(size, <path d="M12 3a9 9 0 1 0 9 9" />);
export const MoonIcon = ({ size = 14 }: P) => svg(size, <path d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z" />);
/** Sub-agents: one session branching into helpers. */
export const AgentsIcon = ({ size = 14 }: P) => svg(size, <><circle cx="12" cy="5" r="2.5" /><circle cx="5" cy="19" r="2.5" /><circle cx="19" cy="19" r="2.5" /><path d="M12 7.5v4M12 11.5l-5.5 5M12 11.5l5.5 5" /></>);
export const SlidersIcon = ({ size = 18 }: P) => svg(size, <><path d="M4 6h9M17 6h3M4 12h3M11 12h9M4 18h11M19 18h1" /><circle cx="15" cy="6" r="2" /><circle cx="9" cy="12" r="2" /><circle cx="17" cy="18" r="2" /></>);
export const VolumeIcon = ({ size = 14 }: P) => svg(size, <><path d="M11 5L6 9H3v6h3l5 4z" /><path d="M15.5 8.5a5 5 0 0 1 0 7" /><path d="M18.5 5.5a9 9 0 0 1 0 13" /></>);
export const ExternalIcon = ({ size = 14 }: P) => svg(size, <><path d="M14 4h6v6" /><path d="M20 4l-9 9" /><path d="M19 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1h5" /></>);
export const ChevronIcon = ({ size = 14, up = false }: P & { up?: boolean }) => svg(size, <path d={up ? "M6 15l6-6 6 6" : "M6 9l6 6 6-6"} />);
export const PlayIcon = ({ size = 12 }: P) => svg(size, <path d="M7 5l12 7-12 7z" />);
export const GaugeIcon = ({ size = 14 }: P) => svg(size, <><path d="M4.5 17a8.5 8.5 0 1 1 15 0" /><path d="M12 13l4-4" /></>);
