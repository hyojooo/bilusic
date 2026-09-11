/**
 * Theme engine for Bilusic.
 *
 * Two independent dimensions:
 *  - mode:  "light" | "dark" | "system"   -> toggles [data-theme] on <html>
 *  - accent: one of 12 Tailwind-style hues -> rewrites --c-accent-50..900
 *
 * All color tokens flow through CSS variables (see styles/index.css), so a
 * single runtime write re-skins the entire UI instantly, exactly like Spotube.
 */

export type ThemeMode = "light" | "dark" | "system";

export interface AccentMeta {
  /** machine id, e.g. "slate" */
  id: string;
  /** human label (zh) */
  label: string;
  /** representative swatch (accent-500) for the settings grid */
  swatch: string;
  /** 50..900 hex scale */
  scale: Record<string, string>;
}

const hexToTriplet = (hex: string): string => {
  const h = hex.replace("#", "");
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return `${r} ${g} ${b}`;
};

const SHADES = ["50", "100", "200", "300", "400", "500", "600", "700", "800", "900"];

/** Build a 50..900 scale object from an ordered list of 10 hex strings. */
const scale = (list: string): Record<string, string> => {
  const vals = list.trim().split(/\s+/);
  const out: Record<string, string> = {};
  SHADES.forEach((k, i) => (out[k] = "#" + vals[i]));
  return out;
};

/** The 12 primary-color options, mirroring Spotube's picker. */
export const ACCENTS: AccentMeta[] = [
  { id: "slate",   label: "石墨", swatch: "#64748b", scale: scale("f8fafc f1f5f9 e2e8f0 cbd5e1 94a3b8 64748b 475569 334155 1e293b 0f172a") },
  { id: "gray",    label: "灰",   swatch: "#6b7280", scale: scale("f9fafb f3f4f6 e5e7eb d1d5db 9ca3af 6b7280 4b5563 374151 1f2937 111827") },
  { id: "zinc",    label: "锌",   swatch: "#71717a", scale: scale("fafafa f4f4f5 e4e4e7 d4d4d8 a1a1aa 71717a 52525b 3f3f46 27272a 18181b") },
  { id: "neutral", label: "中性", swatch: "#737373", scale: scale("fafafa f5f5f5 e5e5e5 d4d4d4 a3a3a3 737373 525252 404040 262626 171717") },
  { id: "stone",   label: "石",   swatch: "#78716c", scale: scale("fafaf9 f5f5f4 e7e5e4 d6d3d1 a8a29e 78716c 57534e 44403c 292524 1c1917") },
  { id: "red",     label: "红",   swatch: "#ef4444", scale: scale("fef2f2 fee2e2 fecaca fca5a5 f87171 ef4444 dc2626 b91c1c 991b1b 7f1d1d") },
  { id: "orange",  label: "橙",   swatch: "#f97316", scale: scale("fff7ed ffedd5 fed7aa fdba74 fb923c f97316 ea580c c2410c 9a3412 7c2d12") },
  { id: "yellow",  label: "黄",   swatch: "#eab308", scale: scale("fefce8 fef9c3 fef08a fde047 facc15 eab308 ca8a04 a16207 854d0e 713f12") },
  { id: "green",   label: "绿",   swatch: "#22c55e", scale: scale("f0fdf4 dcfce7 bbf7d0 86efac 4ade80 22c55e 16a34a 15803d 166534 14532d") },
  { id: "blue",    label: "蓝",   swatch: "#3b82f6", scale: scale("eff6ff dbeafe bfdbfe 93c5fd 60a5fa 3b82f6 2563eb 1d4ed8 1e40af 1e3a8a") },
  { id: "violet",  label: "紫",   swatch: "#8b5cf6", scale: scale("f5f3ff ede9fe ddd6fe c4b5fd a78bfa 8b5cf6 7c3aed 6d28d9 5b21b6 4c1d95") },
  { id: "rose",    label: "玫瑰", swatch: "#f43f5e", scale: scale("fff1f2 ffe4e6 fecdd3 fda4af fb7185 f43f5e e11d48 be123c 9f1239 881337") },
];

export const DEFAULT_ACCENT = "slate";

export const getAccent = (id: string): AccentMeta =>
  ACCENTS.find((a) => a.id === id) ?? ACCENTS[0];

/** Write the chosen accent scale into CSS variables on <html>. */
export function applyAccent(id: string): void {
  const meta = getAccent(id);
  const root = document.documentElement;
  Object.entries(meta.scale).forEach(([shade, hex]) => {
    root.style.setProperty(`--c-accent-${shade}`, hexToTriplet(hex));
  });
}

/** Resolve "system" to a concrete mode and set [data-theme]. */
export function applyMode(mode: ThemeMode): void {
  const dark =
    mode === "dark" ||
    (mode === "system" &&
      window.matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.setAttribute("data-theme", dark ? "dark" : "light");
}

/** Convenience: apply both dimensions at once. */
export function applyTheme(mode: ThemeMode, accent: string): void {
  applyMode(mode);
  applyAccent(accent);
}
