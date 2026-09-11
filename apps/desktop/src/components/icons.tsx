import type { SVGProps } from "react";

/**
 * Minimal inline icon set (stroke-based, inherits currentColor).
 * Avoids pulling an icon dependency for the P0 shell.
 */
type IconProps = SVGProps<SVGSVGElement>;

const base = (props: IconProps) => ({
  width: 22,
  height: 22,
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.8,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
  ...props,
});

export const HomeIcon = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M3 10.5 12 3l9 7.5" />
    <path d="M5 9.5V21h14V9.5" />
  </svg>
);

export const SearchIcon = (p: IconProps) => (
  <svg {...base(p)}>
    <circle cx="11" cy="11" r="7" />
    <path d="m21 21-4.3-4.3" />
  </svg>
);

export const LibraryIcon = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M4 4v16" />
    <path d="M9 4v16" />
    <path d="m14 5 5 14" />
  </svg>
);

export const SettingsIcon = (p: IconProps) => (
  <svg {...base(p)}>
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.6 1.6 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.6 1.6 0 0 0-2.7 1.1V21a2 2 0 1 1-4 0v-.1A1.6 1.6 0 0 0 7 19.4a1.6 1.6 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1A1.6 1.6 0 0 0 2.6 14H2.5a2 2 0 1 1 0-4h.1A1.6 1.6 0 0 0 4.6 7a1.6 1.6 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1A1.6 1.6 0 0 0 9 2.6V2.5a2 2 0 1 1 4 0v.1a1.6 1.6 0 0 0 2.7 1.1l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.6 1.6 0 0 0-.3 1.8V9a1.6 1.6 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.6 1.6 0 0 0-1.5 1Z" />
  </svg>
);

export const PlayIcon = (p: IconProps) => (
  <svg {...base(p)} fill="currentColor" stroke="none">
    <path d="M8 5.5v13a1 1 0 0 0 1.5.86l11-6.5a1 1 0 0 0 0-1.72l-11-6.5A1 1 0 0 0 8 5.5Z" />
  </svg>
);

export const PauseIcon = (p: IconProps) => (
  <svg {...base(p)} fill="currentColor" stroke="none">
    <rect x="6" y="5" width="4" height="14" rx="1.2" />
    <rect x="14" y="5" width="4" height="14" rx="1.2" />
  </svg>
);

export const PrevIcon = (p: IconProps) => (
  <svg {...base(p)} fill="currentColor" stroke="none">
    <rect x="5" y="5" width="2.5" height="14" rx="1" />
    <path d="M20 5.6v12.8a1 1 0 0 1-1.5.86l-9-6.4a1 1 0 0 1 0-1.72l9-6.4A1 1 0 0 1 20 5.6Z" />
  </svg>
);

export const NextIcon = (p: IconProps) => (
  <svg {...base(p)} fill="currentColor" stroke="none">
    <rect x="16.5" y="5" width="2.5" height="14" rx="1" />
    <path d="M4 5.6v12.8a1 1 0 0 0 1.5.86l9-6.4a1 1 0 0 0 0-1.72l-9-6.4A1 1 0 0 0 4 5.6Z" />
  </svg>
);

export const VolumeIcon = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M4 9v6h4l5 4V5L8 9H4Z" />
    <path d="M16 8.5a4 4 0 0 1 0 7" />
    <path d="M18.5 6a7 7 0 0 1 0 12" />
  </svg>
);

export const PlusIcon = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M12 5v14M5 12h14" />
  </svg>
);

export const ClockIcon = (p: IconProps) => (
  <svg {...base(p)}>
    <circle cx="12" cy="12" r="8.5" />
    <path d="M12 7.5V12l3 2" />
  </svg>
);

export const TrashIcon = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M4 7h16" />
    <path d="M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2" />
    <path d="M6 7l1 13a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-13" />
    <path d="M10 11v6M14 11v6" />
  </svg>
);

export const RepeatIcon = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M17 2l4 4-4 4" />
    <path d="M3 11V9a4 4 0 0 1 4-4h14" />
    <path d="M7 22l-4-4 4-4" />
    <path d="M21 13v2a4 4 0 0 1-4 4H3" />
  </svg>
);

export const ShuffleIcon = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M16 3h5v5" />
    <path d="M4 20 21 3" />
    <path d="M21 16v5h-5" />
    <path d="M15 15l6 6" />
    <path d="M4 4l5 5" />
  </svg>
);

export const MoreIcon = (p: IconProps) => (
  <svg {...base(p)}>
    <circle cx="5" cy="12" r="1.4" fill="currentColor" stroke="none" />
    <circle cx="12" cy="12" r="1.4" fill="currentColor" stroke="none" />
    <circle cx="19" cy="12" r="1.4" fill="currentColor" stroke="none" />
  </svg>
);

export const DragIcon = (p: IconProps) => (
  <svg {...base(p)}>
    <circle cx="9" cy="6" r="1.2" fill="currentColor" stroke="none" />
    <circle cx="15" cy="6" r="1.2" fill="currentColor" stroke="none" />
    <circle cx="9" cy="12" r="1.2" fill="currentColor" stroke="none" />
    <circle cx="15" cy="12" r="1.2" fill="currentColor" stroke="none" />
    <circle cx="9" cy="18" r="1.2" fill="currentColor" stroke="none" />
    <circle cx="15" cy="18" r="1.2" fill="currentColor" stroke="none" />
  </svg>
);

export const CheckIcon = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M5 12.5 10 17l9-10" />
  </svg>
);

/** "Other sources" — a signal/source switcher glyph (two overlapping nodes). */
export const SourceIcon = (p: IconProps) => (
  <svg {...base(p)}>
    <circle cx="6.5" cy="12" r="2.5" />
    <circle cx="17.5" cy="7" r="2.2" />
    <circle cx="17.5" cy="17" r="2.2" />
    <path d="M8.6 10.8 15.4 7.9M8.6 13.2l6.8 2.9" />
  </svg>
);

/** "Clear input" — small × used inside the search box. Sized 14 by
 *  default so it can sit comfortably next to the 18px search icon. */
export const CloseIcon = (p: IconProps) => (
  <svg {...base({ ...p, width: p.width ?? 14, height: p.height ?? 14 })}>
    <path d="M6 6l12 12M18 6 6 18" />
  </svg>
);

/** "Play queue" — three list lines plus a play triangle (up-next glyph). */
export const QueueIcon = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M4 7h11" />
    <path d="M4 12h11" />
    <path d="M4 17h7" />
    <path d="M17 14l4 2.5-4 2.5V14z" fill="currentColor" stroke="none" />
  </svg>
);

/** "Back" — a left-pointing arrow. Used by the playlist-detail (and any
 *  future detail) page's top bar. */
export const ArrowLeftIcon = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M15 6l-6 6 6 6" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);

/** Three bouncing bars used to indicate the currently-playing row in a
 *  list (mirrors how Spotify / Last.fm highlight the active song). Lives
 *  next to the icons since it's the same "status glyph" concern. */
export function EqualizerBars({ className = "" }: { className?: string }) {
  return (
    <span
      className={`inline-flex h-3.5 items-end gap-[2px] ${className}`}
      aria-hidden
    >
      <span className="block w-[3px] origin-bottom animate-[eq-bar_1.1s_ease-in-out_infinite] rounded-sm bg-current" />
      <span className="block w-[3px] origin-bottom animate-[eq-bar_1.4s_ease-in-out_0.18s_infinite] rounded-sm bg-current" />
      <span className="block w-[3px] origin-bottom animate-[eq-bar_0.9s_ease-in-out_0.32s_infinite] rounded-sm bg-current" />
    </span>
  );
}
