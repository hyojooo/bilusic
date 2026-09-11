import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { usePlayer } from "@/stores/player";
import { TrashIcon, DragIcon, PlayIcon } from "@/components/icons";
import CoverImage from "@/components/CoverImage";

function fmtDur(ms: number): string {
  if (!ms || !isFinite(ms)) return "";
  const total = Math.floor(ms / 1000);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

interface QueueDrawerProps {
  onClose: () => void;
}

/**
 * "Play queue" popover — mirrored on the right of the player bar (the "other
 * sources" picker sits on the left). Lists every queued track, highlights the
 * currently-playing one, lets you drag to reorder, remove a single entry, or
 * clear the whole queue. **Clicking a row is intentionally a no-op** — to
 * actually switch to a different entry, hover it and use the explicit play
 * button on the right; this prevents accidental "click-while-scrolling"
 * interruption of the currently-playing track.
 */
export default function QueueDrawer({ onClose }: QueueDrawerProps) {
  const { t } = useTranslation();
  const queue = usePlayer((s) => s.queue);
  const index = usePlayer((s) => s.index);
  const jumpTo = usePlayer((s) => s.jumpTo);
  const removeAt = usePlayer((s) => s.removeAt);
  const moveItem = usePlayer((s) => s.moveItem);
  const clearQueue = usePlayer((s) => s.clearQueue);

  const dragIndex = useRef<number | null>(null);

  return (
    <>
      {/* Backdrop: click anywhere to dismiss */}
      <div className="fixed inset-0 z-40" onClick={onClose} />
      <div className="fixed bottom-20 right-4 z-50 flex max-h-[60vh] w-80 flex-col overflow-hidden rounded-2xl border border-line bg-surface shadow-soft backdrop-blur">
        <div className="flex items-center justify-between border-b border-line px-4 py-3">
          <div className="min-w-0">
            <p className="text-sm font-medium text-fg">{t("player.queue")}</p>
            <p className="text-[11px] text-muted">
              {t("player.queueCount", { count: queue.length })}
            </p>
          </div>
          <div className="flex items-center gap-1">
            {queue.length > 0 && (
              <button
                onClick={() => clearQueue()}
                className="btn-ghost h-7 px-2 text-[11px] text-red-300"
                aria-label={t("player.clearQueue")}
                title={t("player.clearQueue")}
              >
                {t("player.clearQueue")}
              </button>
            )}
            <button
              onClick={onClose}
              className="btn-ghost h-7 w-7 !p-0 text-muted"
              aria-label={t("common.close")}
            >
              <svg
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.8"
                strokeLinecap="round"
              >
                <path d="M6 6l12 12M18 6 6 18" />
              </svg>
            </button>
          </div>
        </div>

        <div className="min-h-[3rem] flex-1 overflow-y-auto p-2">
          {queue.length === 0 ? (
            <p className="px-2 py-6 text-center text-xs text-muted">
              {t("player.queueEmpty")}
            </p>
          ) : (
            <ul className="divide-y divide-line">
              {queue.map((item, i) => {
                const isCurrent = i === index;
                const trk = item.track;
                return (
                  <li
                    key={i}
                    draggable
                    onDragStart={() => (dragIndex.current = i)}
                    onDragOver={(e) => e.preventDefault()}
                    onDrop={() => {
                      const from = dragIndex.current;
                      dragIndex.current = null;
                      if (from != null && from !== i) moveItem(from, i);
                    }}
                    className={`group flex items-center gap-3 rounded-xl px-2 py-2 transition-colors ${
                      isCurrent
                        ? "bg-accent-500/10 ring-1 ring-accent-500/30"
                        : "hover:bg-surface-2"
                    }`}
                  >
                    <DragIcon
                      width={16}
                      height={16}
                      className="shrink-0 cursor-grab text-muted opacity-0 group-hover:opacity-100"
                    />
                    <span className="w-5 shrink-0 text-right text-xs tabular-nums text-muted">
                      {i + 1}
                    </span>
                    {trk.cover ? (
                      <CoverImage
                        src={trk.cover}
                        alt={trk.title}
                        wrapperClassName="h-10 w-10 rounded-lg"
                        className="object-cover"
                      />
                    ) : (
                      <div className="h-10 w-10 shrink-0 rounded-lg bg-surface-3" />
                    )}
                    <div className="min-w-0 flex-1">
                      <p
                        className={`truncate text-[13px] ${
                          isCurrent ? "font-medium text-accent-500" : "text-fg"
                        }`}
                        title={trk.title}
                      >
                        {trk.title}
                      </p>
                      <p className="truncate text-[11px] text-muted">
                        {trk.artist}
                        {trk.album ? ` · ${trk.album}` : ""}
                      </p>
                    </div>
                    {fmtDur(trk.duration_ms) && (
                      <span className="w-10 shrink-0 text-right text-[11px] tabular-nums text-muted">
                        {fmtDur(trk.duration_ms)}
                      </span>
                    )}
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        jumpTo(i);
                      }}
                      className="btn-ghost h-8 w-8 !p-0 text-accent-500 opacity-0 transition-opacity group-hover:opacity-100"
                      aria-label={t("player.jumpTo")}
                      title={t("player.jumpTo")}
                    >
                      <PlayIcon width={14} height={14} />
                    </button>
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        removeAt(i);
                      }}
                      className="btn-ghost h-8 w-8 !p-0 text-red-300 opacity-0 transition-opacity group-hover:opacity-100"
                      aria-label={t("player.removeFromQueue")}
                      title={t("player.removeFromQueue")}
                    >
                      <TrashIcon width={16} height={16} />
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      </div>
    </>
  );
}
