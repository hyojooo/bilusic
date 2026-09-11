import { useEffect, useRef, useState } from "react";

interface ArtistTileProps {
  name: string;
  cover?: string;
  onClick?: () => void;
  /** Optional small uppercase label rendered under the name (e.g. "Artist").
   *  Adds information density without enlarging the tile. */
  subtitle?: string;
  /** Optional rounded-corner radius class; defaults to full circle. */
  rounded?: string;
  className?: string;
}

/** Stable 0..1 hash from a string — used to derive a per-artist hue so the
 *  editorial fallback tiles feel like a wall of distinct magazine covers
 *  rather than 8 copies of the same grey circle. */
function hash01(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return ((h >>> 0) % 100000) / 100000;
}

/**
 * A circular artist tile.
 *  • Real cover photo when available (home feed's `feed.artists[*].cover` is a
 *    `y.gtimg.cn` URL when the live QQ Music source is reachable).
 *  • When the cover is missing or fails to load, falls back to an editorial
 *    "magazine cover" tile: per-artist OKLCH hue + serif glyph + paper grain +
 *    a thin decorative ring on hover.
 *
 *  Used by the home "歌手" chart and the search-page "热门歌手" wall.
 */
export default function ArtistTile({
  name,
  cover,
  onClick,
  subtitle,
  rounded = "rounded-full",
  className = "",
}: ArtistTileProps) {
  const [errored, setErrored] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const imgRef = useRef<HTMLImageElement>(null);
  const showImg = !!cover && !errored;
  const initial = name.trim().charAt(0) || "?";

  // A real cover already in the browser cache (tab switch-back) is `complete`
  // on mount — reveal it instantly so it doesn't fade up from blank.
  useEffect(() => {
    const el = imgRef.current;
    if (el && el.complete && el.naturalWidth > 0) {
      setLoaded(true);
    }
  }, [cover]);

  // Per-artist editorial palette (only used by the fallback).
  const hue = hash01(name) * 360; // 0..360
  // Each tile gets its own gradient stop hue (offset for variety).
  const hue2 = (hue + 28) % 360;
  // A subtle paper-grain overlay via inline SVG noise — keeps weight off
  // the network and the bundle, gives the fallback a printed feel.
  const grain =
    "url(\"data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='160' height='160'><filter id='n'><feTurbulence type='fractalNoise' baseFrequency='0.9' numOctaves='2' stitchTiles='stitch'/><feColorMatrix values='0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 0.55 0'/></filter><rect width='100%25' height='100%25' filter='url(%23n)'/></svg>\")";

  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={name}
      className={`group flex w-full flex-col items-center gap-1.5 text-center ${className}`}
    >
      <div
        className={`relative aspect-square w-full overflow-hidden ${rounded} ring-1 ring-black/5 transition-all duration-500 ease-[cubic-bezier(0.16,1,0.3,1)] group-hover:-translate-y-1 group-hover:shadow-float group-focus-visible:outline-none group-focus-visible:ring-2 group-focus-visible:ring-accent-500 dark:ring-white/10`}
        style={
          showImg
            ? undefined
            : {
                background: `radial-gradient(120% 95% at 30% 22%, oklch(0.32 0.13 ${hue}) 0%, oklch(0.22 0.09 ${hue2}) 60%, oklch(0.16 0.05 ${hue2}) 100%)`,
              }
        }
      >
        {showImg ? (
          <img
            ref={imgRef}
            src={cover}
            alt={name}
            loading="lazy"
            decoding="async"
            onLoad={() => setLoaded(true)}
            onError={() => setErrored(true)}
            className={`h-full w-full object-cover transition-opacity duration-500 ease-out ${
              loaded ? 'opacity-100' : 'opacity-0'
            }`}
          />
        ) : (
          <>
            {/* Paper grain — keeps the editorial feel without being sterile. */}
            <span
              aria-hidden
              className="pointer-events-none absolute inset-0 mix-blend-overlay opacity-60"
              style={{ backgroundImage: grain }}
            />
            {/* Top "masthead" bar — like a magazine cover's coloured header. */}
            <span
              aria-hidden
              className="absolute inset-x-4 top-3 h-px"
              style={{ background: `oklch(0.86 0.04 ${hue})` }}
            />
            {/* Decorative outer ring — fades in on hover. */}
            <span
              aria-hidden
              className="absolute inset-1 rounded-full border opacity-0 transition-opacity duration-500 ease-out group-hover:opacity-100"
              style={{ borderColor: `oklch(0.78 0.08 ${hue} / 0.55)` }}
            />
            {/* Big serif glyph — the editorial anchor. */}
            <span
              aria-hidden
              className="absolute inset-0 flex items-center justify-center font-bold leading-none tracking-tight"
              style={{
                fontFamily:
                  "'Songti SC', 'STSong', 'SimSun', 'Source Han Serif SC', 'Noto Serif CJK SC', Georgia, 'Times New Roman', serif",
                fontSize: "clamp(2.5rem, 5.5vw, 3.75rem)",
                color: `oklch(0.94 0.02 ${hue})`,
                textShadow: `0 1px 0 oklch(0.18 0.06 ${hue2} / 0.6), 0 6px 22px oklch(0 0 0 / 0.45)`,
              }}
            >
              {initial}
            </span>
            {/* Bottom corner dot — small editorial punctuation. */}
            <span
              aria-hidden
              className="absolute bottom-3 right-4 text-[10px] font-medium uppercase tracking-[0.22em]"
              style={{ color: `oklch(0.78 0.06 ${hue} / 0.85)` }}
            >
              №
            </span>
          </>
        )}
      </div>
      <span className="w-full min-w-0 truncate text-[13px] font-medium text-fg transition-colors group-hover:text-accent-500">
        {name}
      </span>
      {subtitle && (
        <span className="w-full min-w-0 truncate text-[10px] uppercase tracking-[0.16em] text-muted/70">
          {subtitle}
        </span>
      )}
    </button>
  );
}