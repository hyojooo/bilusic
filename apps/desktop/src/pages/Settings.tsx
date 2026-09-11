import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import i18n from "@/i18n";
import { useSettings, type LayoutType, type Locale } from "@/stores/settings";
import { usePlayer } from "@/stores/player";
import { ACCENTS, type ThemeMode } from "@/lib/theme";
import { Select, type SelectOption } from "@/components/Select";
import {
  cachePrune,
  cacheReset,
  historyClear,
  cacheRefresh,
  cacheOptimize,
  cacheStats,
  extractErrorMessage,
  pluginList,
  pluginToggle,
  pluginReorder,
  pluginUninstall,
  pluginImport,
  type PluginInfo,
  type PluginListItem,
  type CacheStats,
  type CachePruneResult,
} from "@/lib/bili";
import { clearLocalCache } from "@/lib/homeCache";
import { open } from "@tauri-apps/plugin-dialog";

const MODES: { id: ThemeMode; key: string }[] = [
  { id: "system", key: "settings.modeSystem" },
  { id: "light", key: "settings.modeLight" },
  { id: "dark", key: "settings.modeDark" },
];

const LAYOUTS: { id: LayoutType; key: string }[] = [
  { id: "adaptive", key: "settings.layoutAdaptive" },
  { id: "compact", key: "settings.layoutCompact" },
  { id: "expanded", key: "settings.layoutExpanded" },
];

const LOCALES: { id: Locale; label: string }[] = [
  { id: "zh-CN", label: "简体中文" },
  { id: "en-US", label: "English" },
];

function Section({
  title,
  desc,
  children,
}: {
  title: string;
  desc?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="panel p-5">
      <h3 className="text-sm font-semibold text-fg">{title}</h3>
      {desc && <p className="mt-1 text-xs text-muted">{desc}</p>}
      <div className="mt-4">{children}</div>
    </section>
  );
}

function PluginCard({
  info,
  active,
  onSelect,
  role,
}: {
  info: PluginInfo;
  active: boolean;
  onSelect: () => void;
  role: "metadata" | "engine";
}) {
  return (
    <button
      onClick={onSelect}
      className={`w-full rounded-xl border p-3 text-left transition-colors ${
        active
          ? "border-accent-500 bg-accent-500/10"
          : "border-line bg-surface-2 hover:border-accent-500/50"
      }`}
    >
      <div className="flex items-center justify-between">
        <p className="text-sm font-semibold text-fg">{info.name}</p>
        <span className="text-[10px] uppercase tracking-wider text-muted">
          v{info.version}
        </span>
      </div>
      <p className="mt-1 text-[11px] text-muted">{pluginDescription(info)}</p>
      <p className="mt-2 text-[10px] text-muted">
        {i18n.t("settings.pluginRole", {
          role: role === "metadata" ? i18n.t("settings.roleMetadata") : i18n.t("settings.roleEngine"),
          id: info.id,
        })}
      </p>
    </button>
  );
}

function Spinner({ className = "" }: { className?: string }) {
  return (
    <svg
      className={`animate-spin ${className}`}
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeOpacity="0.3" strokeWidth="3" />
      <path
        d="M21 12a9 9 0 0 0-9-9"
        stroke="currentColor"
        strokeWidth="3"
        strokeLinecap="round"
      />
    </svg>
  );
}

const CAP_DEFS: { bit: number; key: string }[] = [
  { bit: 1, key: "settings.capSearch" },
  { bit: 2, key: "settings.capGet" },
  { bit: 4, key: "settings.capLyrics" },
  { bit: 8, key: "settings.capHome" },
  { bit: 16, key: "settings.capById" },
  { bit: 32, key: "settings.capByQuery" },
  { bit: 64, key: "settings.capNeedsAuth" },
];

function capLabels(bits: number): string[] {
  return CAP_DEFS.filter((d) => (bits & d.bit) !== 0).map((d) => i18n.t(d.key));
}

function PluginManageRow({
  item,
  confirmId,
  onToggle,
  onMove,
  onAskUninstall,
  onConfirmUninstall,
  onCancelUninstall,
  busy,
}: {
  item: PluginListItem;
  confirmId: string | null;
  onToggle: (enabled: boolean) => void;
  onMove: (dir: -1 | 1) => void;
  onAskUninstall: () => void;
  onConfirmUninstall: () => void;
  onCancelUninstall: () => void;
  busy: boolean;
}) {
  const key = `${item.kind}:${item.id}:${item.stub ? "stub" : "real"}`;
  const kindLabel =
    item.kind === "metadata"
      ? i18n.t("settings.kindMetadata")
      : item.kind === "engine"
        ? i18n.t("settings.kindEngine")
        : i18n.t("settings.kindLyrics");
  return (
    <div
      className={`rounded-xl border p-3 ${
        item.enabled ? "border-line bg-surface-2" : "border-line bg-surface-2/50 opacity-70"
      }`}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <p className="truncate text-sm font-semibold text-fg">{item.name}</p>
            <span className="text-[10px] uppercase tracking-wider text-muted">
              v{item.version}
            </span>
            <span className="rounded bg-surface px-1.5 py-0.5 text-[10px] text-muted">
              {kindLabel}
            </span>
            {item.builtin ? (
              <span className="rounded bg-accent-500/15 px-1.5 py-0.5 text-[10px] text-accent-500">
                {i18n.t("settings.badgeOfficial")}
              </span>
            ) : (
              <span className="rounded bg-sky-500/15 px-1.5 py-0.5 text-[10px] text-sky-500">
                {i18n.t("settings.badgeThirdParty")}
              </span>
            )}
            {item.stub && (
              <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] text-amber-500">
                {i18n.t("settings.badgeStub")}
              </span>
            )}
          </div>
          <p className="mt-1 truncate text-[11px] text-muted">{pluginDescription(item)}</p>
        </div>
        <label className="relative inline-flex shrink-0 cursor-pointer items-center">
          <input
            type="checkbox"
            className="peer sr-only"
            checked={item.enabled}
            disabled={busy || item.stub}
            onChange={(e) => onToggle(e.target.checked)}
          />
          <div className="h-5 w-9 rounded-full bg-line transition-colors peer-checked:bg-accent-500 peer-disabled:opacity-50" />
          <div className="absolute left-0.5 top-0.5 h-4 w-4 rounded-full bg-white transition-transform peer-checked:translate-x-4" />
        </label>
      </div>

      <div className="mt-2 flex items-center justify-between gap-3">
        <div className="flex flex-wrap gap-1">
          {capLabels(item.capabilities).map((c) => (
            <span key={c} className="rounded bg-surface px-1.5 py-0.5 text-[10px] text-muted">
              {c}
            </span>
          ))}
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          <button
            onClick={() => onMove(-1)}
            disabled={busy || item.stub}
            className="rounded-md border border-line px-2 py-1 text-[11px] text-muted hover:text-fg disabled:opacity-40"
            title={i18n.t("settings.moveUp")}
            aria-label={i18n.t("settings.moveUp")}
          >
            ↑
          </button>
          <button
            onClick={() => onMove(1)}
            disabled={busy || item.stub}
            className="rounded-md border border-line px-2 py-1 text-[11px] text-muted hover:text-fg disabled:opacity-40"
            title={i18n.t("settings.moveDown")}
            aria-label={i18n.t("settings.moveDown")}
          >
            ↓
          </button>
          {item.removable &&
            (confirmId === key ? (
              <div className="flex gap-1">
                <button
                  onClick={onCancelUninstall}
                  disabled={busy}
                  className="rounded-md border border-line px-2 py-1 text-[11px] text-muted hover:text-fg"
                >
                  {i18n.t("common.cancel")}
                </button>
                <button
                  onClick={onConfirmUninstall}
                  disabled={busy}
                  className="rounded-md bg-red-500 px-2 py-1 text-[11px] font-medium text-white hover:bg-red-600"
                >
                  {busy ? (
                    <>
                      <Spinner className="mr-1 inline" />
                      {i18n.t("settings.uninstalling")}
                    </>
                  ) : (
                    i18n.t("settings.confirmUninstall")
                  )}
                </button>
              </div>
            ) : (
              <button
                onClick={onAskUninstall}
                disabled={busy}
                className="rounded-md border border-red-500/50 px-2 py-1 text-[11px] text-red-500 hover:bg-red-500/10 disabled:opacity-40"
              >
                {i18n.t("settings.uninstall")}
              </button>
            ))}
        </div>
      </div>
    </div>
  );
}

function toPluginInfo(p: PluginListItem): PluginInfo {
  return {
    id: p.id,
    name: p.name,
    version: p.version,
    description: p.description,
    capabilities: p.capabilities,
    kind: p.kind,
  };
}

/**
 * Render a plugin's description through i18n.
 *
 * Keys are namespaced by (id, kind) so plugins that exist in two roles
 * (e.g. `bilibili` doubles as both an audio engine and a metadata source)
 * can carry distinct translations without naming collisions. Unknown
 * combinations fall back to the backend-supplied original Chinese so a
 * freshly-imported third-party plugin never breaks the UI.
 */
// 仅 `bilibili` 是同 id 双角色插件（`metadata:` 与 `engine:` 是两段不同
// description）。其它内置/第三方都是单角色（kind 没有意义），统一按
// `plugin.<id>.description` 单层查，namespace 在 i18n 里也更清爽。
const DUAL_ROLE_IDS: ReadonlySet<string> = new Set(["bilibili"]);

function pluginDescription(plugin: {
  id: string;
  description: string;
  kind?: string;
}): string {
  const useKind = plugin.kind && DUAL_ROLE_IDS.has(plugin.id);
  const key = useKind
    ? `plugin.${plugin.id}.${plugin.kind}.description`
    : `plugin.${plugin.id}.description`;
  return i18n.t(key, { defaultValue: plugin.description });
}

export default function Settings() {
  const { t } = useTranslation();
  const {
    mode,
    accent,
    layout,
    locale,
    metadataSource,
    audioEngine,
    biliCookie,
    setMode,
    setAccent,
    setLayout,
    setLocale,
    setMetadataSource,
    setAudioEngine,
    setBiliCookie,
  } = useSettings();

  // Toast queue — multiple toasts can stack without replacing each other.
  // Each entry auto-dismisses after `TOAST_TTL_MS`. The render layer uses
  // `bottom-20 right-6` (matches PlayerBar's offset convention to clear the
  // 64-px player bar) and `shadow-float` for a glassy elevation.
  type ToastItem = { id: number; text: string; tone: "ok" | "err" };
  const TOAST_TTL_MS = 4500;
  const TOAST_MAX = 5;
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const toastIdRef = useRef(0);
  const [busy, setBusy] = useState<"" | "prune" | "history" | "reset" | "refresh" | "optimize" | "local">("");
  useEffect(() => {
    if (toasts.length === 0) return;
    const oldest = toasts[0];
    const t = setTimeout(() => {
      setToasts((cur) => cur.filter((x) => x.id !== oldest.id));
    }, TOAST_TTL_MS);
    return () => clearTimeout(t);
  }, [toasts]);
  function showToast(text: string, tone: "ok" | "err" = "ok") {
    const id = ++toastIdRef.current;
    setToasts((cur) => {
      const next = [...cur, { id, text, tone }];
      // Keep the newest `TOAST_MAX`; older ones quietly age out so the UI
      // never explodes if many actions fire in rapid succession.
      return next.length > TOAST_MAX ? next.slice(next.length - TOAST_MAX) : next;
    });
  }
  function dismissToast(id: number) {
    setToasts((cur) => cur.filter((x) => x.id !== id));
  }
  const [confirmReset, setConfirmReset] = useState(false);
  const [stats, setStats] = useState<CacheStats | null>(null);

  const [pluginItems, setPluginItems] = useState<PluginListItem[]>([]);
  const [pluginBusy, setPluginBusy] = useState(false);
  const [pluginImporting, setPluginImporting] = useState(false);
  const [confirmUninstall, setConfirmUninstall] = useState<string | null>(null);
  const [pluginTab, setPluginTab] = useState<"metadata" | "engine" | "lyrics">("metadata");

  function fmtBytes(n: number): string {
    if (n <= 0) return "0 B";
    const units = ["B", "KB", "MB", "GB"];
    const i = Math.min(units.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
    return `${(n / Math.pow(1024, i)).toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
  }

  // Derive available metadata sources / audio engines from the plugin management list
  // (single source of truth — toggling a plugin here immediately adds/removes its card above).
  const availableMeta: PluginInfo[] = useMemo(
    () =>
      pluginItems
        .filter((p) => p.kind === "metadata" && p.enabled)
        .map(toPluginInfo),
    [pluginItems],
  );
  const availableEngine: PluginInfo[] = useMemo(
    () =>
      pluginItems
        .filter((p) => p.kind === "engine" && p.enabled)
        .map(toPluginInfo),
    [pluginItems],
  );

  // Plugin management list filtered by the active category tab.
  const pluginItemsForTab: PluginListItem[] = useMemo(
    () => pluginItems.filter((p) => p.kind === pluginTab),
    [pluginItems, pluginTab],
  );

  // On first pluginItems arrival, if the persisted active source/engine is no longer
  // enabled, fall back to the first enabled one (handles: user disabled it in a prior session).
  const initialLinkageDone = useRef(false);
  useEffect(() => {
    if (initialLinkageDone.current) return;
    if (pluginItems.length === 0) return;
    initialLinkageDone.current = true;
    if (metadataSource && !availableMeta.some((p) => p.id === metadataSource)) {
      if (availableMeta.length > 0) setMetadataSource(availableMeta[0].id);
    }
    if (audioEngine && !availableEngine.some((p) => p.id === audioEngine)) {
      if (availableEngine.length > 0) setAudioEngine(availableEngine[0].id);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pluginItems]);

  async function loadStats() {
    try {
      setStats(await cacheStats());
    } catch {
      /* ignore — stats are informational */
    }
  }

  useEffect(() => {
    void loadStats();
  }, []);

  async function loadPlugins() {
    try {
      setPluginItems(await pluginList());
    } catch {
      /* ignore — plugin list is informational */
    }
  }

  useEffect(() => {
    void loadPlugins();
  }, []);

  async function handlePluginToggle(item: PluginListItem, enabled: boolean) {
    setPluginBusy(true);
    try {
      await pluginToggle(item.kind, item.id, enabled);
      setPluginItems((prev) =>
        prev.map((p) =>
          p.kind === item.kind && p.id === item.id ? { ...p, enabled } : p,
        ),
      );
      showToast(
        enabled
          ? t("settings.pluginEnabled", { name: item.name })
          : t("settings.pluginDisabled", { name: item.name }),
      );
      // Linkage: if the toggled-off plugin was the active source/engine, auto-switch
      // to the next enabled one. Exclude the current item from the candidate set
      // (stale pluginItems still marks it enabled before the optimistic update).
      if (!enabled) {
        if (item.kind === "metadata" && metadataSource === item.id) {
          const next = pluginItems.find(
            (p) =>
              p.kind === "metadata" &&
              p.enabled &&
              !(p.kind === item.kind && p.id === item.id),
          );
          if (next) {
            setMetadataSource(next.id);
            showToast(
              t("settings.sourceAutoSwitched", { name: item.name, to: next.name }),
            );
          } else {
            showToast(t("settings.noSourceAvailable"), "err");
          }
        }
        if (item.kind === "engine" && audioEngine === item.id) {
          const next = pluginItems.find(
            (p) =>
              p.kind === "engine" &&
              p.enabled &&
              !(p.kind === item.kind && p.id === item.id),
          );
          if (next) {
            setAudioEngine(next.id);
            showToast(
              t("settings.engineAutoSwitched", { name: item.name, to: next.name }),
            );
          } else {
            showToast(t("settings.noEngineAvailable"), "err");
          }
        }
      }
    } catch (e) {
      showToast(
        t("common.opFailed", { op: t("settings.toggle"), msg: extractErrorMessage(e) }),
        "err",
      );
    } finally {
      setPluginBusy(false);
    }
  }

  async function handlePluginMove(item: PluginListItem, dir: -1 | 1) {
    setPluginBusy(true);
    try {
      // Reorder within the active tab only, then rebuild the full ordered list
      // so the other category's stored order is not disturbed.
      const tabItems = pluginItems.filter((p) => p.kind === pluginTab);
      const idx = tabItems.findIndex((p) => p.kind === item.kind && p.id === item.id);
      const newIdx = idx + dir;
      if (newIdx < 0 || newIdx >= tabItems.length) return;
      const reorderedTab = [...tabItems];
      [reorderedTab[idx], reorderedTab[newIdx]] = [reorderedTab[newIdx], reorderedTab[idx]];
      const reordered = [...pluginItems];
      let ti = 0;
      for (let i = 0; i < reordered.length; i++) {
        if (reordered[i].kind === pluginTab) {
          reordered[i] = reorderedTab[ti++];
        }
      }
      const ordered: [string, string][] = reordered.map((p) => [p.kind, p.id]);
      await pluginReorder(ordered);
      setPluginItems(reordered);
    } catch (e) {
      showToast(
        t("common.opFailed", { op: t("settings.reorder"), msg: extractErrorMessage(e) }),
        "err",
      );
    } finally {
      setPluginBusy(false);
    }
  }

  async function handlePluginUninstall(item: PluginListItem) {
    setPluginBusy(true);
    try {
      await pluginUninstall(item.kind, item.id);
      // Remove ONLY the specific row that was clicked. A same-id built-in +
      // bundled stub pair (e.g. 酷狗音乐 built-in ↔ 酷狗音乐 bundled stub)
      // share the same `kind` + `id`; without the `stub` check the filter
      // would wipe both rows even though the backend only hid the stub.
      setPluginItems((prev) =>
        prev.filter(
          (p) =>
            !(p.kind === item.kind && p.id === item.id && p.stub === item.stub),
        ),
      );
      setConfirmUninstall(null);
      showToast(t("settings.uninstalled", { name: item.name }));
      // Linkage: if the uninstalled plugin was the active source/engine, fall
      // back. Gate on `!item.stub` so hiding a bundled/dev stub (whose real
      // adapter — if any — is a same-id built-in that we did not touch) does
      // not yank the active source away from a still-functional built-in.
      if (
        item.kind === "metadata" &&
        !item.stub &&
        metadataSource === item.id
      ) {
        const next = pluginItems.find(
          (p) =>
            p.kind === "metadata" &&
            p.enabled &&
            !(p.kind === item.kind && p.id === item.id && p.stub === item.stub),
        );
        if (next) {
          setMetadataSource(next.id);
          showToast(
            t("settings.sourceAutoSwitched", { name: item.name, to: next.name }),
          );
        } else {
          showToast(t("settings.noSourceAvailable"), "err");
        }
      }
      if (
        item.kind === "engine" &&
        !item.stub &&
        audioEngine === item.id
      ) {
        const next = pluginItems.find(
          (p) =>
            p.kind === "engine" &&
            p.enabled &&
            !(p.kind === item.kind && p.id === item.id && p.stub === item.stub),
        );
        if (next) {
          setAudioEngine(next.id);
          showToast(
            t("settings.engineAutoSwitched", { name: item.name, to: next.name }),
          );
        } else {
          showToast(t("settings.noEngineAvailable"), "err");
        }
      }
    } catch (e) {
      showToast(
        t("common.opFailed", { op: t("settings.uninstall"), msg: extractErrorMessage(e) }),
        "err",
      );
    } finally {
      setPluginBusy(false);
    }
  }

  /**
   * Import a third-party plugin: native picker → plugin_import(path) → refresh.
   * `asDir = true` picks a plugin folder; otherwise a `plugin.json` file.
   */
  async function handlePluginImport(asDir: boolean) {
    let picked: string | null = null;
    try {
      picked = await open({
        directory: asDir,
        multiple: false,
        title: asDir
          ? t("settings.pluginImportDirTitle")
          : t("settings.pluginImportFileTitle"),
        filters: asDir
          ? undefined
          : [{ name: "plugin.json", extensions: ["json"] }],
      });
    } catch (e) {
      showToast(
        t("common.opFailed", { op: t("settings.pluginImportFile"), msg: extractErrorMessage(e) }),
        "err",
      );
      return;
    }
    if (typeof picked !== "string") return; // cancelled
    setPluginImporting(true);
    try {
      const id = await pluginImport(picked);
      showToast(t("settings.pluginImported", { name: id }));
      await loadPlugins();
    } catch (e) {
      showToast(
        t("common.opFailed", { op: t("settings.pluginImportFile"), msg: extractErrorMessage(e) }),
        "err",
      );
    } finally {
      setPluginImporting(false);
    }
  }

  async function handlePrune() {
    setBusy("prune");
    try {
      const r: CachePruneResult = await cachePrune();
      const parts: string[] = [];
      if (r.removed > 0) parts.push(t("settings.cacheCleaned", { count: r.removed }));
      else parts.push(t("settings.cacheCleanNothing"));
      if (r.freed_bytes > 0)
        parts.push(t("settings.cacheFreed", { space: fmtBytes(r.freed_bytes) }));
      showToast(parts.join(t("common.listSep")));
    } catch (e) {
      showToast(
        t("common.opFailed", { op: t("settings.cleanCache"), msg: extractErrorMessage(e) }),
        "err",
      );
    } finally {
      setBusy("");
      void loadStats();
    }
  }

  async function handleOptimize() {
    setBusy("optimize");
    try {
      const freed = await cacheOptimize();
      showToast(
        freed > 0
          ? t("settings.cacheOptimized", { space: fmtBytes(freed) })
          : t("settings.cacheOptimizedNothing"),
      );
    } catch (e) {
      showToast(
        t("common.opFailed", { op: t("settings.optimize"), msg: extractErrorMessage(e) }),
        "err",
      );
    } finally {
      setBusy("");
      void loadStats();
    }
  }

  async function handleClearHistory() {
    setBusy("history");
    try {
      const n = await historyClear();
      showToast(t("settings.cacheHistoryCleared", { count: n }));
    } catch (e) {
      showToast(
        t("common.opFailed", { op: t("settings.clearHistory"), msg: extractErrorMessage(e) }),
        "err",
      );
    } finally {
      setBusy("");
      void loadStats();
    }
  }

  async function handleRefresh() {
    setBusy("refresh");
    try {
      const n = await cacheRefresh();
      showToast(
        n > 0
          ? t("settings.cacheRefreshed", { count: n })
          : t("settings.cacheRefreshedNothing"),
      );
    } catch (e) {
      showToast(
        t("common.opFailed", { op: t("settings.refresh"), msg: extractErrorMessage(e) }),
        "err",
      );
    } finally {
      setBusy("");
      void loadStats();
    }
  }

  async function handleReset() {
    setBusy("reset");
    try {
      // 1) Wipe frontend-side playback state FIRST. The queue lives in a
      //    zustand persist slice (key = "bilusic-player") and the play mode /
      //    volume preferences would otherwise survive the reload that ends
      //    this handler. `clearQueue()` stops the audio and zeroes the
      //    in-memory queue; the explicit `removeItem` is defensive — partialize
      //    would otherwise write an empty `[]` back to localStorage which is
      //    fine, but we'd rather not leave the play-mode / volume slice either
      //    so the user truly gets a clean slate.
      usePlayer.getState().clearQueue();
      localStorage.removeItem("bilusic-player");
      // 2) Wipe backend SQLite (playlists, history, cache).
      await cacheReset();
      showToast(t("settings.cacheResetDone"));
      setConfirmReset(false);
      window.location.reload();
    } catch (e) {
      showToast(
        t("common.opFailed", { op: t("settings.reset"), msg: extractErrorMessage(e) }),
        "err",
      );
    } finally {
      setBusy("");
    }
  }

  /**
   * Clear frontend-local caches only (home feeds / charts / audio-search
   * candidates in localStorage). This is the lightweight, user-facing escape
   * hatch for "I changed sources / the home layout looks stale" — replaces the
   * old hand-maintained `HOME_SCHEMA_REV` bump. Unlike `handleReset` it does
   * NOT touch the backend SQLite cache and does not wipe settings, so we just
   * wipe local storage and reload to let the home page re-fetch cleanly.
   */
  async function handleClearLocalCache() {
    setBusy("local");
    try {
      clearLocalCache();
      showToast(t("settings.localCacheCleared"));
      // Defer the reload so the toast actually paints before the page
      // tears down. 900ms is enough to render + register the toast in the
      // DOM without making the action feel sluggish.
      window.setTimeout(() => {
        window.location.reload();
      }, 900);
    } catch (e) {
      showToast(
        t("common.opFailed", { op: t("settings.clearLocalCache"), msg: extractErrorMessage(e) }),
        "err",
      );
      setBusy("");
    }
  }

  return (
    <div className="mx-auto max-w-3xl">
      <h2 className="text-xl font-semibold text-fg">{t("settings.title")}</h2>

      <div className="mt-5 flex flex-col gap-5">
        <Section title={t("settings.appearance")} desc={t("settings.appearanceDesc")}>
          <div className="space-y-5">
            <div>
              <p className="text-xs font-medium text-muted">{t("settings.accent")}</p>
              <div className="mt-2 flex flex-wrap gap-2.5">
                {ACCENTS.map((a) => {
                  const active = a.id === accent;
                  return (
                    <button
                      key={a.id}
                      onClick={() => setAccent(a.id)}
                      title={t(`accent.${a.id}`)}
                      className={`flex h-9 w-9 items-center justify-center rounded-full ring-2 ring-offset-2 ring-offset-surface transition-transform hover:scale-105 ${
                        active ? "ring-accent-500" : "ring-transparent"
                      }`}
                      style={{ backgroundColor: a.swatch }}
                      aria-label={t(`accent.${a.id}`)}
                    >
                      {active && (
                        <span className="h-2.5 w-2.5 rounded-full bg-white/90" />
                      )}
                    </button>
                  );
                })}
              </div>
            </div>

            <div>
              <p className="text-xs font-medium text-muted">{t("settings.theme")}</p>
              <div className="mt-2 inline-flex rounded-xl border border-line bg-surface-2 p-1">
                {MODES.map((m) => (
                  <button
                    key={m.id}
                    onClick={() => setMode(m.id)}
                    className={`rounded-lg px-3 py-1.5 text-xs font-medium transition-colors ${
                      mode === m.id
                        ? "bg-surface text-fg shadow-soft"
                        : "text-muted hover:text-fg"
                    }`}
                  >
                    {t(m.key)}
                  </button>
                ))}
              </div>
            </div>
          </div>
        </Section>

        <Section title={t("settings.layoutLang")}>
          <div className="grid grid-cols-2 gap-4">
            <Select
              label={t("settings.layout")}
              ariaLabel={t("settings.layout")}
              value={layout}
              onChange={(v) => setLayout(v as LayoutType)}
              options={LAYOUTS.map(
                (l): SelectOption => ({ value: l.id, label: t(l.key) }),
              )}
            />
            <Select
              label={t("settings.language")}
              ariaLabel={t("settings.language")}
              value={locale}
              onChange={(v) => setLocale(v as Locale)}
              options={LOCALES.map(
                (l): SelectOption => ({ value: l.id, label: l.label }),
              )}
            />
          </div>
        </Section>

        <Section
          title={t("settings.source")}
          desc={t("settings.sourceDesc")}
        >
          <div className="space-y-5">
            <div>
              <p className="text-xs font-medium text-muted">
                {t("settings.metadataSource")}{" "}
                <span className="text-muted/70">{t("settings.metadataSourceHint")}</span>
              </p>
              <div className="mt-2 grid grid-cols-1 gap-2 sm:grid-cols-2">
                {pluginItems.length === 0 ? (
                  <p className="text-xs text-muted">{t("common.loading")}</p>
                ) : availableMeta.length === 0 ? (
                  <p className="text-xs text-muted">{t("settings.noSourceAvailable")}</p>
                ) : (
                  availableMeta.map((p) => (
                    <PluginCard
                      key={p.id}
                      info={p}
                      active={p.id === metadataSource}
                      onSelect={() => setMetadataSource(p.id)}
                      role="metadata"
                    />
                  ))
                )}
              </div>
            </div>

            <div>
              <p className="text-xs font-medium text-muted">
                {t("settings.audioEngine")}{" "}
                <span className="text-muted/70">{t("settings.audioEngineHint")}</span>
              </p>
              <div className="mt-2 grid grid-cols-1 gap-2 sm:grid-cols-2">
                {pluginItems.length === 0 ? (
                  <p className="text-xs text-muted">{t("common.loading")}</p>
                ) : availableEngine.length === 0 ? (
                  <p className="text-xs text-muted">{t("settings.noEngineAvailable")}</p>
                ) : (
                  availableEngine.map((p) => (
                    <PluginCard
                      key={p.id}
                      info={p}
                      active={p.id === audioEngine}
                      onSelect={() => setAudioEngine(p.id)}
                      role="engine"
                    />
                  ))
                )}
              </div>
            </div>
          </div>
        </Section>

        <Section
          title={t("settings.plugins")}
          desc={t("settings.pluginsDesc")}
        >
          {pluginItems.length === 0 ? (
            <p className="text-xs text-muted">{t("common.loading")}</p>
          ) : (
            <>
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="inline-flex rounded-xl border border-line bg-surface-2 p-1">
                  <button
                    onClick={() => setPluginTab("metadata")}
                    className={`rounded-lg px-3 py-1.5 text-xs font-medium transition-colors ${
                      pluginTab === "metadata"
                        ? "bg-surface text-fg shadow-soft"
                        : "text-muted hover:text-fg"
                    }`}
                  >
                    {t("settings.kindMetadata")}
                  </button>
                  <button
                    onClick={() => setPluginTab("engine")}
                    className={`rounded-lg px-3 py-1.5 text-xs font-medium transition-colors ${
                      pluginTab === "engine"
                        ? "bg-surface text-fg shadow-soft"
                        : "text-muted hover:text-fg"
                    }`}
                  >
                    {t("settings.kindEngine")}
                  </button>
                  <button
                    onClick={() => setPluginTab("lyrics")}
                    className={`rounded-lg px-3 py-1.5 text-xs font-medium transition-colors ${
                      pluginTab === "lyrics"
                        ? "bg-surface text-fg shadow-soft"
                        : "text-muted hover:text-fg"
                    }`}
                  >
                    {t("settings.kindLyrics")}
                  </button>
                </div>
                <div className="flex items-center gap-2">
                  <button
                    onClick={() => void handlePluginImport(true)}
                    disabled={pluginImporting}
                    title={t("settings.pluginImportDirTitle")}
                    className="rounded-lg border border-line bg-surface-2 px-3 py-1.5 text-xs font-medium text-fg transition-colors hover:border-accent-400 hover:text-accent-400 disabled:opacity-50"
                  >
                    {pluginImporting ? t("common.loading") : t("settings.pluginImportDir")}
                  </button>
                  <button
                    onClick={() => void handlePluginImport(false)}
                    disabled={pluginImporting}
                    title={t("settings.pluginImportFileTitle")}
                    className="rounded-lg bg-accent-500 px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-accent-600 disabled:opacity-50"
                  >
                    {t("settings.pluginImportFile")}
                  </button>
                </div>
              </div>
              <div className="mt-3 space-y-2">
                {pluginItemsForTab.length === 0 ? (
                  <p className="text-xs text-muted">{t("settings.noPluginsInTab")}</p>
                ) : (
                  pluginItemsForTab.map((item) => (
                    <PluginManageRow
                      key={`${item.kind}:${item.id}:${item.stub ? "stub" : "real"}`}
                      item={item}
                      confirmId={confirmUninstall}
                      busy={pluginBusy}
                      onToggle={(enabled) => void handlePluginToggle(item, enabled)}
                      onMove={(dir) => void handlePluginMove(item, dir)}
                      onAskUninstall={() => setConfirmUninstall(`${item.kind}:${item.id}:${item.stub ? "stub" : "real"}`)}
                      onConfirmUninstall={() => void handlePluginUninstall(item)}
                      onCancelUninstall={() => setConfirmUninstall(null)}
                    />
                  ))
                )}
              </div>
              <details className="mt-4 overflow-hidden rounded-md border border-line">
                <summary className="cursor-pointer select-none px-3 py-2 text-xs font-medium text-base transition-colors hover:bg-fg/5">
                  {t("settings.pluginsNoteTitle")}
                </summary>
                <div className="whitespace-pre-line border-t border-line bg-surface-2/50 px-3 py-2 text-[11px] leading-snug text-muted">
                  {t("settings.pluginsNote")}
                </div>
              </details>
            </>
          )}
        </Section>

        <Section
          title={t("settings.cookie")}
          desc={t("settings.cookieDesc")}
        >
          <textarea
            value={biliCookie}
            onChange={(e) => setBiliCookie(e.target.value)}
            placeholder={t("settings.cookiePlaceholder")}
            className="field h-20 resize-none font-mono text-[11px] leading-relaxed"
          />
        </Section>

        <Section
          title={t("settings.cache")}
          desc={t("settings.cacheDesc")}
        >
          {stats && (
            <div className="mb-3 grid grid-cols-2 gap-2 rounded-lg bg-surface-2 p-3 text-[11px] sm:grid-cols-4">
              <div>
                <p className="text-muted">{t("settings.statRows")}</p>
                <p className="text-sm font-semibold text-fg">{stats.rows}</p>
              </div>
              <div>
                <p className="text-muted">{t("settings.statReferenced")}</p>
                <p className="text-sm font-semibold text-fg">{stats.referenced}</p>
              </div>
              <div>
                <p className="text-muted">{t("settings.statStale")}</p>
                <p className="text-sm font-semibold text-fg">{stats.stale}</p>
              </div>
              <div>
                <p className="text-muted">{t("settings.statOldest")}</p>
                <p className="text-sm font-semibold text-fg">
                  {Math.floor(stats.oldest_age_ms / 86_400_000)} {t("settings.statOldestUnit")}
                </p>
              </div>
            </div>
          )}
          <div className="flex flex-col gap-4">
            {/* 分组 1：本地缓存（前端 localStorage） */}
            <div className="rounded-lg border border-line bg-surface-2/40 p-3">
              <p className="text-[11px] font-semibold uppercase tracking-wide text-muted">
                {t("settings.groupLocal")}
              </p>
              <div className="mt-2.5 flex items-center justify-between gap-4">
                <div>
                  <p className="text-sm font-medium text-fg">{t("settings.clearLocalCache")}</p>
                  <p className="text-[11px] text-muted">
                    {t("settings.clearLocalCacheDesc")}
                  </p>
                </div>
                <button
                  className="shrink-0 rounded-lg border border-line bg-surface-2 px-3 py-1.5 text-xs font-medium text-fg hover:border-accent-500/50"
                  onClick={handleClearLocalCache}
                  disabled={busy !== ""}
                >
                  {busy === "local" ? (<><Spinner className="mr-1.5 inline" />{t("settings.clearing")}</>) : t("settings.clearLocalCache")}
                </button>
              </div>
            </div>

            {/* 分组 2：曲库缓存（后端 SQLite） */}
            <div className="rounded-lg border border-line bg-surface-2/40 p-3">
              <p className="text-[11px] font-semibold uppercase tracking-wide text-muted">
                {t("settings.groupLibrary")}
              </p>
              <p className="mt-1 text-[11px] text-muted">
                {t("settings.libraryCacheDesc")}
              </p>
              <div className="mt-2.5 grid grid-cols-1 gap-2 sm:grid-cols-3">
                <button
                  className="rounded-lg border border-line bg-surface-2 px-3 py-2 text-xs font-medium text-fg transition-colors hover:border-accent-500/50 disabled:opacity-50"
                  onClick={handlePrune}
                  disabled={busy !== ""}
                  title={t("settings.cleanCacheDesc")}
                >
                  {busy === "prune" ? (<><Spinner className="mr-1.5 inline" />{t("settings.cleaning")}</>) : t("settings.cleanNow")}
                </button>
                <button
                  className="rounded-lg border border-line bg-surface-2 px-3 py-2 text-xs font-medium text-fg transition-colors hover:border-accent-500/50 disabled:opacity-50"
                  onClick={handleOptimize}
                  disabled={busy !== ""}
                  title={t("settings.optimizeDesc")}
                >
                  {busy === "optimize" ? (<><Spinner className="mr-1.5 inline" />{t("settings.optimizing")}</>) : t("settings.optimizeNow")}
                </button>
                <button
                  className="rounded-lg border border-line bg-surface-2 px-3 py-2 text-xs font-medium text-fg transition-colors hover:border-accent-500/50 disabled:opacity-50"
                  onClick={handleRefresh}
                  disabled={busy !== ""}
                  title={t("settings.refreshDesc")}
                >
                  {busy === "refresh" ? (<><Spinner className="mr-1.5 inline" />{t("settings.refreshing")}</>) : t("settings.refreshNow")}
                </button>
              </div>
              <div className="mt-2 flex items-center justify-between gap-4 border-t border-line pt-2.5">
                <div>
                  <p className="text-sm font-medium text-fg">{t("settings.clearHistory")}</p>
                  <p className="text-[11px] text-muted">{t("settings.clearHistoryDesc")}</p>
                </div>
                <button
                  className="shrink-0 rounded-lg border border-line bg-surface-2 px-3 py-1.5 text-xs font-medium text-fg hover:border-accent-500/50"
                  onClick={handleClearHistory}
                  disabled={busy !== ""}
                >
                  {busy === "history" ? (<><Spinner className="mr-1.5 inline" />{t("settings.clearing")}</>) : t("settings.clearNow")}
                </button>
              </div>
            </div>

            <div className="flex items-center justify-between gap-4 rounded-xl border border-red-500/40 bg-red-500/5 p-3">
              <div>
                <p className="text-sm font-medium text-fg">{t("settings.reset")}</p>
                <p className="text-[11px] text-muted">
                  {t("settings.resetDesc")}
                </p>
              </div>
              {confirmReset ? (
                <div className="flex gap-2">
                  <button
                    className="rounded-lg border border-line bg-surface-2 px-3 py-1.5 text-xs font-medium text-fg hover:border-accent-500/50"
                    onClick={() => setConfirmReset(false)}
                    disabled={busy !== ""}
                  >
                    {t("common.cancel")}
                  </button>
                  <button
                    className="rounded-lg bg-red-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-red-600"
                    onClick={handleReset}
                    disabled={busy !== ""}
                  >
                    {busy === "reset" ? (<><Spinner className="mr-1.5 inline" />{t("settings.resetting")}</>) : t("settings.confirmReset")}
                  </button>
                </div>
              ) : (
                <button
                  className="rounded-lg border border-red-500/60 px-3 py-1.5 text-xs font-medium text-red-500 hover:bg-red-500/10"
                  onClick={() => setConfirmReset(true)}
                  disabled={busy !== ""}
                >
                  {t("settings.resetBtn")}
                </button>
              )}
            </div>
          </div>
        </Section>

        <Section title={t("settings.about")}>
          <div className="text-xs text-muted">
            <p>
              <span className="font-brand text-base text-accent-500">Bilusic</span>{" "}
              · {t("settings.about1")}
            </p>
            <p className="mt-2">{t("settings.about2")}</p>
            <p className="mt-2">{t("settings.about3")}</p>
          </div>
        </Section>
      </div>

      {toasts.length > 0 && (
        <div
          className="pointer-events-none fixed bottom-20 right-6 z-50 flex w-[min(360px,calc(100vw-3rem))] flex-col gap-2"
          role="status"
          aria-live="polite"
        >
          {toasts.map((t) => {
            const isErr = t.tone === "err";
            return (
              <div
                key={t.id}
                className={`pointer-events-auto flex animate-fade-in items-start gap-2.5 rounded-2xl border px-3.5 py-2.5 shadow-float backdrop-blur ${
                  isErr
                    ? "border-red-500/40 bg-red-500/10 text-red-200"
                    : "border-accent-500/40 bg-accent-500/10 text-fg"
                }`}
              >
                <span
                  className={`mt-0.5 inline-flex h-4 w-4 shrink-0 items-center justify-center rounded-full text-[10px] ${
                    isErr ? "bg-red-500/30 text-red-100" : "bg-accent-500/30 text-accent-100"
                  }`}
                  aria-hidden
                >
                  {isErr ? (
                    <svg
                      width="11"
                      height="11"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2.4"
                      strokeLinecap="round"
                    >
                      <path d="M12 8v5M12 16.5v.5" />
                      <circle cx="12" cy="12" r="9" />
                    </svg>
                  ) : (
                    <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
                      <path d="M5 12.5l4.5 4.5L19 7.5" />
                    </svg>
                  )}
                </span>
                <p className="flex-1 text-xs leading-4">{t.text}</p>
                <button
                  type="button"
                  onClick={() => dismissToast(t.id)}
                  aria-label="dismiss"
                  className="-mr-0.5 -mt-0.5 inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-current/70 transition-colors hover:bg-white/10 focus:outline-none focus-visible:ring-2 focus-visible:ring-current/40"
                >
                  <svg
                    width="11"
                    height="11"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2.4"
                    strokeLinecap="round"
                  >
                    <path d="M6 6l12 12M6 18L18 6" />
                  </svg>
                </button>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
