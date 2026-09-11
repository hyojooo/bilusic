import { invoke } from "@tauri-apps/api/core";
import i18n from "@/i18n";

export interface PluginInfo {
  id: string;
  name: string;
  version: string;
  description: string;
  capabilities: number;
  /** Optional role (`"metadata"` / `"engine"`). Propagated by `toPluginInfo`
   *  from `PluginListItem` so the UI can pick a (id, kind)-namespaced i18n key
   *  for the description; the backend's `StateSnapshot` shape doesn't include
   *  it (metadata_sources / audio_engines are split by role). The field is
   *  optional for callers that hold a pure `PluginInfo` without role context. */
  kind?: string;
}

export interface MetadataTrack {
  source_id: string;
  source: string;
  engine_hint: string | null;
  title: string;
  artist: string;
  album: string;
  duration_ms: number;
  cover: string;
  lyrics_id: string | null;
}

export interface PlaybackInfo {
  track: MetadataTrack;
  stream: {
    url: string;
    mime: string;
    bitrate: number;
    duration_ms: number;
    cover: string;
    title: string;
    artist: string;
  };
  /**
   * Identifier of the audio source on the active engine (e.g. a Bilibili
   * `bvid`). Lets the UI highlight "this is what we're already playing"
   * inside the player bar's 其他音源 picker so a re-open of the picker
   * remembers the selection. `undefined` / `null` when the engine has no
   * stable id for the resolved stream.
   */
  bvid?: string | null;
  engine: string;
}

/** A parsed lyric line (from a synced LRC or a plain fallback). */
export interface LyricLine {
  time_ms: number;
  text: string;
}

/** A candidate audio source returned by an engine's `search` (before stream
 *  resolution). Used by the player bar's "其他音源" picker. */
export interface AudioCandidate {
  bvid: string;
  title: string;
  author: string;
  duration_sec: number;
  cover: string;
}

export interface Lyrics {
  lines: LyricLine[];
}

export interface StateSnapshot {
  metadata_sources: PluginInfo[];
  audio_engines: PluginInfo[];
  active_metadata: string;
  active_engine: string;
}

/** A row in the Settings → Plugins management list. */
export interface PluginListItem {
  id: string;
  kind: string; // "metadata" | "engine" | "lyrics"
  name: string;
  version: string;
  description: string;
  capabilities: number;
  enabled: boolean;
  order: number;
  stub: boolean; // discovered manifest w/o compiled adapter (runtime slot pending)
  builtin: boolean; // shipped Rust trait — cannot be uninstalled
  removable: boolean; // lives under user plugin dir — may be uninstalled
}

/** A single entry inside a home-page section. kind drives the renderer:
 *  "song" → playable song card, "playlist"/"album"/"artist" → browse card. */
export interface SectionItem {
  id: string;
  title: string;
  subtitle: string | null;
  cover: string;
  kind: string; // "song" | "playlist" | "album" | "artist"
  source: string;
  artist: string | null; // present only when kind === "song"
  duration_ms: number | null;
  engine_hint: string | null;
}

/** A home-page section declared by a metadata plugin. The plugin supplies the
 *  real title and the item kind; the UI renders sections generically. */
export interface HomeSection {
  id: string;
  title: string;
  kind: string; // "song" | "card" — renderer hint
  items: SectionItem[];
  hint: string | null;
}

/** A playable song returned by toplist / search. */
export interface FeedSong {
  source_id: string;
  title: string;
  artist: string;
  cover: string;
  duration_ms: number;
  source:  string;
  engine_hint: string | null;
}

/** The home / discover feed returned by a metadata source as an ordered list
 *  of sections declared by the plugin. */
export interface HomeFeed {
  sections: HomeSection[];
}

// ─── P3: playlists / history (local SQLite) ──────────────────────────────

export interface PlaylistSummary {
  id: string;
  name: string;
  cover: string | null;
  track_count: number;
  updated_at: number;
}

export interface PlaylistTrack {
  position: number;
  track: MetadataTrack;
}

/** Base URL of the local streaming proxy (see src-tauri/src/proxy.rs). */
export const PROXY_BASE = "http://127.0.0.1:9527";

/** Wrap a direct CDN URL behind the local proxy. */
export function proxyStreamUrl(
  streamUrl: string,
  mime?: string
): string {
  const params = new URLSearchParams({ u: streamUrl });
  if (mime) params.set("mime", mime);
  return `${PROXY_BASE}/stream?${params.toString()}`;
}

// ─── Plugin discovery & selection ────────────────────────────────────────

export async function listPlugins(): Promise<StateSnapshot> {
  return invoke<StateSnapshot>("list_plugins");
}

export async function setActiveMetadata(id: string): Promise<void> {
  return invoke<void>("set_active_metadata", { id });
}

export async function setActiveEngine(id: string): Promise<void> {
  return invoke<void>("set_active_engine", { id });
}

export async function setBilibiliCookie(cookie: string): Promise<void> {
  return invoke<void>("set_bilibili_cookie", { cookie: cookie || null });
}

// ─── Plugin management (Settings → Plugins) ──────────────────────────────

/** Full inventory: built-ins + discovered third-party manifests. */
export async function pluginList(): Promise<PluginListItem[]> {
  return invoke<PluginListItem[]>("plugin_list");
}

export async function pluginToggle(
  kind: string,
  id: string,
  enabled: boolean
): Promise<void> {
  return invoke<void>("plugin_toggle", { kind, id, enabled });
}

export async function pluginReorder(
  ordered: [string, string][]
): Promise<void> {
  return invoke<void>("plugin_reorder", { ordered });
}

export async function pluginUninstall(
  kind: string,
  id: string
): Promise<void> {
  return invoke<void>("plugin_uninstall", { kind, id });
}

/** Import a third-party plugin from a user-picked path (plugin.json or folder). */
export async function pluginImport(path: string): Promise<string> {
  return invoke<string>("plugin_import", { path });
}

// ─── Coordinator: metadata → engine ──────────────────────────────────────

export async function metadataSearch(
  query: string,
  page = 1
): Promise<MetadataTrack[]> {
  return invoke<MetadataTrack[]>("metadata_search", { query, page });
}

/** Fetch the home / discover feed for the given metadata source.
 *
 *  ALWAYS pass the front-end's own `metadataSource` here (the same value
 *  used to key the localStorage cache). The backend will resolve the feed
 *  via THIS id rather than its currently-active selection, so the response
 *  is guaranteed to come from the source the user actually selected —
 *  even if the backend's `active_metadata` is still on the cold-boot
 *  fallback (first enabled source, usually `qq-music`). Without this pin,
 *  the launch race would let a default-Q response get
 *  `cache.putFeed`'d into `bilusic.cache.home_feed.<user's saved source>`
 *  and surface as "wrong source content" for the full 30min TTL. */
export async function metadataHome(source: string): Promise<HomeFeed> {
  return invoke<HomeFeed>("metadata_home", { source });
}

/** Fetch a named toplist / chart (by provider-specific id) from the active
 *  metadata source — used for region charts (Billboard / Melon / UK / Oricon /
 *  Douyin…). Returns real chart songs with covers. */
export async function metadataToplist(topid: number): Promise<MetadataTrack[]> {
  return invoke<MetadataTrack[]>("metadata_toplist", { topid });
}

/** Fetch the track list of a third-party playlist (e.g. a QQ / 网易云
 *  recommended playlist from the home feed) by its source-side id. The
 *  `source` MUST be passed from the front-end — same launch-race fix as
 *  `metadataHome` (N11z): the playlist id alone is ambiguous across sources
 *  (QQ's `disstid=123` vs 网易云's `id=123` are completely different
 *  playlists), so the source is pinned by the front-end to avoid landing on
 *  the wrong one when the backend's `active_metadata` is still on its
 *  boot-time fallback. */
export async function metadataGetPlaylist(
  source: string,
  id: string
): Promise<FeedSong[]> {
  return invoke<FeedSong[]>("metadata_playlist", { source, id });
}

export async function coordinatorPlay(
  track: MetadataTrack
): Promise<PlaybackInfo> {
  return invoke<PlaybackInfo>("coordinator_play", { track });
}

/** Skip metadata entirely: paste a bvid, hit the active audio engine directly. */
export async function playByBvid(bvid: string): Promise<PlaybackInfo> {
  return invoke<PlaybackInfo>("play_by_bvid", { bvid });
}

/** List candidate audio sources for a free-text query on the active engine.
 *  Feeds the player bar's "其他音源" picker; the chosen candidate is resolved
 *  via `playByBvid` while keeping the current song's metadata. */
export async function audioSearch(query: string): Promise<AudioCandidate[]> {
  return invoke<AudioCandidate[]>("audio_search", { query });
}

/**
 * Fetch lyrics from a specific metadata source (empty `sourceId` falls back to
 * the active source). Lyrics-only sources such as `lrclib` are reachable here
 * even though they are not selectable as the active search source.
 */
export async function metadataGetLyrics(
  sourceId: string,
  track: MetadataTrack
): Promise<Lyrics> {
  return invoke<Lyrics>("metadata_get_lyrics", { sourceId, track });
}

// ─── Error formatting ────────────────────────────────────────────────────

/**
 * Extract a human-readable message from whatever the `invoke` promise rejected
 * with. Tauri 2 serializes Rust errors to plain objects (e.g.
 * `{ code: 500, message: "..." }`), so `String(e)` would yield `[object Object]`
 * — that's the bug the cross-source path used to show.
 */
export function extractErrorMessage(e: unknown): string {
  if (e == null) return i18n.t("common.unknownError");
  if (typeof e === "string") return e;
  if (e instanceof Error) {
    // Tauri often wraps the payload in an Error whose `.message` is itself the
    // serialized JSON; prefer `message` but fall back to inspecting it.
    if (e.message && e.message !== "[object Object]") return e.message;
    if (typeof (e as Error & { data?: unknown }).data === "string") {
      return (e as Error & { data: string }).data;
    }
  }
  if (typeof e === "object") {
    const o = e as Record<string, unknown>;
    if (typeof o.message === "string" && o.message) return o.message;
    if (typeof o.msg === "string" && o.msg) return o.msg;
    if (typeof o.error === "string" && o.error) return o.error;
    try {
      return JSON.stringify(e);
    } catch {
      return "[object Object]";
    }
  }
  return String(e);
}

// ─── Playlists ───────────────────────────────────────────────────────────

export async function playlistCreate(
  name: string,
  cover?: string | null
): Promise<PlaylistSummary> {
  return invoke<PlaylistSummary>("playlist_create", { name, cover: cover ?? null });
}

export async function playlistList(): Promise<PlaylistSummary[]> {
  return invoke<PlaylistSummary[]>("playlist_list");
}

export async function playlistRename(id: string, name: string): Promise<void> {
  return invoke<void>("playlist_rename", { id, name });
}

export async function playlistDelete(id: string): Promise<number> {
  return invoke<number>("playlist_delete", { id });
}

export async function playlistAddTrack(
  playlistId: string,
  track: MetadataTrack
): Promise<void> {
  return invoke<void>("playlist_add_track", { playlistId, track });
}

export async function playlistRemoveTrack(
  playlistId: string,
  trackId: string
): Promise<void> {
  return invoke<void>("playlist_remove_track", { playlistId, trackId });
}

export async function playlistTracks(playlistId: string): Promise<PlaylistTrack[]> {
  return invoke<PlaylistTrack[]>("playlist_tracks", { playlistId });
}

export async function playlistReorder(
  playlistId: string,
  orderedIds: string[]
): Promise<void> {
  return invoke<void>("playlist_reorder", { playlistId, orderedIds });
}

// ─── Play history ──────────────────────────────────────────────────────────

export async function historyAdd(track: MetadataTrack): Promise<void> {
  return invoke<void>("history_add", { track });
}

export async function historyList(limit = 50): Promise<MetadataTrack[]> {
  return invoke<MetadataTrack[]>("history_list", { limit });
}

// ─── Cache & local-data maintenance ─────────────────────────────────────

/** Result of a manual prune: orphan rows removed + bytes reclaimed by VACUUM. */
export interface CachePruneResult {
  removed: number;
  freed_bytes: number;
}

/**
 * Remove `tracks_cache` rows no longer referenced by any playlist/history,
 * then `VACUUM` so the `.db` file actually shrinks on disk.
 */
export async function cachePrune(): Promise<CachePruneResult> {
  return invoke<CachePruneResult>("cache_prune");
}

/** Wipe all local data (playlists, history, cache). Destructive. */
export async function cacheReset(): Promise<void> {
  return invoke<void>("cache_reset");
}

/** Compact the database file without deleting any data. Returns bytes freed. */
export async function cacheOptimize(): Promise<number> {
  return invoke<number>("cache_optimize");
}

/** Clear play history only; orphaned cache rows are pruned as a side effect. */
export async function historyClear(): Promise<number> {
  return invoke<number>("history_clear");
}

/** Health snapshot of the track cache (size / staleness). */
export interface CacheStats {
  rows: number;
  referenced: number;
  stale: number;
  oldest_age_ms: number;
}

/** Re-fetch metadata for every cached track and re-upsert (refresh covers). */
export async function cacheRefresh(): Promise<number> {
  return invoke<number>("cache_refresh");
}

/** Read cache health stats for the Settings page. */
export async function cacheStats(): Promise<CacheStats> {
  return invoke<CacheStats>("cache_stats");
}
