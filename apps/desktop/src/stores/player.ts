import { create } from "zustand";
import { persist } from "zustand/middleware";
import {
  proxyStreamUrl,
  coordinatorPlay,
  historyAdd,
  type PlaybackInfo,
  type MetadataTrack,
  extractErrorMessage,
} from "@/lib/bili";
import i18n from "@/i18n";

/**
 * Player Core (P1, extended in P3).
 *
 * Wraps a single HTML5 `<audio>` element, exposes transport controls + a
 * queue, a play mode (list-loop / shuffle), and integrates with the OS media session.
 *
 * The audio element itself is kept module-scoped (never in React state) so it
 * is never re-created on re-render. Playback state is mirrored into the store.
 *
 * Decoupled architecture: the store never resolves audio on its own. Callers
 * feed in a fully-resolved `PlaybackInfo` (got via `coordinatorPlay`).
 *
 * Persistence (P3): the queue (track metadata only), current index, play mode,
 * and volume are saved to disk. Bilibili stream URLs are short-lived
 * (signed + expiring), so on restore we re-resolve the current track rather
 * than trust a persisted URL.
 */

export type PlayMode = "list" | "shuffle";

interface QueueItem {
  /** The original metadata (used for richer UI when no PlaybackInfo yet). */
  track: MetadataTrack;
  /** The resolved stream + audio info; null until resolved. */
  resolved: PlaybackInfo | null;
}

interface PlayerState {
  /** Currently playing resolved track. */
  current: PlaybackInfo | null;
  /** Pending queue (each entry may or may not be resolved yet). */
  queue: QueueItem[];
  /** Index into `queue` of the currently-playing entry. */
  index: number;
  isPlaying: boolean;
  currentTime: number;
  duration: number;
  volume: number; // 0..100
  playMode: PlayMode;
  error: string | null;

  /** Enqueue tracks and immediately try to play the first. */
  enqueueAndPlay: (tracks: MetadataTrack[]) => Promise<void>;
  /** Append tracks to the queue. If nothing is playing yet, start playback. */
  enqueue: (tracks: MetadataTrack[]) => Promise<void>;
  /** Replace the queue and play. */
  playNow: (info: PlaybackInfo) => void;
  /** Resolve + play a single metadata track (uses `coordinatorPlay`). */
  resolveAndPlay: (track: MetadataTrack) => Promise<void>;
  /** Set the active index to `i` and start playback at that position.
   *  Unlike `resolveAndPlay`, this **preserves the entire queue** — it only
   *  updates the resolved status of the targeted entry in place. Use this for
   *  any in-queue navigation (jumpTo / next / prev / auto-advance after
   *  removeAt). Use `resolveAndPlay` for the "play this one track fresh,
   *  replacing the queue" semantic. */
  playAt: (i: number) => Promise<void>;
  toggle: () => void;
  seek: (t: number) => void;
  setVolume: (v: number) => void;
  next: () => void;
  prev: () => void;
  /** Toggle play mode between list-loop and shuffle. */
  togglePlayMode: () => void;
  /** Re-resolve the persisted current track after a reload (paused). */
  restore: () => Promise<void>;
  /** Jump to a specific queue index and play it (queue drawer click). */
  jumpTo: (i: number) => void;
  /** Remove the item at `i`. Keeps `index` valid: removing the currently
   *  playing track auto-advances to its successor, or stops playback if it
   *  was the last/only entry; removing a prior entry shifts `index` down by
   *  one so the same track stays current. */
  removeAt: (i: number) => void;
  /** Reorder the queue via drag-and-drop (indices into `queue`). */
  moveItem: (from: number, to: number) => void;
  /** Empty the queue entirely and stop playback. */
  clearQueue: () => void;
  clearError: () => void;
}

let audio: HTMLAudioElement | null = null;

function getAudio(): HTMLAudioElement {
  if (!audio) {
    audio = new Audio();
    audio.preload = "metadata";

    audio.addEventListener("timeupdate", () => {
      const cur = audio!.currentTime;
      usePlayer.setState({ currentTime: cur });
      updatePositionState(cur, audio!.duration || 0);
    });
    audio.addEventListener("durationchange", () => {
      usePlayer.setState({ duration: audio!.duration || 0 });
    });
    audio.addEventListener("play", () => usePlayer.setState({ isPlaying: true }));
    audio.addEventListener("pause", () => usePlayer.setState({ isPlaying: false }));
    audio.addEventListener("ended", () => usePlayer.getState().next());
    audio.addEventListener("error", () => {
      // HTML5 MediaError codes: 1=ABORTED, 2=NETWORK, 3=DECODE, 4=SRC_NOT_SUPPORTED.
      const err = audio!.error;
      const code = err?.code ?? 0;
      const reasonKey =
        code === 1 ? "player.errAborted" :
        code === 2 ? "player.errNetwork" :
        code === 3 ? "player.errDecode" :
        code === 4 ? "player.errUnsupported" :
        "player.errUnknown";
      const reason = i18n.t(reasonKey);
      // Surface the real diagnostic in the dev console — much more useful
      // than the user-facing message for triaging CDN-side issues.
      console.error("[player] audio error", {
        code,
        mediaErrorMessage: err?.message,
        networkState: audio!.networkState,
        readyState: audio!.readyState,
        src: audio!.src,
      });
      usePlayer.setState({
        isPlaying: false,
        error: i18n.t("player.playFailedFull", { reason, code }),
      });
    });
  }
  return audio;
}

function updateMediaSession(info: PlaybackInfo) {
  if (!("mediaSession" in navigator)) return;
  try {
    const t = info.track;
    navigator.mediaSession.metadata = new MediaMetadata({
      title: info.stream.title || t.title,
      artist: info.stream.artist || t.artist,
      album: t.album || "Bilusic",
      artwork: (info.stream.cover || t.cover) ? [{ src: info.stream.cover || t.cover }] : [],
    });
    navigator.mediaSession.setActionHandler("play", () => usePlayer.getState().toggle());
    navigator.mediaSession.setActionHandler("pause", () => usePlayer.getState().toggle());
    navigator.mediaSession.setActionHandler("previoustrack", () => usePlayer.getState().prev());
    navigator.mediaSession.setActionHandler("nexttrack", () => usePlayer.getState().next());
  } catch {
    /* ignore unsupported action handlers */
  }
}

function updatePositionState(position: number, duration: number) {
  if (!("mediaSession" in navigator) || !("setPositionState" in navigator.mediaSession)) return;
  if (duration > 0) {
    try {
      navigator.mediaSession.setPositionState({ duration, position, playbackRate: 1 });
    } catch {
      /* ignore */
    }
  }
}

async function resolveOne(track: MetadataTrack): Promise<PlaybackInfo> {
  return coordinatorPlay(track);
}

export const usePlayer = create<PlayerState>()(
  persist(
    (set, get) => ({
      current: null,
      queue: [],
      index: -1,
      isPlaying: false,
      currentTime: 0,
      duration: 0,
      volume: 72,
      playMode: "list", // 默认列表循环
      error: null,

      enqueueAndPlay: async (tracks) => {
        if (tracks.length === 0) return;
        const queue: QueueItem[] = tracks.map((t) => ({ track: t, resolved: null }));
        set({ queue, index: 0, error: null });

        // Resolve the first track then commit.
        try {
          const resolved = await resolveOne(tracks[0]);
          queue[0].resolved = resolved;
          set({ queue: [...queue] });
          get().playNow(resolved);
        } catch (e: unknown) {
          set({ error: i18n.t("player.resolveFailed", { msg: extractErrorMessage(e) }) });
        }
      },

      enqueue: async (tracks) => {
        if (tracks.length === 0) return;
        const appended: QueueItem[] = tracks.map((t) => ({ track: t, resolved: null }));
        const wasIdle = get().index < 0;
        const prevLen = get().queue.length;
        set({ queue: [...get().queue, ...appended], error: null });
        if (wasIdle) {
          set({ index: prevLen });
          try {
            const resolved = await resolveOne(appended[0].track);
            get().playNow(resolved);
          } catch (e: unknown) {
            set({ error: i18n.t("player.resolveFailed", { msg: extractErrorMessage(e) }) });
          }
        }
      },

      resolveAndPlay: async (track) => {
        set({ queue: [{ track, resolved: null }], index: 0, error: null });
        try {
          const resolved = await resolveOne(track);
          get().playNow(resolved);
        } catch (e: unknown) {
          set({ error: i18n.t("player.resolveFailed", { msg: extractErrorMessage(e) }) });
        }
      },

      playAt: async (i) => {
        const { queue } = get();
        if (i < 0 || i >= queue.length) return;
        set({ index: i, error: null });
        const e = queue[i];
        if (!e) return;
        if (e.resolved) {
          get().playNow(e.resolved);
          return;
        }
        // Resolve in place — DO NOT touch the rest of the queue.
        try {
          const resolved = await resolveOne(e.track);
          // Patch the resolved status back in. Guard against the user having
          // reordered / removed this entry while we were awaiting the network.
          const q = get().queue;
          if (q[i] && q[i].track === e.track) {
            q[i] = { track: e.track, resolved };
            set({ queue: [...q] });
          }
          // Only commit playback if the user hasn't navigated elsewhere.
          if (get().index === i) {
            get().playNow(resolved);
          }
        } catch (err: unknown) {
          set({ error: i18n.t("player.resolveFailed", { msg: extractErrorMessage(err) }) });
        }
      },

      playNow: (info) => {
        const a = getAudio();
        a.src = proxyStreamUrl(info.stream.url, info.stream.mime);
        a.volume = get().volume / 100;
        set({
          current: info,
          currentTime: 0,
          duration: 0,
          error: null,
        });
        a.play().catch(() => {
          set({ error: i18n.t("player.blocked") });
        });
        updateMediaSession(info);
        // Record into play history (fire-and-forget; do not block playback).
        void historyAdd(info.track).catch(() => {});
        // Pre-resolve the next entry in the background so next-track is snappy.
        const { queue, index } = get();
        const nextEntry = queue[index + 1];
        if (nextEntry && !nextEntry.resolved) {
          resolveOne(nextEntry.track)
            .then((resolved) => {
              const q = get().queue;
              const ni = get().index + 1;
              if (q[ni]) {
                q[ni].resolved = resolved;
                set({ queue: [...q] });
              }
            })
            .catch(() => {});
        }
      },

      toggle: () => {
        const a = getAudio();
        if (a.paused) {
          a.play().catch(() => set({ error: i18n.t("player.playRetry") }));
        } else {
          a.pause();
        }
      },

      seek: (t) => {
        const a = getAudio();
        if (a) a.currentTime = t;
        set({ currentTime: t });
      },

      setVolume: (v) => {
        const a = getAudio();
        if (a) a.volume = v / 100;
        set({ volume: v });
      },

      next: () => {
        const { queue, index, playMode } = get();
        if (queue.length === 0) {
          set({ isPlaying: false });
          return;
        }
        let ni: number;
        if (queue.length === 1) {
          // 单首：原地重播（列表循环 / 随机都无其它可选）
          ni = index;
        } else if (playMode === "shuffle") {
          // 随机播放：选一个不同于当前的随机位置
          do {
            ni = Math.floor(Math.random() * queue.length);
          } while (ni === index);
        } else {
          // 列表循环：顺序前进，到末尾回到开头
          ni = index + 1;
          if (ni >= queue.length) ni = 0;
        }
        // playAt keeps the entire queue intact and just sets index + plays.
        void get().playAt(ni);
      },

      prev: () => {
        const { queue, index, currentTime, playMode } = get();
        const a = getAudio();
        // Restart current track if we're past 3s (matches common players).
        if (a && currentTime > 3) {
          a.currentTime = 0;
          set({ currentTime: 0 });
          return;
        }
        if (queue.length === 0) return;
        let ni: number;
        if (queue.length === 1) {
          ni = index; // 重播当前
        } else if (playMode === "shuffle") {
          // 随机播放：上一首也走随机
          do {
            ni = Math.floor(Math.random() * queue.length);
          } while (ni === index);
        } else {
          // 列表循环：顺序后退，到开头回到末尾
          ni = index - 1;
          if (ni < 0) ni = queue.length - 1;
        }
        void get().playAt(ni);
      },

      togglePlayMode: () => {
        const cur = get().playMode;
        set({ playMode: cur === "list" ? "shuffle" : "list" });
      },

      restore: async () => {
        const { queue, index } = get();
        if (index < 0 || index >= queue.length) return;
        const entry = queue[index];
        if (!entry) return;
        try {
          const resolved = await resolveOne(entry.track);
          const a = getAudio();
          a.src = proxyStreamUrl(resolved.stream.url, resolved.stream.mime);
          a.volume = get().volume / 100;
          set({ current: resolved, currentTime: 0, duration: 0, error: null, isPlaying: false });
          a.pause(); // restored paused — let the user hit play
          updateMediaSession(resolved);
        } catch (e: unknown) {
          set({ error: i18n.t("player.restoreFailed", { msg: extractErrorMessage(e) }) });
        }
      },

      jumpTo: (i) => {
        // playAt keeps the entire queue intact and just sets index + plays.
        void get().playAt(i);
      },

      removeAt: (i) => {
        const state = get();
        const queue = state.queue;
        if (i < 0 || i >= queue.length) return;
        const wasCurrent = i === state.index;
        const newQueue = queue.filter((_, idx) => idx !== i);
        let newIndex: number;
        if (wasCurrent) {
          // Removing the last/only entry → nothing left to play.
          // Otherwise the successor slides into this slot.
          newIndex = state.index >= queue.length - 1 ? -1 : state.index;
        } else if (i < state.index) {
          newIndex = state.index - 1;
        } else {
          newIndex = state.index;
        }
        set({ queue: newQueue, index: newIndex });
        if (wasCurrent) {
          if (newIndex >= 0 && newQueue[newIndex]) {
            // Auto-advance: play the successor in place (keeps the rest of
            // the queue intact — must NOT use resolveAndPlay which would
            // collapse the queue to a single entry).
            void get().playAt(newIndex);
          } else {
            const a = getAudio();
            a.pause();
            set({ current: null, isPlaying: false, error: null });
          }
        }
      },

      moveItem: (from, to) => {
        const state = get();
        const queue = state.queue;
        if (
          from < 0 || from >= queue.length ||
          to < 0 || to >= queue.length ||
          from === to
        ) return;
        const cur = state.index;
        const moved = queue[from];
        const newQueue = queue.slice();
        newQueue.splice(from, 1);
        newQueue.splice(to, 0, moved);
        let newIndex = cur;
        if (cur === from) newIndex = to;
        else if (from < cur && to >= cur) newIndex = cur - 1;
        else if (from > cur && to <= cur) newIndex = cur + 1;
        set({ queue: newQueue, index: newIndex });
      },

      clearQueue: () => {
        set({ queue: [], index: -1 });
        const a = getAudio();
        a.pause();
        set({ current: null, isPlaying: false, error: null });
      },

      clearError: () => set({ error: null }),
    }),
    {
      name: "bilusic-player",
      // Persist queue (tracks only), index, play mode, volume.
      // Resolved stream URLs are intentionally dropped — they expire.
      partialize: (s) => ({
        queue: s.queue.map((q) => ({ track: q.track, resolved: null })),
        index: s.index,
        playMode: s.playMode,
        volume: s.volume,
      }),
    }
  )
);
