import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { PlusIcon, CheckIcon, LibraryIcon, NextIcon } from './icons';
import { usePlaylists } from '@/stores/playlists';
import { usePlayer } from '@/stores/player';
import type { MetadataTrack } from '@/lib/bili';

/**
 * A "+" button that opens a popover to add the given track to one of the
 * user's playlists (or create a new one on the fly). Reused by the search,
 * home, and playlist-detail result rows.
 *
 * Positioning contract:
 *   The caller fully owns the wrapper's positioning via `className`. If no
 *   className is passed, we fall back to `relative` so legacy callers that
 *   relied on the popover living inside the wrapper still work.
 *
 *   We intentionally do NOT merge a default `relative` into a passed
 *   `className` — Tailwind v3 lays out position utilities alphabetically
 *   (`.absolute` precedes `.relative`), so merging produces the class string
 *   `relative absolute right-2 top-2`, in which the LATER declaration in the
 *   generated CSS (`.relative`) wins the cascade and the wrapper becomes
 *   `position: relative` instead of `absolute`. That bug pinned the "+" to
 *   the row directly under the cover and dropped the popover onto the title
 *   instead of overlaying it on the cover (see N11y report).
 *
 * Popover via Portal:
 *   The popover is rendered to `document.body` via createPortal with
 *   `position: fixed`. That severs it from any ancestor `overflow:hidden /
 *   auto` (the horizontal scroll rows on Home clip absolute children that
 *   spill past their track gap), `transform/filter/opacity` that would
 *   otherwise trap the popover in a parent stacking context, and
 *   `[contain:paint]`. Position is recomputed on open and on any ancestor
 *   scroll / window resize. We also flip the popover to the opposite side
 *   when it would overflow the viewport edges.
 */
export default function AddToPlaylistMenu({
  track,
  className = '',
}: {
  track: MetadataTrack;
  className?: string;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [justAdded, setJustAdded] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement | null>(null);
  const popoverRef = useRef<HTMLDivElement | null>(null);
  // `{top,left}` in viewport coords for `position: fixed`. `null` until the
  // first measure so the popover never flashes at (0,0).
  const [popPos, setPopPos] = useState<{ top: number; left: number } | null>(
    null,
  );

  const { playlists, loadPlaylists, createPlaylist, addTrack } = usePlaylists();

  useEffect(() => {
    if (!open) return;
    void loadPlaylists();
    const onDown = (e: MouseEvent) => {
      // Close if the click landed outside BOTH the button wrapper and the
      // portaled popover (otherwise the popover would close instantly when
      // we click inside it).
      const inButton = ref.current && ref.current.contains(e.target as Node);
      const inPopover =
        popoverRef.current && popoverRef.current.contains(e.target as Node);
      if (!inButton && !inPopover) setOpen(false);
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [open, loadPlaylists]);

  // Position the popover every time it opens, and recompute on scroll/resize.
  // Default: popover's right edge aligns with the button's right edge, popover
  // hangs 4px below the button (mirrors the old `mt-1`). Flip to the opposite
  // side if it would overflow the viewport.
  const POPOVER_WIDTH = 224; // w-56
  const GUTTER = 8;
  useLayoutEffect(() => {
    if (!open) {
      setPopPos(null);
      return;
    }
    const place = () => {
      const btn = ref.current;
      const pop = popoverRef.current;
      if (!btn) return;
      const r = btn.getBoundingClientRect();
      const ph = pop?.offsetHeight ?? 280;

      // Default: right-aligned (right edge = button right edge).
      let left = r.right - POPOVER_WIDTH;
      // Flip / clamp horizontally.
      if (left + POPOVER_WIDTH > window.innerWidth - GUTTER) {
        // Won't fit on the right → switch to left-aligned.
        left = r.left;
        if (left + POPOVER_WIDTH > window.innerWidth - GUTTER) {
          left = window.innerWidth - POPOVER_WIDTH - GUTTER;
        }
      }
      if (left < GUTTER) left = GUTTER;

      // Default: below the button.
      let top = r.bottom + 4;
      // Flip above if it would overflow the bottom of the viewport.
      if (top + ph > window.innerHeight - GUTTER) {
        top = r.top - ph - 4;
        if (top < GUTTER) top = GUTTER;
      }

      setPopPos({ top, left });
    };
    place();
    // Capture phase catches any scroll in any ancestor (Home's horizontal
    // row, sidebar, etc.) — bubbles don't fire on document for those.
    window.addEventListener('scroll', place, true);
    window.addEventListener('resize', place);
    return () => {
      window.removeEventListener('scroll', place, true);
      window.removeEventListener('resize', place);
    };
  }, [open]);

  const add = async (playlistId: string) => {
    await addTrack(playlistId, track);
    setJustAdded(playlistId);
    setTimeout(() => setJustAdded(null), 1200);
  };

  const enqueue = async () => {
    await usePlayer.getState().enqueue([track]);
    setOpen(false);
  };

  const createAndAdd = async () => {
    const name = t('addMenu.defaultName', { n: playlists.length + 1 });
    const pl = await createPlaylist(name);
    await addTrack(pl.id, track);
    setJustAdded(pl.id);
    setTimeout(() => setJustAdded(null), 1200);
    setOpen(false);
  };

  return (
    <div ref={ref} className={className || 'relative'}>
      <button
        onClick={() => setOpen((v) => !v)}
        className="btn-ghost h-9 w-9 !p-0 opacity-0 transition-opacity group-hover:opacity-100"
        aria-label={t('addMenu.title')}
        title={t('addMenu.title')}
      >
        <PlusIcon width={18} height={18} />
      </button>

      {open &&
        popPos &&
        typeof document !== 'undefined' &&
        createPortal(
          <div
            ref={popoverRef}
            style={{ position: 'fixed', top: popPos.top, left: popPos.left }}
            className="z-50 w-56 overflow-hidden rounded-xl border border-line bg-surface shadow-xl"
          >
            <p className="border-b border-line px-3 py-2 text-xs font-medium text-muted">
              {t('addMenu.title')}
            </p>
            <button
              onClick={() => void enqueue()}
              className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-fg transition-colors hover:bg-surface-2"
            >
              <NextIcon
                width={16}
                height={16}
                className="shrink-0 text-muted"
              />
              <span className="min-w-0 flex-1 truncate">
                {t('addMenu.enqueue')}
              </span>
            </button>
            <p className="border-t border-line px-3 pt-2 pb-1 text-[10px] uppercase tracking-wider text-muted">
              {t('addMenu.playlistSection')}
            </p>
            <div className="max-h-64 overflow-y-auto pb-1">
              {playlists.length === 0 && (
                <p className="px-3 py-3 text-xs text-muted">
                  {t('addMenu.noPlaylists')}
                </p>
              )}
              {playlists.map((pl) => (
                <button
                  key={pl.id}
                  onClick={() => void add(pl.id)}
                  className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-fg transition-colors hover:bg-surface-2"
                >
                  <LibraryIcon
                    width={16}
                    height={16}
                    className="shrink-0 text-muted"
                  />
                  <span className="min-w-0 flex-1 truncate">{pl.name}</span>
                  {justAdded === pl.id && (
                    <CheckIcon
                      width={16}
                      height={16}
                      className="text-accent-500"
                    />
                  )}
                </button>
              ))}
            </div>
            <button
              onClick={() => void createAndAdd()}
              className="flex w-full items-center gap-2 border-t border-line px-3 py-2 text-left text-sm text-accent-600 transition-colors hover:bg-surface-2"
            >
              <PlusIcon width={16} height={16} />
              {t('addMenu.newAndAdd')}
            </button>
          </div>,
          document.body,
        )}
    </div>
  );
}
