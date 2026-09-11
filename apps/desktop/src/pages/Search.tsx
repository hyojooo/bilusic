import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useSearchParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import {
  SearchIcon,
  PlayIcon,
  PauseIcon,
  CloseIcon,
  EqualizerBars,
} from "@/components/icons";
import AddToPlaylistMenu from "@/components/AddToPlaylistMenu";
import { usePlayer } from "@/stores/player";
import { useSettings } from "@/stores/settings";
import {
  metadataSearch,
  metadataToplist,
  type MetadataTrack,
  extractErrorMessage,
} from "@/lib/bili";

const HISTORY_KEY = "bilusic.search.history";
const HISTORY_MAX = 10;

/** Kuwo's `toplistOp` understands 9 chart ids (kept in lockstep with
 *  `apps/desktop/plugins/kuwo/index.js::toplistOp.queries`). Surfaced in the
 *  Search tab as a one-tap "热门推荐" picker when the active metadata
 *  source is `kuwo`. */
const KUWO_CHARTS: Array<{ id: number; name: string }> = [
  { id: 1, name: "热歌榜" },
  { id: 2, name: "新歌榜" },
  { id: 3, name: "飙升榜" },
  { id: 4, name: "电音榜" },
  { id: 5, name: "ACG榜" },
  { id: 6, name: "粤语榜" },
  { id: 7, name: "欧美榜" },
  { id: 8, name: "韩语榜" },
  { id: 9, name: "日语榜" },
];

type SortMode = "default" | "dur_asc" | "dur_desc";

const PAGE_SIZE = 20;

function fmtDur(ms: number): string {
  if (!ms || !isFinite(ms)) return "0:00";
  const total = Math.floor(ms / 1000);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

function loadHistory(): string[] {
  try {
    const raw = localStorage.getItem(HISTORY_KEY);
    if (!raw) return [];
    const arr = JSON.parse(raw);
    return Array.isArray(arr) ? arr.filter((x) => typeof x === "string") : [];
  } catch {
    return [];
  }
}

function saveHistory(list: string[]) {
  try {
    localStorage.setItem(HISTORY_KEY, JSON.stringify(list.slice(0, HISTORY_MAX)));
  } catch {
    /* ignore quota / private mode */
  }
}

function sortTracks(items: MetadataTrack[], mode: SortMode): MetadataTrack[] {
  if (mode === "default") return items;
  const copy = [...items];
  copy.sort((a, b) =>
    mode === "dur_asc"
      ? a.duration_ms - b.duration_ms
      : b.duration_ms - a.duration_ms
  );
  return copy;
}

/* ── Single search result row — hover dance + playing state. ── */
function ResultRow({
  index,
  track,
  isCurrent,
  isPlaying,
  onPlay,
  onPause,
}: {
  index: number;
  track: MetadataTrack;
  isCurrent: boolean;
  isPlaying: boolean;
  onPlay: () => void;
  onPause: () => void;
}) {
  const { t } = useTranslation();
  const num = String(index + 1).padStart(2, " ");
  const showPause = isCurrent && isPlaying;

  const handlePrimary = () => {
    if (isCurrent) onPause();
    else onPlay();
  };

  return (
    <li
      className={`group relative flex items-center gap-4 px-3 py-3 transition-colors ${
        isCurrent
          ? "bg-accent-500/[0.06]"
          : "hover:bg-surface-2/70"
      }`}
    >
      {/* Index / play swap (Spotify/Last.fm classic dance). */}
      <div className="relative flex w-9 shrink-0 items-center justify-center">
        {!isCurrent ? (
          <span className="font-mono text-[13px] tabular-nums text-muted transition-opacity duration-150 group-hover:opacity-0">
            {num}
          </span>
        ) : isPlaying ? (
          <EqualizerBars className="text-accent-500" />
        ) : null}
        <button
          onClick={handlePrimary}
          aria-label={showPause ? t("common.pause") : t("common.play")}
          title={showPause ? t("common.pause") : t("common.play")}
          className={`absolute inset-0 flex items-center justify-center rounded-full transition-opacity duration-150 ${
            isCurrent
              ? "text-accent-500 opacity-100"
              : "text-fg opacity-0 group-hover:opacity-100 hover:bg-accent-500 hover:text-white"
          }`}
        >
          {showPause ? (
            <PauseIcon width={16} height={16} />
          ) : (
            <PlayIcon width={16} height={16} />
          )}
        </button>
      </div>

      {/* Cover */}
      {track.cover ? (
        <img
          src={track.cover}
          alt=""
          className="h-14 w-14 shrink-0 rounded-lg object-cover shadow-soft bg-surface-3"
        />
      ) : (
        <div className="h-14 w-14 shrink-0 rounded-lg bg-surface-3" />
      )}

      {/* Title + meta */}
      <div className="min-w-0 flex-1">
        <p
          className={`truncate text-[15px] font-medium tracking-tight ${
            isCurrent ? "text-accent-500" : "text-fg"
          }`}
          title={track.title}
        >
          {track.title}
        </p>
        <p className="mt-0.5 truncate text-xs text-muted">
          <span className="text-fg/70">{track.artist}</span>
          <span className="mx-1.5 text-line">·</span>
          <span>{track.album || "—"}</span>
          <span className="mx-1.5 text-line">·</span>
          <span className="tabular-nums">{fmtDur(track.duration_ms)}</span>
        </p>
      </div>

      {/* Right-side actions: hover-revealed + (AddToPlaylistMenu's own
          button handles its own reveal; we just keep the slot reserved
          so the cover doesn't shift when hovering). */}
      <div className="flex w-9 shrink-0 items-center justify-end">
        <AddToPlaylistMenu track={track} />
      </div>
    </li>
  );
}

export default function Search() {
  const { t } = useTranslation();
  const [params, setParams] = useSearchParams();
  const [query, setQuery] = useState(params.get("q") ?? "");
  const [items, setItems] = useState<MetadataTrack[]>([]);
  const [page, setPage] = useState(1);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [searched, setSearched] = useState(false);
  const [hasMore, setHasMore] = useState(false);
  const [sort, setSort] = useState<SortMode>("default");
  const [history, setHistory] = useState<string[]>([]);

  // Kuwo "热门推荐" chart view. When `chartView` is set we swap the result
  // list to render `chartView.items` (sourced from `metadataToplist(topid)`)
  // instead of the normal search results, and hide the discover panel —
  // a focused, single-chart list (mirrors how a user lands inside the kuwo
  // topboard UI). `chartView` is mutually exclusive with the keyword
  // search: typing a query and submitting clears the chart view first.
  const [chartView, setChartView] = useState<
    { topid: number; title: string; items: MetadataTrack[] } | null
  >(null);
  const [chartBusy, setChartBusy] = useState(false);
  const [chartError, setChartError] = useState<string | null>(null);

  const metadataSource = useSettings((s) => s.metadataSource);

  const sentinelRef = useRef<HTMLDivElement | null>(null);

  // Player state — used to mark the currently-playing row.
  const current = usePlayer((s) => s.current);
  const isPlaying = usePlayer((s) => s.isPlaying);
  const toggle = usePlayer((s) => s.toggle);
  const currentId =
    current?.track ? `${current.track.source}:${current.track.source_id}` : null;

  useEffect(() => {
    setHistory(loadHistory());
  }, []);

  const hasQuery = query.trim().length > 0;

  // Discover panel policy:
  //   • Hide it the moment the input has text — typing/searching shouldn't
  //     share the screen with stale history + hot suggestions.
  //   • Show it back when the API errors out — it's the user's escape hatch
  //     when the search itself breaks.
  //   • Hide it when we're inside a kuwo chart view — the chart list is
  //     itself the "discovery" surface; mixing it with history cards
  //     would dilute focus.
  const showDiscover = (chartView == null) && (!hasQuery || !!error);

  // Kuwo-only chart picker. The chips are hidden for every other source
  // because `metadataToplist(topid)` is a kuwo-specific contract (1..9 →
  // `toplistOp.queries`); other sources have their own (different) toplist
  // semantics that we'd need to model separately before exposing here.
  const showHotCharts = showDiscover && metadataSource === "kuwo";

  const run = useCallback(
    async (q: string, pageNum: number, replace: boolean) => {
      const kw = q.trim();
      if (!kw) return;
      setBusy(true);
      setError(null);
      try {
        const res = await metadataSearch(kw, pageNum);
        setItems((prev) => (replace ? res : [...prev, ...res]));
        setHasMore(res.length >= PAGE_SIZE);
        setSearched(true);
      } catch (e: unknown) {
        if (replace) {
          setItems([]);
          setError(t("search.failed", { msg: extractErrorMessage(e) }));
        }
      } finally {
        setBusy(false);
      }
    },
    []
  );

  const submit = useCallback(
    (q: string) => {
      const kw = q.trim();
      if (!kw) return;
      // Submitting a keyword search always drops any active chart view so
      // the two surfaces never coexist — keeps the result list a single
      // unambiguous source of truth.
      setChartView(null);
      setChartError(null);
      setQuery(kw);
      setParams({ q: kw });
      setPage(1);
      setHistory((prev) => {
        const next = [kw, ...prev.filter((h) => h !== kw)].slice(0, HISTORY_MAX);
        saveHistory(next);
        return next;
      });
      void run(kw, 1, true);
    },
    [run, setParams]
  );

  const loadMore = useCallback(() => {
    if (busy || !hasMore) return;
    const next = page + 1;
    setPage(next);
    void run(query, next, false);
  }, [busy, hasMore, page, query, run]);

  // Auto-run when arriving with ?q= (e.g. from a home card).
  useEffect(() => {
    const q = params.get("q");
    if (q) submit(q);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Infinite scroll via IntersectionObserver on the sentinel.
  useEffect(() => {
    const el = sentinelRef.current;
    if (!el || !hasMore || busy) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries[0]?.isIntersecting) loadMore();
      },
      { rootMargin: "200px" }
    );
    io.observe(el);
    return () => io.disconnect();
  }, [hasMore, busy, loadMore]);

  const sorted = useMemo(() => sortTracks(items, sort), [items, sort]);

  const play = useCallback(async (track: MetadataTrack) => {
    setError(null);
    try {
      await usePlayer.getState().resolveAndPlay(track);
    } catch (e: unknown) {
      setError(t("search.parseFailed", { msg: extractErrorMessage(e) }));
    }
  }, [t]);

  const pause = useCallback(() => {
    toggle();
  }, [toggle]);

  const clearHistory = () => {
    setHistory([]);
    saveHistory([]);
  };

  // Open a kuwo chart by its `topid`. Resets the keyword search state
  // (query / page / items) so the chart view owns the screen until the
  // user backs out — keyword search and chart view are mutually exclusive
  // surfaces in this UI.
  const openChart = useCallback(async (topid: number, title: string) => {
    setChartBusy(true);
    setChartError(null);
    setChartView({ topid, title, items: [] });
    try {
      const tracks = await metadataToplist(topid);
      setChartView({ topid, title, items: tracks });
    } catch (e: unknown) {
      setChartError(t("search.chartFailed", { msg: extractErrorMessage(e) }));
      setChartView(null);
    } finally {
      setChartBusy(false);
    }
  }, [t]);

  const closeChart = useCallback(() => {
    setChartView(null);
    setChartError(null);
  }, []);

  return (
    <div className="mx-auto max-w-4xl pb-12">
      {/* ── Editorial header ───────────────────────────────────────── */}
      <div className="pt-8">
        <p className="text-[11px] font-medium uppercase tracking-[0.18em] text-muted">
          {t("search.eyebrow")}
        </p>
        {!hasQuery && (
          <h1 className="mt-3 font-brand text-4xl leading-none text-accent-500">
            Bilusic
          </h1>
        )}
        {!hasQuery && !searched && (
          <p className="mt-2 text-sm text-muted">{t("search.hint")}</p>
        )}

        {/* Search bar — the single source of truth for the query. A
            trailing × wipes the query back to the empty/discover state
            without firing a search (clearing ≠ searching). The × only
            shows when the input is non-empty so the discover panel
            doesn't carry UI noise. */}
        <form
          className="relative mt-6"
          onSubmit={(e) => {
            e.preventDefault();
            submit(query);
          }}
        >
          <SearchIcon
            width={18}
            height={18}
            className="pointer-events-none absolute left-1 top-1/2 -translate-y-1/2 text-muted"
          />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("search.placeholder")}
            className={`w-full border-0 border-b border-line bg-transparent pb-3 pt-1 text-2xl font-medium tracking-tight text-fg placeholder:font-normal placeholder:text-muted/60 outline-none transition-colors focus:border-accent-500 ${
              hasQuery ? "pl-7 pr-12" : "px-7"
            }`}
          />
          {hasQuery && (
            <button
              type="button"
              onClick={() => {
                // Reset the entire keyword-search state so the
                // result list & sort row disappear and the
                // discover panel comes back. Don't fire `submit` —
                // clearing the input is not a search.
                setQuery("");
                setParams({});
                setItems([]);
                setSearched(false);
                setHasMore(false);
                setPage(1);
                setError(null);
                setSort("default");
                setChartView(null);
                setChartError(null);
              }}
              aria-label={t("common.clear")}
              title={t("common.clear")}
              className="absolute right-1 top-1/2 -translate-y-1/2 flex h-7 w-7 items-center justify-center rounded-full text-muted transition-colors hover:bg-surface-2 hover:text-fg"
            >
              <CloseIcon />
            </button>
          )}
        </form>

        {/* Chart mode indicator — only shown when we're inside a kuwo
            chart view. The "← 返回热门推荐" pill drops the chart view and
            brings back the discover panel so the user can pick another
            chart (or use the search box). */}
        {chartView && (
          <div className="mt-5 flex items-center justify-between gap-3">
            <p className="text-sm text-muted">
              <span className="text-fg/80">{t("search.hotRecs")}</span>
              <span className="mx-1.5 text-line">·</span>
              <span className="font-medium text-fg">{chartView.title}</span>
              {chartView.items.length > 0 && (
                <>
                  <span className="mx-1.5 text-line">·</span>
                  <span className="tabular-nums text-muted">
                    {chartView.items.length} {t("search.resultsNoun")}
                  </span>
                </>
              )}
            </p>
            <button
              onClick={closeChart}
              className="rounded-full border border-line bg-surface px-3 py-1 text-xs text-muted transition-colors hover:border-accent-500 hover:text-accent-500"
            >
              ← {t("search.backToCharts")}
            </button>
          </div>
        )}

        {/* Sort / result-count strip — only relevant while the search
            results are visible. Hidden when the input is empty (so the
            discover panel owns the screen) and when we're in a kuwo
            chart view (the chart already shows its own count above). */}
        {searched && hasQuery && sorted.length > 0 && (
          <div className="mt-5 flex flex-wrap items-baseline gap-x-5 gap-y-2">
            <p className="text-sm text-muted">
              <span className="font-medium text-fg tabular-nums">
                {sorted.length}
              </span>{" "}
              {t("search.resultsNoun")}
              {hasMore && (
                <span className="ml-2 text-xs text-muted/70">
                  {t("search.scrollMore")}
                </span>
              )}
            </p>
            <div className="flex items-center gap-1 text-xs">
              {[
                { id: "default", label: t("search.sortDefault") },
                { id: "dur_asc", label: t("search.sortAscShort") },
                { id: "dur_desc", label: t("search.sortDescShort") },
              ].map((opt) => {
                const active = sort === (opt.id as SortMode);
                return (
                  <button
                    key={opt.id}
                    onClick={() => setSort(opt.id as SortMode)}
                    className={`rounded-full px-2.5 py-1 transition-colors ${
                      active
                        ? "bg-fg text-bg"
                        : "text-muted hover:text-fg"
                    }`}
                  >
                    {opt.label}
                  </button>
                );
              })}
            </div>
          </div>
        )}
      </div>

      {/* ── Discover panel: history + hot (numbered list) ─────────── */}
      {showDiscover && (
        <div className="mt-10 space-y-8">
          {!hasQuery && history.length > 0 && (
            <section>
              <div className="mb-3 flex items-baseline justify-between">
                <h2 className="text-[11px] font-medium uppercase tracking-[0.18em] text-muted">
                  {t("search.history")}
                </h2>
                <button
                  onClick={clearHistory}
                  className="text-xs text-muted/70 underline-offset-2 transition-colors hover:text-fg hover:underline"
                >
                  {t("search.clearHistory")}
                </button>
              </div>
              <div className="flex flex-wrap gap-2">
                {history.map((h) => (
                  <button
                    key={h}
                    onClick={() => submit(h)}
                    className="rounded-full border border-line bg-surface px-3.5 py-1.5 text-sm text-fg transition-colors hover:border-accent-500 hover:text-accent-500"
                  >
                    {h}
                  </button>
                ))}
              </div>
            </section>
          )}
          {/* Kuwo-only "热门推荐" chart picker. Pure text rows, no chrome,
              no dividers, no numbers — explicitly split into two vertical
              columns (left 5, right 4) so the visual order matches
              KUWO_CHARTS (热歌→ACG on the left, 粤语→日语 on the right). */}
          {showHotCharts && (
            <section>
              <div className="mb-3 flex items-baseline justify-between">
                <h2 className="text-[11px] font-medium uppercase tracking-[0.18em] text-muted">
                  {t("search.hotChartsTitle")}
                </h2>
              </div>
              <p className="mb-4 text-xs text-muted/80">
                {t("search.hotChartsDesc")}
              </p>
              <div className="flex gap-x-6 md:gap-x-10">
                <div className="flex flex-1 flex-col">
                  {KUWO_CHARTS.slice(0, 5).map((c) => (
                    <button
                      key={c.id}
                      onClick={() => void openChart(c.id, c.name)}
                      className="py-2 text-left text-[15px] text-fg/90 transition-colors hover:text-accent-500"
                      title={c.name}
                    >
                      {c.name}
                    </button>
                  ))}
                </div>
                <div className="flex flex-1 flex-col">
                  {KUWO_CHARTS.slice(5).map((c) => (
                    <button
                      key={c.id}
                      onClick={() => void openChart(c.id, c.name)}
                      className="py-2 text-left text-[15px] text-fg/90 transition-colors hover:text-accent-500"
                      title={c.name}
                    >
                      {c.name}
                    </button>
                  ))}
                </div>
              </div>
            </section>
          )}
        </div>
      )}

      {/* ── First-page loading + error ──────────────────────────────── */}
      {busy && page === 1 && searched === false && (
        <p className="mt-12 text-sm text-muted">{t("common.searching")}</p>
      )}
      {/* Chart-mode loading + error — surfaced in the chart view's own
          row so we don't fight with the keyword-search error region. */}
      {chartView && chartBusy && (
        <p className="mt-12 text-sm text-muted">{t("search.chartLoading")}</p>
      )}
      {chartError && (
        <p className="mt-10 border-l-2 border-red-500/40 pl-4 text-sm text-muted">
          {chartError}
        </p>
      )}
      {error && (
        <p className="mt-10 border-l-2 border-red-500/40 pl-4 text-sm text-muted">
          {error}
        </p>
      )}

      {/* ── Result list (numbered, hover dance) ──────────────────────
          Drives both keyword search (`sorted`) and chart view
          (`chartView.items`). When inside a chart view we deliberately
          ignore sort modes — toplist tracks are already ordered by rank
          and re-sorting by duration would just confuse.
          Gated on `(chartView || hasQuery)` so clearing the search box
          instantly drops stale results — without this gate the
          `items` state could persist one search's worth of tracks and
          render them on top of the discover panel. */}
      {(() => {
        const list = chartView ? chartView.items : sorted;
        // Two gates:
        //  • keyword result list → only when the input is non-empty
        //  • chart view list     → only when `chartView` is set
        // Both check the `list` length as final sanity guard.
        if (!chartView && !hasQuery) return null;
        if (list.length === 0) return null;
        return (
          <ul className="mt-2 divide-y divide-line/70">
            {list.map((it, i) => {
              const id = `${it.source}:${it.source_id}`;
              const isCurrent = id === currentId;
              return (
                <ResultRow
                  key={id}
                  index={i}
                  track={it}
                  isCurrent={isCurrent}
                  isPlaying={isPlaying}
                  onPlay={() => void play(it)}
                  onPause={pause}
                />
              );
            })}
          </ul>
        );
      })()}

      {/* ── Infinite scroll sentinel + load-more ───────────────────── */}
      {!chartView && searched && hasQuery && hasMore && (
        <div ref={sentinelRef} className="h-10" aria-hidden />
      )}
      {!chartView && searched && hasQuery && hasMore && !busy && (
        <div className="mt-6 flex justify-center">
          <button
            onClick={loadMore}
            className="rounded-full border border-line px-5 py-2 text-xs text-muted transition-colors hover:border-accent-500 hover:text-accent-500"
          >
            {t("search.loadMore")}
          </button>
        </div>
      )}
      {!chartView && busy && page > 1 && (
        <p className="mt-6 text-center text-xs text-muted">
          {t("common.loading")}
        </p>
      )}

      {/* ── Empty state ────────────────────────────────────────────── */}
      {!chartView && searched && hasQuery && !busy && sorted.length === 0 && !error && (
        <div className="mt-16 flex flex-col items-center text-center">
          <div className="text-7xl font-light leading-none text-muted/30">∅</div>
          <p className="mt-5 text-base text-fg">{t("search.noResults")}</p>
          <p className="mt-1 text-xs text-muted">
            {t("search.noResultsHint")}
          </p>
        </div>
      )}
      {chartView && !chartBusy && chartView.items.length === 0 && !chartError && (
        <div className="mt-16 flex flex-col items-center text-center">
          <div className="text-7xl font-light leading-none text-muted/30">∅</div>
          <p className="mt-5 text-base text-fg">{t("search.noResults")}</p>
          <p className="mt-1 text-xs text-muted">
            {t("search.noResultsHint")}
          </p>
        </div>
      )}
    </div>
  );
}