//! Bilusic desktop library entry point.
//!
//! Boot order:
//!   1. Construct Bili plugin instances once, share `Arc`s between the
//!      registries (for plugin discovery) and the cookie service (for
//!      pushing SESSDATA in at runtime).
//!   2. Construct the [`Coordinator`] with the populated registries.
//!   3. Spawn the local stream proxy.
//!   4. Hand coordinator + cookie service to Tauri so the frontend can call
//!      them via commands.

mod commands;
mod coordinator;
mod cache;
mod db;
mod engine;
mod error;
mod metadata;
mod plugin;
mod plugin_host;
mod declarative;
#[cfg(feature = "wasm")]
mod wasm_runtime;
#[cfg(feature = "js")]
mod js_runtime;
mod plugins;
mod proxy;
mod wbi;

use std::sync::Arc;
use tauri::Manager;

use crate::coordinator::Coordinator;
use crate::engine::EngineRegistry;
use crate::metadata::MetadataRegistry;
use crate::plugin_host::PluginHost;
use crate::plugins::bilibili_engine::BilibiliEngine;
use crate::plugins::bilibili_metadata::BilibiliMetadata;
use crate::plugins::lrclib::Lrclib;
use crate::plugins::netease_metadata::NeteaseMetadata;
use crate::plugins::qq_music_metadata::QqMusicMetadata;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Single instances shared by registry + cookie service.
    let bilibili_metadata = Arc::new(BilibiliMetadata::new());
    let bilibili_engine = Arc::new(BilibiliEngine::new());
    let qq_music = Arc::new(QqMusicMetadata::new());

    // First-party metadata sources (P5-II): lrclib is fully implemented
    // (lyrics-only); netease is fully implemented (weapi encryption +
    // search/get_track/get_lyrics/home). kuwo is implemented as a JS plugin
    // under apps/desktop/plugins/kuwo/ (real source, enabled by default).
    //
    // N14 (2026-08-27): `kugou` / `spotify` / `ytmusic` 三个 阶段 2 skeleton
    // 已彻底删除（永不启用、纯死代码）；EXPERIMENTAL 数组随之清空。
    let lrclib = Arc::new(Lrclib::new());
    let netease = Arc::new(NeteaseMetadata::new());

    let metadata_reg = MetadataRegistry::default();
    // Register QQ Music first so it is the default active metadata source
    // (Coordinator picks the first search-capable enabled source). Bilibili
    // metadata + the first-party sources remain available as alternatives.
    metadata_reg.register(qq_music);
    metadata_reg.register(bilibili_metadata.clone());
    metadata_reg.register(lrclib);
    metadata_reg.register(netease);
    let engine_reg = EngineRegistry::default();
    engine_reg.register(bilibili_engine.clone());

    // The plugin host owns both registries plus discovered third-party
    // manifests and the enable/order state. The Coordinator delegates all
    // source/engine resolution to it.
    //
    // N11c: JS HTTP host handle is captured inside `host.init()` via
    // `tauri::async_runtime::handle()` — Tauri's global, thread-agnostic
    // handle accessor. It uses `get_or_init(default_runtime)` (Tauri
    // source: `async_runtime.rs::handle`), so it works from the sync
    // `.setup` closure (which runs on the tao main thread, NOT inside any
    // tokio runtime) where `Handle::current()` / `Handle::try_current()`
    // would fail with "there is no reactor running" (N11b mistake).
    let host = PluginHost::new(Arc::new(metadata_reg), Arc::new(engine_reg));
    let coordinator = Coordinator::new(host.clone());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(coordinator)
        .manage(PluginCookieService {
            bilibili_metadata: bilibili_metadata.clone(),
            bilibili_engine: bilibili_engine.clone(),
        })
        .setup(|app| {
            // Open the local SQLite database inside the app's data dir.
            let dir = app
                .path()
                .app_data_dir()
                .map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let db = db::Database::open(dir.join("bilusic.db"))
                .map_err(|e| e.to_string())?;
            // Best-effort: reclaim any orphaned cache rows (and stale
            // playlist-track links from before FK cascade was enabled) left by
            // older builds. The prune logic also runs after every delete.
            if let Err(e) = db.prune_orphans() {
                eprintln!("[bilusic] 启动缓存清理失败：{e}");
            }
            app.manage(db);

            // Deferred vacuum: if a *previous* run left a `.need_vacuum` marker
            // (i.e. it deleted/reset data on exit), compact the file now in a
            // background thread so the user never waits. The marker is removed
            // first so a crash mid-VACUUM can't loop forever.
            let marker = dir.join(".need_vacuum");
            if marker.exists() {
                let _ = std::fs::remove_file(&marker);
                let handle = app.handle().clone();
                tauri::async_runtime::spawn_blocking(move || {
                    match handle.state::<db::Database>().auto_vacuum() {
                        Ok(freed) if freed > 0 => {
                            eprintln!("[bilusic] 后台压缩数据库，释放 {} 字节", freed);
                        }
                        Ok(_) => {}
                        Err(e) => eprintln!("[bilusic] 后台 VACUUM 失败：{e}"),
                    }
                });
            }

            // Spawn the local streaming proxy (best-effort: never fails setup).
            proxy::start_proxy();

            // Discover third-party plugin manifests + load persisted enable/
            // order state, then pick default active source/engine.
            let coord = app.state::<Coordinator>();
            let res_dir = app.path().resource_dir().ok();
            // N11i: hand the AppHandle to host.init() so JS plugins'
            // `bilusic.log(...)` can be mirrored into the webview's
            // DevTools Console via WebviewWindow::eval.
            coord.host.init(&dir, res_dir.as_deref(), &app.handle());
            coord.init_defaults();

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_plugins,
            commands::set_active_metadata,
            commands::set_active_engine,
            commands::plugin_list,
            commands::plugin_toggle,
            commands::plugin_reorder,
            commands::plugin_uninstall,
            commands::plugin_import,
            commands::metadata_search,
            commands::metadata_home,
            commands::metadata_toplist,
            commands::metadata_playlist,
            commands::metadata_get_lyrics,
            commands::coordinator_play,
            commands::play_by_bvid,
            commands::audio_search,
            commands::set_bilibili_cookie,
            commands::proxy_base_url,
            commands::playlist_create,
            commands::playlist_list,
            commands::playlist_rename,
            commands::playlist_delete,
            commands::playlist_add_track,
            commands::playlist_remove_track,
            commands::playlist_tracks,
            commands::playlist_reorder,
            commands::history_add,
            commands::history_list,
            commands::cache_prune,
            commands::cache_reset,
            commands::cache_optimize,
            commands::history_clear,
            commands::cache_refresh,
            commands::cache_stats,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // On shutdown, do NOT run VACUUM on the UI/main thread (it would
            // block quit and look like a freeze). Instead, if there's free
            // space to reclaim, leave a `.need_vacuum` marker so the *next*
            // launch compacts in a background thread — quitting stays instant.
            if let tauri::RunEvent::Exit = event {
                if app.state::<db::Database>().needs_vacuum() {
                    if let Ok(dir) = app.path().app_data_dir() {
                        if let Err(e) = std::fs::write(dir.join(".need_vacuum"), "") {
                            eprintln!("[bilusic] 写入 VACUUM 标记失败：{e}");
                        }
                    }
                }
            }
        });
}

/// Side-channel for pushing the user's SESSDATA into the Bili plugins. Lives
/// in the app state so the frontend can update it from the Settings page.
pub struct PluginCookieService {
    pub bilibili_metadata: Arc<BilibiliMetadata>,
    pub bilibili_engine: Arc<BilibiliEngine>,
}
