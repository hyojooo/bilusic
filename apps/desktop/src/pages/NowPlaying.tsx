import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { usePlayer } from "@/stores/player";
import {
  metadataGetLyrics,
  extractErrorMessage,
  type Lyrics,
} from "@/lib/bili";
import {
  PlayIcon,
  PauseIcon,
  PrevIcon,
  NextIcon,
  RepeatIcon,
  ShuffleIcon,
  CloseIcon,
} from "@/components/icons";

interface NowPlayingProps {
  onClose: () => void;
}

function fmt(sec: number): string {
  if (!isFinite(sec) || sec < 0) sec = 0;
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

/**
 * Full-screen "Now Playing" overlay (P7 — lyrics).
 *
 * Layout: left column = cover + meta + transport; right column = plain lyric
 * text (no highlight, no auto-scroll, no click-to-seek — on Bilibili audio the
 * lyric timestamps won't line up with playback anyway, so lyrics are shown as
 * a calm, uniform read-only transcript).
 *
 * Lyrics source strategy (smart fallback):
 *   1. Active metadata source via `metadataGetLyrics("")` — QQ now returns
 *      real lyrics via musicu.fcg / PlayLyricInfo.
 *   2. If empty, fall back to the lyrics-only `lrclib` source.
 *   3. Empty state shows "暂无歌词" — never silently white-screen.
 *
 * Re-fetch is gated on the track fingerprint (source_id + lyrics_id + title +
 * artist) so seeking within the same track does NOT trigger a new request.
 */
export default function NowPlaying({ onClose }: NowPlayingProps) {
  const { t } = useTranslation();
  const {
    current,
    isPlaying,
    currentTime,
    duration,
    playMode,
    toggle,
    seek,
    next,
    prev,
    togglePlayMode,
  } = usePlayer();

  const trk = current?.track;
  const s = current?.stream;
  const title = s?.title || trk?.title || "";
  const artist = s?.artist || trk?.artist || "";
  const cover = s?.cover || trk?.cover || "";

  // ── Lyrics fetch lifecycle ─────────────────────────────────────────
  const [lyrics, setLyrics] = useState<Lyrics | null>(null);
  const [lyricsLoading, setLyricsLoading] = useState(false);
  const [lyricsErr, setLyricsErr] = useState<string | null>(null);

  const trkKey = useMemo(() => {
    if (!trk) return "";
    return [
      trk.source_id ?? "",
      trk.lyrics_id ?? "",
      trk.title ?? "",
      trk.artist ?? "",
    ].join("|");
  }, [trk]);

  // Shared fetch body — extracted so the retry button can reuse the same
  // active→lrclib fallback chain without duplicating ~30 lines.
  const fetchLyrics = async (track: NonNullable<typeof trk>) => {
    setLyricsLoading(true);
    setLyricsErr(null);
    let result: Lyrics | null = null;
    try {
      result = await metadataGetLyrics("", track);
    } catch (e) {
      setLyricsErr(extractErrorMessage(e));
    }
    if (!result || result.lines.length === 0) {
      try {
        const fb = await metadataGetLyrics("lrclib", track);
        if (fb.lines.length > 0) result = fb;
      } catch (e) {
        // Preserve any error message already shown from the primary call;
        // lrclib blips shouldn't clobber the real reason.
        setLyricsErr((prev) => prev ?? extractErrorMessage(e));
      }
    }
    setLyrics(result);
    setLyricsLoading(false);
  };

  useEffect(() => {
    if (!trk) {
      setLyrics(null);
      setLyricsErr(null);
      setLyricsLoading(false);
      return;
    }
    let alive = true;
    (async () => {
      if (!alive) return;
      await fetchLyrics(trk);
    })();
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [trkKey]);

  // ── Keyboard: Esc closes the overlay ──────────────────────────────
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="fixed inset-0 z-50 flex flex-col overflow-hidden bg-bg/95 backdrop-blur-xl">
      {/* Top bar — back to player + dismiss */}
      <header className="relative z-10 flex shrink-0 items-center justify-between px-6 py-4">
        <button
          onClick={onClose}
          className="btn-ghost h-9 w-9 !p-0 text-muted hover:text-fg"
          aria-label={t("nowPlaying.back")}
          title={t("nowPlaying.back")}
        >
          <CloseIcon width={18} height={18} />
        </button>
        <div className="w-9" />
      </header>

      {/* Main two-column body */}
      <div className="relative z-10 flex flex-1 min-h-0 gap-10 px-10 pb-8">
        {/* ── Left column: cover + meta + transport ────────────────── */}
        <div className="flex w-[42%] min-w-0 flex-col items-center gap-6">
          {cover ? (
            <img
              src={cover}
              alt=""
                className="aspect-square w-full max-w-md rounded-2xl object-cover
                           ring-1 ring-line shadow-2xl"
              draggable={false}
            />
          ) : (
            <div className="aspect-square w-full max-w-md rounded-2xl bg-surface-2 ring-1 ring-line" />
          )}

          <div className="w-full max-w-md text-center">
            <h1
              className="truncate text-2xl font-semibold text-fg"
              title={title}
            >
              {title || t("player.notPlaying")}
            </h1>
            <p className="mt-1 truncate text-sm text-muted" title={artist}>
              {artist}
            </p>
          </div>

          {/* Progress + transport */}
          <div className="flex w-full max-w-md flex-col gap-4">
            <div className="flex items-center gap-2">
              <span className="w-10 shrink-0 text-right text-[11px] tabular-nums text-muted">
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
                aria-label={t("player.progress")}
                className="h-1 flex-1 cursor-pointer appearance-none rounded-full
                           bg-surface-2 accent-accent-500 disabled:opacity-40"
              />
              <span className="w-10 shrink-0 text-[11px] tabular-nums text-muted">
                {fmt(duration)}
              </span>
            </div>
            <div className="flex items-center justify-center gap-4">
              <button
                onClick={togglePlayMode}
                disabled={!current}
                className="btn-ghost h-9 w-9 !p-0 text-accent-500 disabled:opacity-40"
                aria-label={
                  playMode === "list"
                    ? t("player.repeatAll")
                    : t("player.shuffle")
                }
                title={
                  playMode === "list"
                    ? t("player.repeatAll")
                    : t("player.shuffle")
                }
              >
                {playMode === "list" ? (
                  <RepeatIcon width={18} height={18} />
                ) : (
                  <ShuffleIcon width={18} height={18} />
                )}
              </button>
              <button
                onClick={prev}
                disabled={!current}
                className="btn-ghost h-10 w-10 !p-0 disabled:opacity-40"
                aria-label={t("player.prev")}
              >
                <PrevIcon width={20} height={20} />
              </button>
              <button
                onClick={toggle}
                disabled={!current}
                className="flex h-12 w-12 items-center justify-center rounded-full bg-accent-500 text-white
                           transition-transform hover:scale-105 active:scale-95 disabled:opacity-40"
                aria-label={isPlaying ? t("player.pause") : t("player.play")}
              >
                {isPlaying ? (
                  <PauseIcon width={24} height={24} />
                ) : (
                  <PlayIcon width={24} height={24} />
                )}
              </button>
              <button
                onClick={next}
                disabled={!current}
                className="btn-ghost h-10 w-10 !p-0 disabled:opacity-40"
                aria-label={t("player.next")}
              >
                <NextIcon width={20} height={20} />
              </button>
            </div>
          </div>
        </div>

        {/* ── Right column: lyrics ─────────────────────────────────── */}
        <div className="flex w-[58%] min-w-0 flex-col">
          <div
            className="flex-1 overflow-x-hidden overflow-y-scroll pl-16 pr-2 pt-2 pb-10
                       [scrollbar-width:none] [-ms-overflow-style:none]
                       [&::-webkit-scrollbar]:hidden"
          >
            {lyricsLoading && (
              <p className="py-12 text-center text-sm text-muted">
                {t("nowPlaying.loading")}
              </p>
            )}
            {!lyricsLoading && lyricsErr && (
              <div className="py-12 text-center">
                <p className="text-sm text-red-300">
                  {t("nowPlaying.error", { msg: lyricsErr })}
                </p>
                <button
                  onClick={() => trk && fetchLyrics(trk)}
                  className="mt-3 text-xs text-accent-500 underline-offset-2 hover:underline"
                >
                  {t("nowPlaying.retry")}
                </button>
              </div>
            )}
            {!lyricsLoading && !lyricsErr && lyrics?.lines.length === 0 && (
              <p className="py-12 text-center text-sm text-muted">
                {t("nowPlaying.noLyrics")}
              </p>
            )}
            {!lyricsLoading && lyrics && lyrics.lines.length > 0 && (
              <ul className="flex flex-col gap-3">
                {lyrics.lines.map((ln, idx) => (
                  <li
                    key={idx}
                    className="w-full text-left text-sm leading-snug text-fg/70"
                  >
                    {ln.text || " "}
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}