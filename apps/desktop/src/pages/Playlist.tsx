import { useEffect, useRef, useState } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { PlayIcon, TrashIcon, DragIcon } from "@/components/icons";
import { usePlaylists, trackCacheId } from "@/stores/playlists";
import { usePlayer } from "@/stores/player";
import { extractErrorMessage, type MetadataTrack } from "@/lib/bili";
import CoverImage from "@/components/CoverImage";

function fmtDur(ms: number): string {
  if (!ms || !isFinite(ms)) return "0:00";
  const total = Math.floor(ms / 1000);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

export default function Playlist() {
  const { t } = useTranslation();
  const { id = "" } = useParams();
  const navigate = useNavigate();
  const {
    playlists,
    tracksById,
    getTracks,
    removeTrack,
    reorder,
    renamePlaylist,
    deletePlaylist,
    error,
    clearError,
  } = usePlaylists();

  const [editing, setEditing] = useState(false);
  const [nameDraft, setNameDraft] = useState("");
  const dragIndex = useRef<number | null>(null);
  const [err, setErr] = useState<string | null>(error);

  const summary = playlists.find((p) => p.id === id);
  const tracks = tracksById[id] ?? [];

  useEffect(() => {
    if (id) void getTracks(id);
  }, [id, getTracks]);

  useEffect(() => setErr(error), [error]);

  const onRename = async () => {
    const name = nameDraft.trim();
    if (name && summary) {
      await renamePlaylist(summary.id, name);
    }
    setEditing(false);
  };

  const onDelete = async () => {
    if (!summary) return;
    if (!confirm(t("playlist.deleteConfirm", { name: summary.name }))) return;
    await deletePlaylist(summary.id);
    navigate("/playlists");
  };

  const onPlayAll = async () => {
    if (tracks.length === 0) return;
    try {
      await usePlayer.getState().enqueueAndPlay(tracks.map((t) => t.track));
    } catch (e: unknown) {
      setErr(t("playlist.playFailed", { msg: extractErrorMessage(e) }));
    }
  };

  const onPlayOne = async (track: MetadataTrack) => {
    try {
      await usePlayer.getState().resolveAndPlay(track);
    } catch (e: unknown) {
      setErr(t("playlist.playFailed", { msg: extractErrorMessage(e) }));
    }
  };

  const onRemove = async (trackId: string) => {
    if (!summary) return;
    await removeTrack(summary.id, trackId);
  };

  // Native HTML5 drag-and-drop reorder (no extra deps).
  const onDrop = async (target: number) => {
    const from = dragIndex.current;
    dragIndex.current = null;
    if (from == null || from === target || !summary) return;
    const ordered = tracks.map((t) => trackCacheId(t.track));
    const [moved] = ordered.splice(from, 1);
    ordered.splice(target, 0, moved);
    await reorder(summary.id, ordered);
  };

  return (
    <div className="mx-auto max-w-4xl">
      {/* Header */}
      <div className="flex items-end gap-5">
        <div className="h-40 w-40 shrink-0 overflow-hidden rounded-2xl border border-line bg-surface-2">
          {summary?.cover ? (
            <CoverImage src={summary.cover} wrapperClassName="h-full w-full" className="object-cover" />
          ) : null}
        </div>
        <div className="min-w-0 flex-1">
          <p className="text-xs uppercase tracking-wide text-muted">{t("playlist.label")}</p>
          {editing ? (
            <input
              autoFocus
              value={nameDraft}
              onChange={(e) => setNameDraft(e.target.value)}
              onBlur={onRename}
              onKeyDown={(e) => {
                if (e.key === "Enter") onRename();
                if (e.key === "Escape") setEditing(false);
              }}
              className="field mt-1 !text-3xl !font-bold"
            />
          ) : (
            <h2
              className="mt-1 cursor-text text-3xl font-bold text-fg"
              onClick={() => {
                setNameDraft(summary?.name ?? "");
                setEditing(true);
              }}
              title={t("playlist.clickRename")}
            >
              {summary?.name ?? t("playlist.label")}
            </h2>
          )}
          <p className="mt-2 text-sm text-muted">
            {t("playlist.tracksInfo", { count: tracks.length })}
          </p>
          <div className="mt-3 flex gap-2">
            <button onClick={onPlayAll} className="btn-accent" disabled={tracks.length === 0}>
              <PlayIcon width={18} height={18} /> {t("playlist.playAll")}
            </button>
            <button onClick={onDelete} className="btn-ghost text-red-300" disabled={!summary}>
              <TrashIcon width={18} height={18} /> {t("playlist.delete")}
            </button>
          </div>
        </div>
      </div>

      {err && (
        <p className="mt-4 rounded-xl border border-line bg-surface-2 px-4 py-2 text-xs text-red-300">
          {err}
          <button onClick={() => { clearError(); setErr(null); }} className="ml-3 underline">{t("common.close")}</button>
        </p>
      )}

      {/* Track list */}
      {tracks.length === 0 ? (
        <div className="mt-8 rounded-2xl border border-dashed border-line py-16 text-center">
          <p className="text-sm font-medium text-fg">{t("playlist.empty")}</p>
          <p className="mt-1 text-xs text-muted">
            {t("playlist.emptyHint")}
          </p>
        </div>
      ) : (
        <ul className="mt-8 divide-y divide-line">
          {tracks.map((trk, i) => (
            <li
              key={trackCacheId(trk.track)}
              draggable
              onDragStart={() => (dragIndex.current = i)}
              onDragOver={(e) => e.preventDefault()}
              onDrop={() => void onDrop(i)}
              className="group flex items-center gap-3 rounded-xl px-2 py-2 transition-colors hover:bg-surface-2"
            >
              <DragIcon
                width={16}
                height={16}
                className="shrink-0 cursor-grab text-muted opacity-0 group-hover:opacity-100"
              />
              <span className="w-5 shrink-0 text-right text-xs tabular-nums text-muted">
                {i + 1}
              </span>
              {trk.track.cover ? (
                <CoverImage src={trk.track.cover} wrapperClassName="h-10 w-10 rounded-lg" className="object-cover" />
              ) : (
                <div className="h-10 w-10 rounded-lg bg-surface-3" />
              )}
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium text-fg">{trk.track.title}</p>
                <p className="truncate text-xs text-muted">
                  {trk.track.artist} · {trk.track.album || "—"}
                </p>
              </div>
              <span className="w-12 shrink-0 text-right text-xs tabular-nums text-muted">
                {fmtDur(trk.track.duration_ms)}
              </span>
              <button
                onClick={() => void onPlayOne(trk.track)}
                className="btn-ghost h-9 w-9 !p-0 opacity-0 transition-opacity group-hover:opacity-100"
                aria-label={t("common.play")}
                title={t("common.play")}
              >
                <PlayIcon width={18} height={18} />
              </button>
              <button
                onClick={() => void onRemove(trackCacheId(trk.track))}
                className="btn-ghost h-9 w-9 !p-0 text-red-300 opacity-0 transition-opacity group-hover:opacity-100"
                aria-label={t("playlist.remove")}
                title={t("playlist.remove")}
              >
                <TrashIcon width={18} height={18} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
