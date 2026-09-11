//! Metadata source plugin trait + registry.
//!
//! Metadata sources answer *what to play* (track title, artist, cover,
//! duration, lyrics) and never touch audio bytes. Plugins live as
//! `apps/desktop/plugins/<id>/{plugin.json,index.js}` for future JS-driven
//! plugins; for now we ship Rust-built-in implementations.

use crate::error::AppError;
use crate::plugin::{Plugin, PluginCapabilities, PluginInfo};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// A track as returned by a metadata source. Carries enough info to render a
/// search result, populate the player bar, and (optionally) hand back to an
/// audio engine for stream resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataTrack {
    /// ID inside the originating metadata source (e.g. bvid for bilibili, mid
    /// for QQ music). The audio engine uses this when its own IDs match.
    pub source_id: String,
    /// Plugin id of the originating metadata source (`"bilibili"`, `"qq-music"`, …).
    pub source: String,
    /// Plugin id of the audio engine expected to resolve this track
    /// (`"bilibili"`, `"youtube"`, …). Set by the coordinator at search time,
    /// not by the plugin itself — the source just returns the canonical info.
    pub engine_hint: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: i64,
    pub cover: String,
    /// Used to fetch lyrics later. Some sources (Spotify, QQ) make this the
    /// canonical id; for bilibili we reuse the bvid.
    pub lyrics_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyricLine {
    pub time_ms: i64,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
}

/// A single entry inside a home-page section. Carries enough info to render
/// either a playable song card or a clickable browse card (playlist / album /
/// artist). The `kind` field drives which renderer the frontend picks.
///
/// This is the unified replacement for the old `FeedItem` / `feed.playlists`
/// / `feed.new_songs` / `feed.albums` / `feed.artists` split — a plugin now
/// declares *what* to show by composing `[HomeSection]` instead of filling
/// four predetermined buckets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionItem {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub cover: String,
    /// What this item is: "song" | "playlist" | "album" | "artist".
    /// "song" items are playable; the rest are browse-able (click → search).
    pub kind: String,
    pub source: String,
    // ── song-only fields (present only when kind == "song") ──
    pub artist: Option<String>,
    pub duration_ms: Option<i64>,
    pub engine_hint: Option<String>,
}

/// A home-page section declared by a metadata plugin. The plugin supplies the
/// real section title (no frontend i18n key), the item kind, and the items.
/// The frontend renders `sections` generically via `kind` → renderer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeSection {
    /// Stable id for React keys / cache bucketing.
    pub id: String,
    /// Human title shown above the row (plugin-supplied, any language).
    pub title: String,
    /// Renderer hint: "song" → SongCard row; otherwise → browse card row.
    pub kind: String,
    pub items: Vec<SectionItem>,
    /// Optional small caption next to the title (e.g. "Top N").
    pub hint: Option<String>,
}

/// A playable song returned by `toplist` / `search`. Kept separate from
/// `SectionItem` because toplist is addressed by a numeric `topid` and has no
/// section grouping of its own.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedSong {
    pub source_id: String, // songmid / native id
    pub title: String,
    pub artist: String,
    pub cover: String,
    pub duration_ms: i64,
    pub source: String,
    /// Which audio engine should resolve this track (e.g. "bilibili").
    pub engine_hint:  Option<String>,
}

/// The home / discover feed returned by a metadata source. Plugins declare
/// their content as an ordered list of `sections`; the frontend renders them
/// on a fixed canvas (the "recently played" platform layer sits above).
///
/// NOTE: there is intentionally NO `schema_rev` field here. Frontend local
/// cache invalidation is driven by TTL + the Settings-page "Clear local
/// cache" action (which wipes the relevant `bilusic.cache.*` localStorage
/// keys), NOT by a hand-maintained version stamp. See N11p.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HomeFeed {
    pub sections: Vec<HomeSection>,
}

#[async_trait]
pub trait MetadataSource: Plugin {
    async fn search(&self, query: &str, page: u32) -> Result<Vec<MetadataTrack>, AppError>;
    async fn get_track(&self, id: &str) -> Result<MetadataTrack, AppError>;
    /// Fetch synced/plain lyrics for a track. Implementations that need the
    /// track's title/artist (e.g. an external lyrics DB) use `track` directly;
    /// those that resolve by an internal id use `track.lyrics_id` /
    /// `track.source_id`. Defaults to empty (no lyrics).
    async fn get_lyrics(&self, _track: &MetadataTrack) -> Result<Lyrics, AppError> {
        Ok(Lyrics { lines: vec![] })
    }
    /// Home / discover feed (playlists, new songs, albums, artists).
    /// Defaults to empty — sources that don't support browsing return this.
    async fn home(&self) -> Result<HomeFeed, AppError> {
        Ok(HomeFeed::default())
    }
    /// A named toplist / chart, addressed by a provider-specific id
    /// (e.g. QQ Music `topid`). Defaults to empty — sources without a
    /// browsable chart endpoint simply return nothing here.
    async fn toplist(&self, _topid: i32) -> Result<Vec<FeedSong>, AppError> {
        Ok(vec![])
    }
    /// Fetch the track list for a browseable playlist by id (an id that came
    /// back inside a `HomeSection` of `kind: "playlist"`). Plugins that don't
    /// expose playlist-detail endpoints simply return the default error so the
    /// frontend can render a graceful "this source doesn't support playlist
    /// detail" message instead of pretending an empty list is the answer.
    async fn get_playlist(&self, _id: &str) -> Result<Vec<FeedSong>, AppError> {
        Err(AppError::msg("该元数据源不支持查看歌单详情"))
    }
}

#[derive(Default)]
pub struct MetadataRegistry {
    sources: RwLock<HashMap<String, Arc<dyn MetadataSource>>>,
}

impl MetadataRegistry {
    /// `&self` (not `&mut`) so the host can register third-party adapters
    /// discovered at `init` time, after the registry has been wrapped in `Arc`.
    pub fn register(&self, src: Arc<dyn MetadataSource>) {
        self.sources.write().unwrap().insert(src.info().id, src);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn MetadataSource>> {
        self.sources.read().unwrap().get(id).cloned()
    }

    pub fn list(&self) -> Vec<PluginInfo> {
        self.sources
            .read()
            .unwrap()
            .values()
            .map(|s| s.info())
            .filter(|p| p.capabilities & PluginCapabilities::METADATA_SEARCH.bits() != 0)
            .collect()
    }

    /// All registered metadata sources regardless of capability — including
    /// lyrics-only / not-yet-searchable sources that `list()` excludes. Used by
    /// the plugin management page so every source (even a lyrics-only one) is
    /// visible and toggleable.
    pub fn list_all(&self) -> Vec<PluginInfo> {
        self.sources
            .read()
            .unwrap()
            .values()
            .map(|s| s.info())
            .collect()
    }
}
