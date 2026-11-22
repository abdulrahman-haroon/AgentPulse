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
