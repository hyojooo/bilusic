import { create } from "zustand";
import i18n from "@/i18n";
import {
  playlistCreate,
  playlistList,
  playlistRename,
  playlistDelete,
  playlistAddTrack,
  playlistRemoveTrack,
  playlistTracks,
  playlistReorder,
  type PlaylistSummary,
  type PlaylistTrack,
  type MetadataTrack,
} from "@/lib/bili";

/**
 * Stable cache id for a track — must match `Database::cache_key` in Rust
 * (`<source>:<source_id>`). Used as the `trackId` argument for remove/reorder.
 */
export function trackCacheId(track: MetadataTrack): string {
  return `${track.source}:${track.source_id}`;
}

interface PlaylistsState {
  playlists: PlaylistSummary[];
  /** Tracks keyed by playlist id (loaded lazily). */
  tracksById: Record<string, PlaylistTrack[]>;
  loading: boolean;
  error: string | null;

  loadPlaylists: () => Promise<void>;
  createPlaylist: (name: string) => Promise<PlaylistSummary>;
  renamePlaylist: (id: string, name: string) => Promise<void>;
  deletePlaylist: (id: string) => Promise<void>;
  getTracks: (id: string) => Promise<PlaylistTrack[]>;
  addTrack: (playlistId: string, track: MetadataTrack) => Promise<void>;
  removeTrack: (playlistId: string, trackId: string) => Promise<void>;
  reorder: (playlistId: string, orderedIds: string[]) => Promise<void>;
  clearError: () => void;
}

export const usePlaylists = create<PlaylistsState>((set, get) => ({
  playlists: [],
  tracksById: {},
  loading: false,
  error: null,

  loadPlaylists: async () => {
    set({ loading: true, error: null });
    try {
      const playlists = await playlistList();
      set({ playlists, loading: false });
    } catch (e: unknown) {
      set({ error: i18n.t("playlists.loadFailed", { msg: errMsg(e) }), loading: false });
    }
  },

  createPlaylist: async (name) => {
    set({ error: null });
    const pl = await playlistCreate(name);
    set({ playlists: [pl, ...get().playlists] });
    return pl;
  },

  renamePlaylist: async (id, name) => {
    await playlistRename(id, name);
    set({
      playlists: get().playlists.map((p) => (p.id === id ? { ...p, name } : p)),
    });
  },

  deletePlaylist: async (id) => {
    await playlistDelete(id);
    const { tracksById } = get();
    const next = { ...tracksById };
    delete next[id];
    set({
      playlists: get().playlists.filter((p) => p.id !== id),
      tracksById: next,
    });
  },

  getTracks: async (id) => {
    const tracks = await playlistTracks(id);
    set({ tracksById: { ...get().tracksById, [id]: tracks } });
    return tracks;
  },

  addTrack: async (playlistId, track) => {
    await playlistAddTrack(playlistId, track);
    // Refresh the cached list for that playlist + bump the count.
    await get().getTracks(playlistId);
    set({
      playlists: get().playlists.map((p) =>
        p.id === playlistId ? { ...p, track_count: p.track_count + 1 } : p
      ),
    });
  },

  removeTrack: async (playlistId, trackId) => {
    await playlistRemoveTrack(playlistId, trackId);
    await get().getTracks(playlistId);
    set({
      playlists: get().playlists.map((p) =>
        p.id === playlistId
          ? { ...p, track_count: Math.max(0, p.track_count - 1) }
          : p
      ),
    });
  },

  reorder: async (playlistId, orderedIds) => {
    await playlistReorder(playlistId, orderedIds);
    await get().getTracks(playlistId);
  },

  clearError: () => set({ error: null }),
}));

function errMsg(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  try {
    return JSON.stringify(e);
  } catch {
    return i18n.t("common.unknownError");
  }
}
