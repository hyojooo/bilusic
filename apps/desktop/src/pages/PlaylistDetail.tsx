/**
 * Playlist detail page — the "click → actual songs" path that replaces the
 * old `/search?q=<title>` placeholder behavior for third-party playlists on
 * the home feed (QQ / 网易云 / etc.).
 *
 *  - Backend route: `coordinator.get_playlist(source, id)` → metadata plugin's
 *    `get_playlist(id)` → real playlist-detail API on the active source.
 *  - Header (title / cover / subtitle) comes from the home-feed SectionItem
 *    via React Router `state`, so we don't need a separate header endpoint
 *    and the page is snappy on first paint. If the user lands here directly
 *    (no state), the header just doesn't render — the track list still
 *    works on its own.
 *  - Tracks render with the same Spotify-style hover dance as the Search
 *    page (`PlayIcon` swaps to a circular play button on hover; shows
 *    "currently playing" + equalizer when matched).
 *  - "全部播放" → `enqueueAndPlay(tracks)` so the queue is populated with
 *    the full playlist (so the queue drawer shows it too) and playback
 *    starts from index 0.
 */
import { useEffect, useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate, useParams, useLocation } from "react-router-dom";
import { metadataGetPlaylist, type FeedSong, type MetadataTrack, extractErrorMessage } from "@/lib/bili";
import { usePlayer } from "@/stores/player";
import AddToPlaylistMenu from "@/components/AddToPlaylistMenu";
import CoverImage from "@/components/CoverImage";
import { EqualizerBars, PauseIcon, PlayIcon, ArrowLeftIcon } from "@/components/icons";

/** Header metadata carried over from the home-feed card. Optional because a
 *  user can land on this URL directly (shared link / reload) with no state. */
interface NavState {
  title?: string;
  cover?: string;
  subtitle?: string;
}

export default function PlaylistDetail() {
  const { t } = useTranslation();
  const { source = "", id = "" } = useParams<{ source: string; id: string }>();
  const nav = useNavigate();
  const location = useLocation();
  const header = (location.state as NavState | null) ?? {};

  const [tracks, setTracks] = useState<FeedSong[] | null>(null);
  const [err, setErr] = useState<string | null>(null);

  // Subscribe to player state for the "currently playing" highlight.
  const current = usePlayer((s) => s.current);
  const isPlaying = usePlayer((s) => s.isPlaying);
  const toggle = usePlayer((s) => s.toggle);
  const enqueueAndPlay = usePlayer((s) => s.enqueueAndPlay);
  const resolveAndPlay = usePlayer((s) => s.resolveAndPlay);

  const currentId = current?.track
    ? `${current.track.source}:${current.track.source_id}`
    : null;

  useEffect(() => {
    if (!source || !id) {
      setErr(t("playlistDetail.badUrl"));
      setTracks([]);
      return;
    }
    let alive = true;
    setTracks(null);
    setErr(null);
    metadataGetPlaylist(source, id)
      .then((songs) => {
        if (!alive) return;
        setTracks(songs);
      })
      .catch((e: unknown) => {
        if (!alive) return;
        const msg = extractErrorMessage(e);
        // Backend's default `get_playlist` impl returns an "不支持" error for
        // sources that haven't implemented it (e.g. 酷我 JS). Surface that
        // as a friendly empty state rather than a scary error toast.
        if (msg.includes("不支持")) {
          setErr(t("playlistDetail.unsupported", { source }));
          setTracks([]);
        } else {
          setErr(t("playlistDetail.failed", { msg }));
          setTracks([]);
        }
      });
    return () => {
      alive = false;
    };
  }, [source, id, t]);

  /** Convert FeedSong → MetadataTrack for `resolveAndPlay`. Mirrors the
   *  reverse direction in Home.tsx (`songFromTrack`) but going the other
   *  way. Fields we don't have (album, lyrics_id) are left empty. */
  const feedToTrack = (s: FeedSong): MetadataTrack => ({
    source_id: s.source_id,
    source: s.source,
    engine_hint: s.engine_hint,
    title: s.title,
    artist: s.artist,
    album: "",
    duration_ms: s.duration_ms,
    cover: s.cover,
    lyrics_id: null,
  });

  const playOne = useCallback(
    async (song: FeedSong) => {
      try {
        await resolveAndPlay(feedToTrack(song));
      } catch (e: unknown) {
        setErr(t("playlistDetail.playFailed", { msg: extractErrorMessage(e) }));
      }
    },
    [resolveAndPlay, t],
  );

  /** "全部播放" → fill the queue with the whole playlist and start at 0.
   *  Same UX as clicking a playlist card in the legacy Spotify-like player:
   *  everything the user is about to hear is now in the queue drawer. */
  const playAll = useCallback(async () => {
    if (!tracks || tracks.length === 0) return;
    try {
      await enqueueAndPlay(tracks.map(feedToTrack));
    } catch (e: unknown) {
      setErr(t("playlistDetail.playFailed", { msg: extractErrorMessage(e) }));
    }
  }, [tracks, enqueueAndPlay, t]);

  return (
    <div className="flex h-full flex-col overflow-y-auto bg-bg">
      {/* Top bar: back + (optional) header */}
      <div className="sticky top-0 z-10 flex items-center gap-3 border-b border-line bg-surface/80 px-5 py-3 backdrop-blur">
        <button
          onClick={() => nav(-1)}
          className="btn-ghost h-9 w-9 !p-0"
          aria-label={t("common.back")}
          title={t("common.back")}
        >
          <ArrowLeftIcon width={18} height={18} />
        </button>
        <div className="min-w-0 flex-1">
          <p className="truncate text-base font-semibold text-fg" title={header.title}>
            {header.title ?? t("playlistDetail.title")}
          </p>
          {header.subtitle && (
            <p className="truncate text-xs text-muted">{header.subtitle}</p>
          )}
        </div>
      </div>

      {/* Hero header (only when we have state from the home card) */}
      {header.cover && (
        <div className="flex gap-5 border-b border-line bg-surface/40 px-5 py-6">
          <CoverImage
            src={header.cover}
            alt={header.title ?? ""}
            wrapperClassName="h-44 w-44 shrink-0 rounded-2xl"
            className="object-cover shadow-soft"
          />
          <div className="flex min-w-0 flex-1 flex-col justify-end gap-3">
            <div className="min-w-0">
              <h1 className="line-clamp-2 text-2xl font-bold tracking-tight text-fg">
                {header.title ?? t("playlistDetail.title")}
              </h1>
              {header.subtitle && (
                <p className="mt-1 text-sm text-muted">{header.subtitle}</p>
              )}
            </div>
            <div className="flex items-center gap-3">
              <button
                onClick={playAll}
                disabled={!tracks || tracks.length === 0}
                className="flex items-center gap-2 rounded-full bg-accent-500 px-5 py-2 text-sm font-medium text-white transition-transform hover:scale-105 active:scale-95 disabled:opacity-40"
              >
                <PlayIcon width={16} height={16} />
                {t("playlistDetail.playAll")}
              </button>
              {tracks && (
                <span className="text-xs text-muted">
                  {t("playlistDetail.trackCount", { n: tracks.length })}
                </span>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Body: loading / error / empty / list */}
      <div className="flex-1 px-5 pb-32 pt-5">
        {tracks == null && !err && (
          <p className="mt-10 text-center text-sm text-muted">
            {t("playlistDetail.loading")}
          </p>
        )}
        {err && tracks && tracks.length === 0 && (
          <div className="mt-12 text-center">
            <p className="text-sm text-muted">{err}</p>
            {header.cover == null && (
              <button
                onClick={() => nav(-1)}
                className="mt-4 btn-ghost"
              >
                {t("common.back")}
              </button>
            )}
          </div>
        )}
        {tracks && tracks.length > 0 && (
          <ul className="overflow-hidden rounded-xl border border-line">
            {tracks.map((song, i) => {
              const rowId = `${song.source}:${song.source_id}`;
              const isCurrent = rowId === currentId;
              const showPause = isCurrent && isPlaying;
              const handlePrimary = () => {
                if (isCurrent) toggle();
                else void playOne(song);
              };
              return (
                <li
                  key={`${i}:${rowId}`}
                  className={`group relative flex items-center gap-3 px-3 py-2 transition-colors ${
                    isCurrent
                      ? "bg-accent-500/[0.06]"
                      : "hover:bg-surface-2/70"
                  } ${i > 0 ? "border-t border-line/60" : ""}`}
                >
                  {/* Index / play swap */}
                  <div className="relative flex w-8 shrink-0 items-center justify-center">
                    {!isCurrent ? (
                      <span className="font-mono text-[12px] tabular-nums text-muted transition-opacity group-hover:opacity-0">
                        {String(i + 1).padStart(2, " ")}
                      </span>
                    ) : isPlaying ? (
                      <EqualizerBars className="text-accent-500" />
                    ) : null}
                    <button
                      onClick={handlePrimary}
                      aria-label={
                        showPause ? t("common.pause") : t("common.play")
                      }
                      className={`absolute inset-0 flex items-center justify-center rounded-full transition-opacity ${
                        isCurrent
                          ? "text-accent-500 opacity-100"
                          : "text-fg opacity-0 group-hover:opacity-100"
                      }`}
                    >
                      {showPause ? (
                        <PauseIcon width={14} height={14} />
                      ) : (
                        <PlayIcon width={14} height={14} />
                      )}
                    </button>
                  </div>

                  {/* Cover */}
                  {song.cover ? (
                    <img
                      src={song.cover}
                      alt=""
                      className="h-11 w-11 shrink-0 rounded-lg object-cover bg-surface-3"
                    />
                  ) : (
                    <div className="h-11 w-11 shrink-0 rounded-lg bg-surface-3" />
                  )}

                  {/* Title + artist + duration */}
                  <div className="min-w-0 flex-1">
                    <p
                      className={`truncate text-[14px] ${
                        isCurrent ? "font-medium text-accent-500" : "text-fg"
                      }`}
                      title={song.title}
                    >
                      {song.title}
                    </p>
                    <p className="truncate text-xs text-muted">
                      {song.artist || "—"}
                    </p>
                  </div>

                  <span className="shrink-0 font-mono text-[11px] tabular-nums text-muted">
                    {fmtDur(song.duration_ms)}
                  </span>

                  <AddToPlaylistMenu
                    track={feedToTrack(song)}
                    className="shrink-0"
                  />
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </div>
  );
}

function fmtDur(ms: number): string {
  if (!ms || ms <= 0) return "";
  const total = Math.floor(ms / 1000);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}