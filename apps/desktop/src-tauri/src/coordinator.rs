//! Coordinator: the glue between metadata sources and audio engines.
//!
//! Mirrors Spotube's `client/manager/`: the frontend never talks to a
//! metadata source or an audio engine directly. It always goes through the
//! coordinator, which is the only component that knows the user's current
//! `(metadata_source, audio_engine)` pairing, the enabled/disabled plugin
//! state, and the cookies that go with them.

use crate::cache::TtlCache;
use crate::engine::{AudioCandidate, StreamInfo};
use crate::error::AppError;
use crate::metadata::{FeedSong, HomeFeed, Lyrics, MetadataTrack};
use crate::plugin::PluginInfo;
use crate::plugin_host::PluginHost;
use crate::plugins::netease_metadata::NeteaseMetadata;
use serde::Serialize;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// How long home / chart data is considered fresh before we re-hit the
/// upstream provider. Mirrors the frontend's 30-minute localStorage TTL so the
/// two layers agree on "this is still good".
const HOME_TTL: Duration = Duration::from_secs(30 * 60);

pub struct Coordinator {
    /// Owns the built-in registries, discovered manifests, and enable/order
    /// state. All source/engine resolution goes through here.
    pub host: Arc<PluginHost>,
    /// Currently selected metadata source plugin id (must be an enabled,
    /// implemented source).
    pub active_metadata: RwLock<String>,
    /// Currently selected audio engine plugin id (must be an enabled engine).
    pub active_engine: RwLock<String>,
    /// Process-lifetime TTL cache for the home feed (keyed by source id).
    pub home_cache: TtlCache<HomeFeed>,
    /// Process-lifetime TTL cache for toplists (keyed by `source_id:topid`).
    pub toplist_cache: TtlCache<Vec<FeedSong>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaybackInfo {
    pub track: MetadataTrack,
    pub stream: StreamInfo,
    /// Identifier of the audio source on the active engine
    /// (e.g. a Bilibili `bvid`). Lets the UI recognise "the *current*
    /// candidate" inside the "其他音源" picker so it can stay open after a
    /// switch and visually mark the active row. `None` when the engine has
    /// no equivalent (e.g. future ID-less streams).
    pub bvid: Option<String>,
    pub engine: String,
}

impl Coordinator {
    pub fn new(host: Arc<PluginHost>) -> Self {
        Self {
            host,
            active_metadata: RwLock::new(String::new()),
            active_engine: RwLock::new(String::new()),
            // Home + 5 region charts is a handful of entries; capacity 64 is
            // plenty and leaves room for multiple source ids.
            home_cache: TtlCache::new(HOME_TTL, 64),
            toplist_cache: TtlCache::new(HOME_TTL, 64),
        }
    }

    /// Pick sensible defaults once the host has discovered manifests and
    /// loaded persisted state (called from `setup`, after `host.init`).
    pub fn init_defaults(&self) {
        let first_meta = self
            .host
            .enabled_metadata_sources()
            .first()
            .map(|p| p.id.clone())
            .unwrap_or_default();
        let first_eng = self
            .host
            .enabled_audio_engines()
            .first()
            .map(|p| p.id.clone())
            .unwrap_or_default();
        {
            let mut a = self.active_metadata.write().unwrap();
            if a.is_empty() || !self.host.has_enabled_metadata(&a) {
                *a = first_meta;
            }
        }
        {
            let mut a = self.active_engine.write().unwrap();
            if a.is_empty() || !self.host.has_enabled_engine(&a) {
                *a = first_eng;
            }
        }
    }

    pub fn list_metadata_sources(&self) -> Vec<PluginInfo> {
        self.host.enabled_metadata_sources()
    }

    pub fn list_audio_engines(&self) -> Vec<PluginInfo> {
        self.host.enabled_audio_engines()
    }

    pub fn set_active_metadata(&self, id: &str) {
        if self.host.has_enabled_metadata(id) {
            *self.active_metadata.write().unwrap() = id.to_string();
            // Home / toplist feeds are source-specific and cached by source id
            // (keys `home:<id>` / `toplist:<id>`). Invalidate both so the new
            // selection can't be served — or silently overwritten — by a stale
            // entry from the previous source. Without this, switching the
            // metadata source could leave the home page showing the old
            // source's content (N11f).
            self.home_cache.clear();
            self.toplist_cache.clear();
        }
    }

    pub fn set_active_engine(&self, id: &str) {
        if self.host.has_enabled_engine(id) {
            *self.active_engine.write().unwrap() = id.to_string();
        }
    }

    /// Resolve the effective active metadata id, falling back to the first
    /// enabled source if the stored selection was disabled.
    fn active_metadata_id(&self) -> String {
        let a = self.active_metadata.read().unwrap().clone();
        if self.host.has_enabled_metadata(&a) {
            a
        } else {
            self.host
                .enabled_metadata_sources()
                .first()
                .map(|p| p.id.clone())
                .unwrap_or_default()
        }
    }

    /// Same as [`Coordinator::active_metadata_id`] but for the audio engine.
    fn active_engine_id(&self) -> String {
        let a = self.active_engine.read().unwrap().clone();
        if self.host.has_enabled_engine(&a) {
            a
        } else {
            self.host
                .enabled_audio_engines()
                .first()
                .map(|p| p.id.clone())
                .unwrap_or_default()
        }
    }

    pub async fn search(&self, query: &str, page: u32) -> Result<Vec<MetadataTrack>, AppError> {
        let id = self.active_metadata_id();
        let src = self
            .host
            .metadata
            .get(&id)
            .ok_or_else(|| AppError::msg("未选择元数据源"))?;
        src.search(query, page).await
    }

    pub async fn home_feed(
        &self,
        // Optional front-end-supplied source override. When present, fetch
        // the home feed via this plugin (resolved via
        // `active_metadata_id` semantics; enabled-only) instead of the
        // backend's currently-active `active_metadata`. Lets the front-end
        // pin the source it actually wants to display, even if the
        // backend's `active_metadata` hasn't been pushed yet (launch
        // race) or has been pushed but its cache TTL means the front-end
        // cache key (which is derived from front-end state) would be
        // written under the wrong-source name.
        source_override: Option<&str>,
    ) -> Result<HomeFeed, AppError> {
        // Resolve which source to query. Override > active; if neither is
        // usable (empty / disabled), fall through to the same fallback as
        // `active_metadata_id()` so we never return an error on the very
        // first launch before any selection has been made.
        let id = if let Some(s) = source_override {
            if self.host.has_enabled_metadata(s) {
                s.to_string()
            } else {
                self.active_metadata_id()
            }
        } else {
            self.active_metadata_id()
        };
        let key = format!("home:{id}");
        // Serve from cache first — within the TTL there's no reason to hit QQ.
        if let Some(cached) = self.home_cache.get(&key) {
            return Ok(cached);
        }
        let src = self
            .host
            .metadata
            .get(&id)
            .ok_or_else(|| AppError::msg("未选择元数据源"))?;
        let feed = src.home().await?;
        self.home_cache.put(key, feed.clone());
        Ok(feed)
    }

    /// Fetch a named toplist / chart from the active metadata source.
    pub async fn toplist(&self, topid: i32) -> Result<Vec<FeedSong>, AppError> {
        let id = self.active_metadata_id();
        let key = format!("toplist:{id}:{topid}");
        if let Some(cached) = self.toplist_cache.get(&key) {
            return Ok(cached);
        }
        let src = self
            .host
            .metadata
            .get(&id)
            .ok_or_else(|| AppError::msg("未选择元数据源"))?;
        let list = src.toplist(topid).await?;
        self.toplist_cache.put(key, list.clone());
        Ok(list)
    }

    /// Fetch the track list of a third-party playlist by its source-side id.
    /// Accepts a frontend source override for the same launch-race reasons
    /// as `home_feed` (N11z): the playlist id alone is ambiguous if the user
    /// hasn't settled on a metadata source yet — `?disstid=123` from QQ and
    /// `?id=123` from Netease are different things. Pinning the source via
    /// the frontend avoids landing on the wrong one when the backend's
    /// `active_metadata` is still on its boot-time fallback.
    pub async fn get_playlist(
        &self,
        source_override: Option<&str>,
        id: &str,
    ) -> Result<Vec<FeedSong>, AppError> {
        let source_id = if let Some(s) = source_override {
            if self.host.has_enabled_metadata(s) {
                s.to_string()
            } else {
                self.active_metadata_id()
            }
        } else {
            self.active_metadata_id()
        };
        let src = self
            .host
            .metadata
            .get(&source_id)
            .ok_or_else(|| AppError::msg("未选择元数据源"))?;
        src.get_playlist(id).await
    }

    /// Fetch lyrics from a specific metadata source (defaults to the active
    /// source if `source_id` is empty). Lyrics-only sources (e.g. `lrclib`)
    /// are reachable here even though they are not selectable as the active
    /// search source.
    ///
/// Fetch lyrics for a track. Lyrics routing is a **3-step fallback chain**:
///
/// 1. **Active source** (or the explicitly named `source_id`) — tries
///    `src.get_lyrics(track)`. Empty / non-OK responses are OK; we fall
///    through. This is the "source-of-truth" path when a source has its own
///    lyric integration (netease → `song/lyric`, lrclib → `api/get`).
///
/// 2. **Cross-source netease fallback** — only when `source_id` was empty
///    (= "use active, I don't care about which lyric source"). We search
///    netease by `title + artist` and pull LRC for the top hit. ~90% of
///    C-pop tracks (qq_music; bilibili native metadata gets the same
///    fallback when its source_id happens to be empty) get covered this way
///    without having to write a lyrics endpoint per source — see
///    `NeteaseMetadata::lookup_lyrics`. This is the unblocking path for
///    qq-music, whose own lyric API requires VIP login and is **intentionally
///    not implemented**.
///
/// 3. **lrclib fallback** — free, auth-free, English-led community DB.
///    Solid for Western tracks; near-empty for C-pop. Always tried last so
///    it doesn't shadow the active / netease hits.
///
/// Why "source_id 非空跳过 netease 兜底": when the frontend explicitly
/// asks for `lrclib` (e.g. to force-display Western lyrics), we honour that
/// request — adding netease on top would silently change behaviour.
pub async fn get_lyrics(
    &self,
    source_id: &str,
    track: &MetadataTrack,
) -> Result<Lyrics, AppError> {
    let id = if source_id.is_empty() {
        self.active_metadata_id()
    } else {
        source_id.to_string()
    };

    // 1) Active / explicitly-named source.
    if let Some(src) = self.host.metadata.get(&id) {
        if let Ok(lyrics) = src.get_lyrics(track).await {
            if !lyrics.lines.is_empty() {
                return Ok(lyrics);
            }
        }
    }

    // 2) Cross-source netease fallback — only on the "default" path.
    if source_id.is_empty() {
        let netease = NeteaseMetadata::new();
        if let Ok(lyrics) = netease.lookup_lyrics(&track.title, &track.artist).await {
            if !lyrics.lines.is_empty() {
                return Ok(lyrics);
            }
        }
    }

    // 3) lrclib fallback (always; it's a lyrics-only source and will be
    // no-op for sources where lyrics are unsupported).
    if let Some(lrclib) = self.host.metadata.get("lrclib") {
        if let Ok(lyrics) = lrclib.get_lyrics(track).await {
            return Ok(lyrics);
        }
    }

    Ok(Lyrics { lines: vec![] })
}

    pub async fn play(&self, track: &MetadataTrack) -> Result<PlaybackInfo, AppError> {
        // Choose the engine: prefer the track's own hint (if enabled), then the
        // user's active selection (falling back to the first enabled engine).
        let engine_id = track
            .engine_hint
            .clone()
            .filter(|id| self.host.has_enabled_engine(id))
            .unwrap_or_else(|| self.active_engine_id());
        let engine = self
            .host
            .engines
            .get(&engine_id)
            .ok_or_else(|| AppError::msg(format!("未找到音频引擎: {engine_id}")))?;

        // If metadata and engine share ids (e.g. both bilibili), use the direct
        // path; otherwise do a query-based match.
        let (stream, engine_track_id) = if engine_id == track.source {
            (
                engine.find_stream_by_id(&track.source_id).await?,
                // The "by-id" path returns an audio stream keyed by an engine-
                // native id. For Bilibili that id is the bvid; the UI uses it
                // to highlight "the candidate we're already playing" inside
                // the 其他音源 picker. Other engines that adopt by-id
                // resolution can fill this in their own `find_stream_by_id`.
                Some(track.source_id.clone()),
            )
        } else {
            let q = format!("{} {}", track.title, track.artist);
            (
                engine.find_stream_by_query(&q, track).await?,
                None,
            )
        };

        Ok(PlaybackInfo {
            track: track.clone(),
            stream,
            bvid: engine_track_id,
            engine: engine_id,
        })
    }

    /// List candidate audio sources for a free-text query on the active audio
    /// engine. Powers the player bar's "其他音源" picker — the frontend shows
    /// the list and resolves the chosen one via `play_by_bvid` (keeping the
    /// current song's metadata).
    pub async fn audio_search(&self, query: &str) -> Result<Vec<AudioCandidate>, AppError> {
        let engine_id = self.active_engine_id();
        let engine = self
            .host
            .engines
            .get(&engine_id)
            .ok_or_else(|| AppError::msg(format!("未找到音频引擎: {engine_id}")))?;
        engine.search(query).await
    }
}
