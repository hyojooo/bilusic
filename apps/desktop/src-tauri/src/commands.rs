//! Tauri command surface (decoupled metadata/architecture).
//!
//! Commands are thin: they delegate to the [`Coordinator`], which is the only
//! component that knows the user's current `(metadata source, audio engine)`
//! pairing.

use crate::coordinator::{Coordinator, PlaybackInfo};
use crate::engine::AudioCandidate;
use crate::error::AppError;
use crate::metadata::{FeedSong, HomeFeed, Lyrics, MetadataTrack};
use crate::plugin::PluginInfo;
use crate::plugin_host::PluginListItem;
use crate::proxy;
use crate::PluginCookieService;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

#[derive(Debug, Clone, Serialize)]
pub struct StateSnapshot {
    pub metadata_sources: Vec<PluginInfo>,
    pub audio_engines: Vec<PluginInfo>,
    pub active_metadata: String,
    pub active_engine: String,
}

#[tauri::command]
pub fn list_plugins(coordinator: State<'_, Coordinator>) -> StateSnapshot {
    StateSnapshot {
        metadata_sources: coordinator.list_metadata_sources(),
        audio_engines: coordinator.list_audio_engines(),
        active_metadata: coordinator.active_metadata.read().unwrap().clone(),
        active_engine: coordinator.active_engine.read().unwrap().clone(),
    }
}

/// Full plugin inventory for the Settings → Plugins management page: built-ins
/// plus discovered third-party manifests, with enable / order / stub / removable
/// flags. Unlike `list_plugins` (which only returns enabled, implemented
/// sources/engines), this lists everything so the UI can toggle and reorder.
#[tauri::command]
pub fn plugin_list(coordinator: State<'_, Coordinator>) -> Vec<PluginListItem> {
    coordinator.host.all_plugins()
}

/// Enable / disable a plugin. Persisted to `app_data_dir/plugin_state.json`.
#[tauri::command]
pub async fn plugin_toggle(
    kind: String,
    id: String,
    enabled: bool,
    coordinator: State<'_, Coordinator>,
) -> Result<(), AppError> {
    coordinator.host.set_enabled(&kind, &id, enabled);
    Ok(())
}

/// Reorder the plugin list. `ordered` is the full visible list as `(kind, id)`
/// pairs in the new order; sequential order values are assigned.
#[tauri::command]
pub async fn plugin_reorder(
    ordered: Vec<(String, String)>,
    coordinator: State<'_, Coordinator>,
) -> Result<(), AppError> {
    coordinator.host.set_order(&ordered);
    Ok(())
}

/// Uninstall a third-party plugin (delete its folder). Built-in / bundled
/// plugins are rejected.
#[tauri::command]
pub async fn plugin_uninstall(
    kind: String,
    id: String,
    coordinator: State<'_, Coordinator>,
) -> Result<(), AppError> {
    coordinator.host.uninstall(&kind, &id)
}

/// Import a third-party plugin from a user-picked path (a `plugin.json` file
/// or its containing folder). Copies it into the user plugin dir and rescans.
/// Returns the imported plugin `id`.
#[tauri::command]
pub async fn plugin_import(
    path: String,
    coordinator: State<'_, Coordinator>,
) -> Result<String, AppError> {
    coordinator.host.import(std::path::Path::new(&path))
}

#[tauri::command]
pub async fn set_active_metadata(
    id: String,
    coordinator: State<'_, Coordinator>,
) -> Result<(), AppError> {
    coordinator.set_active_metadata(&id);
    Ok(())
}

#[tauri::command]
pub async fn set_active_engine(
    id: String,
    coordinator: State<'_, Coordinator>,
) -> Result<(), AppError> {
    coordinator.set_active_engine(&id);
    Ok(())
}

#[tauri::command]
pub async fn metadata_search(
    query: String,
    page: Option<u32>,
    coordinator: State<'_, Coordinator>,
) -> Result<Vec<MetadataTrack>, AppError> {
    coordinator.search(&query, page.unwrap_or(1)).await
}

#[tauri::command]
pub async fn metadata_playlist(
    source: String,
    id: String,
    coordinator: State<'_, Coordinator>,
) -> Result<Vec<FeedSong>, AppError> {
    coordinator.get_playlist(Some(&source), &id).await
}

#[tauri::command]
pub async fn metadata_home(
    // Optional explicit source override from the front-end. When present,
    // the coordinator will fetch the home feed via THIS source rather than
    // its currently-active selection. This avoids the launch race where
    // `active_metadata` is still on the backend's boot-time fallback
    // (first enabled source, usually `qq-music`) while the front-end has
    // already rehydrated `metadataSource: 'kuwo'` from disk. With this
    // override the response is guaranteed to come from the source the
    // user actually selected, so the front-end's
    // `cache.putFeed(...)` (keyed by the front-end `metadataSource`) can
    // never write a different plugin's feed into the new source's slot.
    source: Option<String>,
    coordinator: State<'_, Coordinator>,
) -> Result<HomeFeed, AppError> {
    coordinator.home_feed(source.as_deref()).await
}

/// Fetch a named toplist / chart (by provider-specific id) from the active
/// metadata source. Used by the home page's region charts (Billboard / Melon /
/// UK / Oricon / Douyin…). Sources without a chart endpoint return empty.
#[tauri::command]
pub async fn metadata_toplist(
    topid: i32,
    coordinator: State<'_, Coordinator>,
) -> Result<Vec<FeedSong>, AppError> {
    coordinator.toplist(topid).await
}

/// Fetch lyrics from a specific metadata source (empty `source_id` falls back
/// to the active source). Lyrics-only sources such as `lrclib` are reachable
/// here even though they are not selectable as the active search source.
#[tauri::command]
pub async fn metadata_get_lyrics(
    source_id: String,
    track: MetadataTrack,
    coordinator: State<'_, Coordinator>,
) -> Result<Lyrics, AppError> {
    coordinator.get_lyrics(&source_id, &track).await
}

#[tauri::command]
pub async fn coordinator_play(
    track: MetadataTrack,
    coordinator: State<'_, Coordinator>,
) -> Result<PlaybackInfo, AppError> {
    coordinator.play(&track).await
}

/// Direct bvid → playback escape hatch. Skips the metadata plugin entirely:
/// the user typed a bvid, so we ask the active audio engine to resolve it.
/// Useful both as a debug path and as the v_voucher bypass.
#[tauri::command]
pub async fn play_by_bvid(
    bvid: String,
    coordinator: State<'_, Coordinator>,
) -> Result<PlaybackInfo, AppError> {
    let engine_id = coordinator.active_engine.read().unwrap().clone();
    let engines = coordinator.list_audio_engines();
    if engines.iter().all(|e| e.id != engine_id) {
        return Err(AppError::msg("未选择音频引擎"));
    }
    let track = MetadataTrack {
        source_id: bvid,
        source: engine_id.to_string(),
        engine_hint: Some(engine_id.to_string()),
        title: String::new(),
        artist: String::new(),
        album: String::new(),
        duration_ms: 0,
        cover: String::new(),
        lyrics_id: None,
    };
    coordinator.play(&track).await
}

/// List candidate audio sources for a free-text query on the active engine.
/// Feeds the player bar's "其他音源" picker — the frontend resolves the
/// chosen candidate back through [`play_by_bvid`] while keeping the current
/// song's metadata.
#[tauri::command]
pub async fn audio_search(
    query: String,
    coordinator: State<'_, Coordinator>,
) -> Result<Vec<AudioCandidate>, AppError> {
    coordinator.audio_search(&query).await
}

#[tauri::command]
pub fn proxy_base_url() -> String {
    proxy::PROXY_BASE.to_string()
}

/// Push the user's SESSDATA (or any cookie blob) into both Bili plugins.
#[tauri::command]
pub async fn set_bilibili_cookie(
    cookie: Option<String>,
    service: State<'_, PluginCookieService>,
) -> Result<(), AppError> {
    service
        .bilibili_metadata
        .set_cookie(cookie.clone())
        .await;
    service.bilibili_engine.set_cookie(cookie).await;
    Ok(())
}

// --- P3: playlists, track cache, play history ----------------------------

#[tauri::command]
pub fn playlist_create(
    name: String,
    cover: Option<String>,
    db: State<'_, crate::db::Database>,
) -> Result<crate::db::PlaylistSummary, AppError> {
    db.create_playlist(&name, cover.as_deref())
}

#[tauri::command]
pub fn playlist_list(db: State<'_, crate::db::Database>) -> Result<Vec<crate::db::PlaylistSummary>, AppError> {
    db.list_playlists()
}

#[tauri::command]
pub fn playlist_rename(id: String, name: String, db: State<'_, crate::db::Database>) -> Result<(), AppError> {
    db.rename_playlist(&id, &name)
}

#[tauri::command]
pub fn playlist_delete(id: String, db: State<'_, crate::db::Database>) -> Result<usize, AppError> {
    db.delete_playlist(&id)
}

#[tauri::command]
pub fn playlist_add_track(
    playlist_id: String,
    track: MetadataTrack,
    db: State<'_, crate::db::Database>,
) -> Result<(), AppError> {
    db.add_track(&playlist_id, &track)
}

#[tauri::command]
pub fn playlist_remove_track(
    playlist_id: String,
    track_id: String,
    db: State<'_, crate::db::Database>,
) -> Result<(), AppError> {
    db.remove_track(&playlist_id, &track_id).map(|_| ())
}

#[tauri::command]
pub fn playlist_tracks(
    playlist_id: String,
    db: State<'_, crate::db::Database>,
) -> Result<Vec<crate::db::PlaylistTrack>, AppError> {
    db.playlist_tracks(&playlist_id)
}

#[tauri::command]
pub fn playlist_reorder(
    playlist_id: String,
    ordered_ids: Vec<String>,
    db: State<'_, crate::db::Database>,
) -> Result<(), AppError> {
    db.reorder(&playlist_id, &ordered_ids)
}

#[tauri::command]
pub fn history_add(track: MetadataTrack, db: State<'_, crate::db::Database>) -> Result<(), AppError> {
    db.add_history(&track).map(|_| ())
}

#[tauri::command]
pub fn history_list(limit: Option<u32>, db: State<'_, crate::db::Database>) -> Result<Vec<MetadataTrack>, AppError> {
    db.list_history(limit.unwrap_or(50))
}

// --- P3 (cleanup): cache + history maintenance ---------------------------

/// Result of a manual prune: how many orphan rows were removed and how many
/// bytes the subsequent `VACUUM` reclaimed from the `.db` file.
#[derive(Debug, Clone, Serialize)]
pub struct CachePruneResult {
    pub removed: usize,
    pub freed_bytes: i64,
}

/// Remove `tracks_cache` rows no longer referenced by any playlist/history,
/// then `VACUUM` so the database file actually shrinks on disk.
///
/// Runs on a blocking thread (`spawn_blocking`) — `VACUUM` is synchronous and
/// exclusive, so it must never execute on the UI/main thread or the app would
/// appear to freeze while it compacts.
#[tauri::command]
pub async fn cache_prune(app: AppHandle) -> Result<CachePruneResult, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.state::<crate::db::Database>();
        let removed = db.prune_orphan_cache()?;
        // Best-effort compaction: even if VACUUM fails, the prune already happened.
        let freed_bytes = db
            .vacuum()
            .unwrap_or_else(|e| {
                eprintln!("[bilusic] cache_prune VACUUM 失败：{e}");
                0
            });
        Ok(CachePruneResult { removed, freed_bytes })
    })
    .await
    .map_err(|e| AppError::msg(format!("清理任务被取消：{e}")))?
}

/// Wipe *all* local data (playlists, history, cache). Destructive. The DB file
/// is `VACUUM`ed inside `reset_all` so the wipe is reflected on disk. Runs off
/// the UI thread via `spawn_blocking`.
#[tauri::command]
pub async fn cache_reset(app: AppHandle) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || app.state::<crate::db::Database>().reset_all())
        .await
        .map_err(|e| AppError::msg(format!("清空任务被取消：{e}")))?
}

/// Compact the database file without deleting any data. Lets the user reclaim
/// disk after many edits/deletes without losing cached metadata. Returns the
/// bytes reclaimed. Runs off the UI thread via `spawn_blocking`.
#[tauri::command]
pub async fn cache_optimize(app: AppHandle) -> Result<i64, AppError> {
    tauri::async_runtime::spawn_blocking(move || app.state::<crate::db::Database>().vacuum())
        .await
        .map_err(|e| AppError::msg(format!("压缩任务被取消：{e}")))?
}

/// Clear play history only (orphaned cache rows are pruned as a side effect).
/// Runs off the UI thread via `spawn_blocking` (the trailing `VACUUM`).
#[tauri::command]
pub async fn history_clear(app: AppHandle) -> Result<usize, AppError> {
    tauri::async_runtime::spawn_blocking(move || app.state::<crate::db::Database>().clear_history())
        .await
        .map_err(|e| AppError::msg(format!("清除任务被取消：{e}")))?
}

/// Re-fetch metadata for every cached track from the active source and
/// re-upsert it, so stale covers/titles are refreshed. Best-effort: a track
/// with no match (or a fetch error) keeps its existing cache entry.
#[tauri::command]
pub async fn cache_refresh(
    coordinator: State<'_, Coordinator>,
    db: State<'_, crate::db::Database>,
) -> Result<usize, AppError> {
    let tracks = db.all_cached_tracks()?;
    let mut refreshed = 0usize;
    for t in tracks {
        if let Ok(results) = coordinator.search(&t.title, 1).await {
            if let Some(m) = results.into_iter().find(|r| r.source_id == t.source_id) {
                db.upsert_track(&m)?;
                refreshed += 1;
            }
        }
    }
    Ok(refreshed)
}

/// A health snapshot of the track cache (size / staleness), for Settings.
#[tauri::command]
pub fn cache_stats(db: State<'_, crate::db::Database>) -> Result<crate::db::CacheStats, AppError> {
    db.cache_stats()
}
