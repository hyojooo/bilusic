import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { usePlayer } from "@/stores/player";
import {
  audioSearch,
  playByBvid,
  extractErrorMessage,
  type AudioCandidate,
  type PlaybackInfo,
} from "@/lib/bili";
import CoverImage from "./CoverImage";
import QueueDrawer from "./QueueDrawer";
import { getAudioSearchCache } from "@/lib/homeCache";
import {
  PlayIcon,
  PauseIcon,
  PrevIcon,
  NextIcon,
  VolumeIcon,
  RepeatIcon,
  ShuffleIcon,
  SourceIcon,
  CheckIcon,
  QueueIcon,
} from "./icons";

/** Delay before the picker auto-closes after a switch — long enough for the
 *  user to see which row just turned into "current", short enough not to feel
 *  sticky. They can also dismiss sooner with the × button. */
const SWITCH_FLASH_MS = 700;

function fmt(sec: number): string {
  if (!isFinite(sec) || sec < 0) sec = 0;
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

/**
 * Bottom player bar — fully wired to the Player Core (P1).
 * Transport controls (play/pause, prev/next), seek, and volume all drive the
 * single HTML5 `<audio>` element through the `usePlayer` store.
 */
interface PlayerBarProps {
  /** Open the full-screen Now Playing overlay (P7 — synced lyrics view).
   *  Wired up by `AppLayout` so the cover doubles as the lyrics trigger. */
  onOpenNowPlaying: () => void;
}

export default function PlayerBar({ onOpenNowPlaying }: PlayerBarProps) {
  const { t } = useTranslation();
  const {
    current,
    isPlaying,
    currentTime,
    duration,
    volume,
    playMode,
    error,
    toggle,
    seek,
    setVolume,
    next,
    prev,
    togglePlayMode,
    clearError,
    playNow,
    queue,
  } = usePlayer();

  const trk = current?.track;
  const s = current?.stream;
  const title = s?.title || trk?.title || "";
  const artist = s?.artist || trk?.artist || "";
  const cover = s?.cover || trk?.cover || "";

  // ── "其他音源" picker (other audio sources for the current song) ────────
  const [pickerOpen, setPickerOpen] = useState(false);
  const [pickerLoading, setPickerLoading] = useState(false);
  const [candidates, setCandidates] = useState<AudioCandidate[] | null>(null);
  const [pickerErr, setPickerErr] = useState<string | null>(null);
  const [switchingBvid, setSwitchingBvid] = useState<string | null>(null);
  /** While `>= 0`, briefly flashes the matching candidate as "just switched
   *  to" so the user sees which row their tap landed on before the popover
   *  closes itself. Cleared together with the auto-close timer. */
  const [flashBvid, setFlashBvid] = useState<string | null>(null);

  // ── "播放队列" drawer ────────────────────────────────────────────────
  const [queueOpen, setQueueOpen] = useState(false);

  const sourceQuery = `${trk?.title ?? ""} ${trk?.artist ?? ""}`.trim();

  /** bvid of the candidate we're currently playing (if any). Mirrors
   *  `current.bvid`, populated by `coordinator.play` on the by-id path and
   *  explicitly by `play_by_bvid`. `null` for cross-source resolutions
   *  (engine resolved via a free-text query, no stable engine id). */
  const currentBvid = current?.bvid ?? null;

  async function loadCandidates() {
    if (!sourceQuery) return;
    // Synchronous cache read: if a fresh result exists for this exact
    // `title + artist` key, render it instantly — no spinner, no network
    // request. Re-opening the picker for the same song within 30 min is free
    // and the list is identical every time (no B 站 dynamic-rank jitter).
    const cache = getAudioSearchCache(sourceQuery);
    const cached = cache.get();
    if (cached) {
      setPickerErr(null);
      setCandidates(cached);
      return;
    }
    setPickerLoading(true);
    setPickerErr(null);
    setCandidates(null);
    try {
      const r = await audioSearch(sourceQuery);
      // Only cache real results — an empty list is usually transient
      // (B 站 rate-limit / temp block), so a later open should retry rather
      // than show "无音源" for half an hour.
      if (r.length > 0) cache.put(r);
      setCandidates(r);
    } catch (e: unknown) {
      setPickerErr(extractErrorMessage(e));
    } finally {
      setPickerLoading(false);
    }
  }

  function togglePicker() {
    if (pickerOpen) {
      setPickerOpen(false);
      return;
    }
    if (!current) return;
    setPickerOpen(true);
    void loadCandidates();
  }

  async function switchSource(cand: AudioCandidate) {
    const cur = usePlayer.getState().current;
    if (!cur) return;
    setSwitchingBvid(cand.bvid);
    try {
      // Resolve the chosen engine source, but keep the *current song's*
      // metadata (title / artist / cover) so the player keeps showing the
      // song name while the audio bytes come from the selected source.
      const info = await playByBvid(cand.bvid);
      const merged: PlaybackInfo = {
        ...info,
        track: cur.track,
        stream: {
          ...info.stream,
          title: cur.track.title,
          artist: cur.track.artist,
          cover: cur.track.cover,
        },
      };
      playNow(merged);
      // Visually mark "we just moved to this row" before closing, so the
      // user isn't left guessing what just changed.
      setFlashBvid(cand.bvid);
      setPickerErr(null);
    } catch (e: unknown) {
      setPickerErr(extractErrorMessage(e));
    } finally {
      setSwitchingBvid(null);
    }
  }

  /** Auto-close the popover a beat after a successful switch, so the
   *  user can see the row they just landed on (or tap another one). The
   *  flash highlights "current" → fade → popover dismisses itself. */
  useEffect(() => {
    if (!flashBvid) return;
    const tid = window.setTimeout(() => {
      setPickerOpen(false);
      setFlashBvid(null);
    }, SWITCH_FLASH_MS);
    return () => window.clearTimeout(tid);
  }, [flashBvid]);

  /** If the user manually dismisses (× / backdrop) during the flash
   *  window, drop the lingering flash so a later re-open doesn't show a
   *  stale highlight. */
  useEffect(() => {
    if (!pickerOpen && flashBvid) setFlashBvid(null);
  }, [pickerOpen, flashBvid]);

  function fmtDur(sec: number): string {
    if (!sec || sec <= 0) return "";
    const m = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    return `${m}:${s.toString().padStart(2, "0")}`;
  }

  return (
    <footer className="flex h-16 shrink-0 items-center gap-4 border-t border-line bg-surface/80 px-5 backdrop-blur">
      {/* Track info — the cover is also the entry point for the full-screen
          Now Playing overlay (synced lyrics, see `pages/NowPlaying.tsx`).
          Wrapped in a button so it's keyboard-reachable and disabled when
          there's no current track. */}
      <div className="flex w-60 shrink-0 items-center gap-3">
        <button
          type="button"
          onClick={onOpenNowPlaying}
          disabled={!current}
          aria-label={t("player.openNowPlaying")}
          title={t("player.openNowPlaying")}
          className="group block shrink-0 rounded-lg focus:outline-none focus-visible:ring-2 focus-visible:ring-accent-500 disabled:cursor-default"
        >
          {cover ? (
            <img
              src={cover}
              alt=""
              className="h-11 w-11 rounded-lg border border-line object-cover transition-transform duration-200 group-hover:scale-[1.04] group-active:scale-[0.98] group-disabled:group-hover:scale-100"
            />
          ) : (
            <div className="h-11 w-11 rounded-lg border border-line bg-surface-2" />
          )}
        </button>
        <div className="min-w-0">
          <p className="truncate text-sm font-medium text-fg">
            {title || t("player.notPlaying")}
          </p>
          <p className="truncate text-xs text-muted">
            {artist || t("player.chooseBili")}
          </p>
        </div>
      </div>

      {/* Controls + progress */}
      <div className="flex flex-1 flex-col items-center gap-1">
        <div className="flex items-center gap-3">
          <button
            onClick={togglePlayMode}
            className="btn-ghost h-8 w-8 !p-0 text-accent-500 disabled:opacity-40"
            disabled={!current}
            aria-label={
              playMode === "list" ? t("player.repeatAll") : t("player.shuffle")
            }
            title={
              playMode === "list" ? t("player.repeatAll") : t("player.shuffle")
            }
          >
            {playMode === "list" ? (
              <RepeatIcon width={16} height={16} />
            ) : (
              <ShuffleIcon width={16} height={16} />
            )}
          </button>
          <button
            onClick={prev}
            className="btn-ghost h-8 w-8 !p-0 disabled:opacity-40"
            disabled={!current}
            aria-label={t("player.prev")}
          >
            <PrevIcon width={18} height={18} />
          </button>
          <button
            onClick={toggle}
            disabled={!current}
            className="flex h-10 w-10 items-center justify-center rounded-full bg-accent-500 text-white transition-transform hover:scale-105 active:scale-95 disabled:opacity-40"
            aria-label={isPlaying ? t("player.pause") : t("player.play")}
          >
            {isPlaying ? (
              <PauseIcon width={20} height={20} />
            ) : (
              <PlayIcon width={20} height={20} />
            )}
          </button>
          <button
            onClick={next}
            className="btn-ghost h-8 w-8 !p-0 disabled:opacity-40"
            disabled={!current}
            aria-label={t("player.next")}
          >
            <NextIcon width={18} height={18} />
          </button>
          {/* Other audio sources for the current song */}
          <button
            onClick={togglePicker}
            disabled={!current}
            className={`btn-ghost h-8 w-8 !p-0 disabled:opacity-30 ${pickerOpen ? "text-accent-500" : "text-muted hover:text-fg"}`}
            aria-label={t("player.otherSources")}
            title={t("player.otherSources")}
          >
            <SourceIcon width={16} height={16} />
          </button>

          {/* Play queue */}
          <button
            onClick={() => setQueueOpen(true)}
            disabled={queue.length === 0}
            className={`btn-ghost h-8 w-8 !p-0 disabled:opacity-30 ${queueOpen ? "text-accent-500" : "text-muted hover:text-fg"}`}
            aria-label={t("player.queue")}
            title={t("player.queue")}
          >
            <QueueIcon width={16} height={16} />
          </button>
        </div>

        <div className="flex w-full max-w-xl items-center gap-2">
          <span className="w-9 text-right text-[11px] tabular-nums text-muted">
            {fmt(currentTime)}
          </span>
          <input
            type="range"
            min={0}
            max={duration || 0}
            step={0.1}
            value={Math.min(currentTime, duration || 0)}
            onChange={(e) => seek(Number(e.target.value))}
            disabled={!current}
            className="h-1 flex-1 cursor-pointer appearance-none rounded-full bg-surface-2 accent-accent-500 disabled:opacity-40"
            aria-label={t("player.progress")}
          />
          <span className="w-9 text-[11px] tabular-nums text-muted">
            {fmt(duration)}
          </span>
        </div>
      </div>

      {/* Volume */}
      <div className="flex w-40 shrink-0 items-center gap-2">
        <VolumeIcon width={16} height={16} className="text-muted" />
        <input
          type="range"
          min={0}
          max={100}
          value={volume}
          onChange={(e) => setVolume(Number(e.target.value))}
          className="h-1 w-full cursor-pointer appearance-none rounded-full bg-surface-2 accent-accent-500"
          aria-label={t("player.volume")}
        />
      </div>

      {error && (
        <div className="fixed bottom-20 left-1/2 z-50 -translate-x-1/2 rounded-xl border border-red-500/40 bg-red-500/10 px-4 py-2 text-xs text-red-300 backdrop-blur">
          <span>{error}</span>
          <button
            onClick={clearError}
            className="ml-3 text-red-200 underline"
          >
            {t("common.close")}
          </button>
        </div>
      )}

      {/* "其他音源" picker popover */}
      {pickerOpen && (
        <>
          {/* Backdrop: click anywhere to dismiss */}
          <div
            className="fixed inset-0 z-40"
            onClick={() => setPickerOpen(false)}
          />
          <div className="fixed bottom-20 left-4 z-50 flex max-h-[60vh] w-80 flex-col overflow-hidden rounded-2xl border border-line bg-surface shadow-soft backdrop-blur">
            <div className="flex items-center justify-between border-b border-line px-4 py-3">
              <div className="min-w-0">
                <p className="text-sm font-medium text-fg">
                  {t("player.otherSources")}
                </p>
                <p className="truncate text-[11px] text-muted" title={sourceQuery}>
                  {sourceQuery}
                </p>
              </div>
              <button
                onClick={() => setPickerOpen(false)}
                className="btn-ghost h-7 w-7 !p-0 text-muted"
                aria-label={t("common.close")}
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
                  <path d="M6 6l12 12M18 6 6 18" />
                </svg>
              </button>
            </div>

            <div className="min-h-[3rem] flex-1 overflow-y-auto p-2">
              {pickerLoading && (
                <p className="py-6 text-center text-xs text-muted">
                  {t("player.switching")}
                </p>
              )}
              {!pickerLoading && pickerErr && (
                <p className="px-2 py-6 text-center text-xs text-red-300">
                  {t("player.otherSourcesError", { msg: pickerErr })}
                </p>
              )}
              {!pickerLoading && !pickerErr && candidates && candidates.length === 0 && (
                <p className="px-2 py-6 text-center text-xs text-muted">
                  {t("player.otherSourcesEmpty")}
                </p>
              )}
              {!pickerLoading &&
                !pickerErr &&
                candidates?.map((cand) => {
                  const isCurrent = cand.bvid === currentBvid;
                  const justSwitched = flashBvid === cand.bvid;
                  const switching = switchingBvid === cand.bvid;
                  return (
                    <button
                      key={cand.bvid}
                      onClick={() => void switchSource(cand)}
                      disabled={switchingBvid !== null}
                      className={`flex w-full items-center gap-3 rounded-xl px-2 py-2 text-left transition-colors disabled:opacity-60 ${
                        isCurrent
                          ? "bg-accent-500/10 ring-1 ring-accent-500/30 hover:bg-accent-500/15"
                          : "hover:bg-surface-2"
                      }`}
                    >
                                            <div className="relative h-10 w-10 shrink-0">
                        <CoverImage
                          src={cand.cover}
                          alt={cand.title}
                          wrapperClassName="h-10 w-10 rounded-lg"
                          className="object-cover"
                        />
                        {/* "Currently playing" badge pinned to the cover so the
                            user can see at a glance which row matches the
                            audio currently in the buffer. */}
                        {isCurrent && (
                          <span
                            className="absolute -bottom-0.5 -right-0.5 flex h-4 w-4 items-center justify-center rounded-full bg-accent-500 text-white ring-2 ring-surface"
                            aria-label={t("player.currentSource")}
                            title={t("player.currentSource")}
                          >
                            <CheckIcon width={10} height={10} />
                          </span>
                        )}
                      </div>
                      <div className="min-w-0 flex-1">
                        <p
                          className={`truncate text-[13px] ${
                            isCurrent
                              ? "font-medium text-accent-500"
                              : "text-fg"
                          }`}
                          title={cand.title}
                        >
                          {cand.title}
                        </p>
                        <p className="truncate text-[11px] text-muted">
                          {cand.author}
                          {cand.duration_sec > 0 &&
                            ` · ${fmtDur(cand.duration_sec)}`}
                        </p>
                      </div>
                      <div className="flex shrink-0 flex-col items-end">
                        {isCurrent && !justSwitched && !switching && (
                          <span className="text-[10px] uppercase tracking-wider text-accent-500">
                            {t("player.currentSource")}
                          </span>
                        )}
                        {justSwitched && (
                          <span className="flex items-center gap-1 text-[11px] text-accent-500">
                            <CheckIcon width={11} height={11} />
                            {t("player.switched")}
                          </span>
                        )}
                        {switching && (
                          <span className="text-[11px] text-accent-500">
                            {t("player.switching")}
                          </span>
                        )}
                      </div>
                    </button>
                  );
                })}
            </div>
          </div>
        </>
      )}

      {queueOpen && <QueueDrawer onClose={() => setQueueOpen(false)} />}
    </footer>
  );
}
