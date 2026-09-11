import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { LibraryIcon, PlusIcon, TrashIcon } from "@/components/icons";
import { usePlaylists } from "@/stores/playlists";
import CoverImage from "@/components/CoverImage";

export default function Playlists() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { playlists, loadPlaylists, createPlaylist, deletePlaylist, error, clearError } =
    usePlaylists();
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [err, setErr] = useState<string | null>(error);

  useEffect(() => {
    void loadPlaylists();
  }, [loadPlaylists]);

  useEffect(() => setErr(error), [error]);

  const submit = async () => {
    const n = name.trim();
    if (!n) return;
    await createPlaylist(n);
    setName("");
    setCreating(false);
  };

  const onDelete = async (id: string, plName: string) => {
    if (!confirm(t("playlists.deleteConfirm", { name: plName }))) return;
    await deletePlaylist(id);
  };

  return (
    <div className="mx-auto max-w-5xl px-4 pb-10">
      <div className="flex items-center justify-between pt-10">
        <h2 className="text-xl font-semibold text-fg">{t("playlists.title")}</h2>
        <button onClick={() => setCreating((v) => !v)} className="btn-accent">
          <PlusIcon width={18} height={18} /> {t("playlists.new")}
        </button>
      </div>

      {err && (
        <p className="mt-4 rounded-xl border border-line bg-surface-2 px-4 py-2 text-xs text-red-300">
          {err}
          <button onClick={() => { clearError(); setErr(null); }} className="ml-3 underline">{t("common.close")}</button>
        </p>
      )}

      {creating && (
        <div className="mt-4 flex gap-2">
          <input
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void submit();
              if (e.key === "Escape") setCreating(false);
            }}
            placeholder={t("playlists.namePlaceholder")}
            className="field flex-1"
          />
          <button onClick={submit} className="btn-accent">{t("playlists.create")}</button>
        </div>
      )}

      {playlists.length === 0 ? (
        <div className="mt-10 flex flex-col items-center justify-center rounded-2xl border border-dashed border-line py-16 text-center">
          <div className="flex h-14 w-14 items-center justify-center rounded-2xl bg-surface-2 text-muted">
            <LibraryIcon width={26} height={26} />
          </div>
          <p className="mt-4 text-sm font-medium text-fg">{t("playlists.empty")}</p>
          <p className="mt-1 text-xs text-muted">{t("playlists.emptyHint")}</p>
        </div>
      ) : (
        <div className="mt-6 grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4">
          {playlists.map((pl) => (
            <div
              key={pl.id}
              className="group relative flex flex-col gap-2 rounded-2xl border border-line bg-surface p-3 transition-colors hover:border-accent-500"
            >
              <button
                onClick={() => navigate(`/playlist/${pl.id}`)}
                className="flex flex-col gap-2 text-left"
              >
                <div className="flex aspect-square w-full items-center justify-center overflow-hidden rounded-xl bg-surface-2">
                  {pl.cover ? (
                    <CoverImage src={pl.cover} wrapperClassName="h-full w-full" className="object-cover" />
                  ) : (
                    <LibraryIcon width={36} height={36} className="text-muted" />
                  )}
                </div>
                <p className="truncate text-sm font-medium text-fg" title={pl.name}>
                  {pl.name}
                </p>
                <p className="text-xs text-muted">{t("playlists.trackCount", { count: pl.track_count })}</p>
              </button>
              <button
                onClick={() => void onDelete(pl.id, pl.name)}
                className="btn-ghost absolute right-2 top-2 h-8 w-8 !p-0 text-red-300 opacity-0 transition-opacity group-hover:opacity-100"
                aria-label={t("playlists.delete")}
                title={t("playlists.delete")}
              >
                <TrashIcon width={16} height={16} />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
