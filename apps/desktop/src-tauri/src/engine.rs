//! Audio engine plugin trait + registry.
//!
//! Audio engines answer *where the audio bytes live* given a metadata track
//! from a source. The engine never knows (or cares) what metadata provider
//! produced the track; it only needs the title/artist and the optional
//! source_id to resolve.

use crate::error::AppError;
use crate::metadata::MetadataTrack;
use crate::plugin::{Plugin, PluginInfo};
use async_trait::async_trait;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Serialize)]
pub struct StreamInfo {
    /// Direct CDN URL (will be wrapped by the local proxy in the frontend).
    pub url: String,
    pub mime: String,
    pub bitrate: u32,
    pub duration_ms: i64,
    pub cover: String,
    pub title: String,
    pub artist: String,
}

/// A candidate audio source returned by an engine's `search`, *before* any
/// stream URL is resolved. Used by the player bar's "其他音源" picker: the
/// frontend shows the list and, when the user picks one, resolves it via
/// `find_stream_by_id` (reusing the existing cross-source switch path).
#[derive(Debug, Clone, Serialize)]
pub struct AudioCandidate {
    /// Engine-native id (e.g. bvid for bilibili) — fed back into
    /// `find_stream_by_id` / `play_by_bvid` to resolve the actual stream.
    pub bvid: String,
    /// Human title of this source (the B 站 video title, not the song name).
    pub title: String,
    /// Uploader / author.
    pub author: String,
    /// Duration in seconds (0 if unknown).
    pub duration_sec: i64,
    /// Thumbnail URL (may be empty).
    pub cover: String,
}

#[async_trait]
pub trait AudioEngine: Plugin {
    /// Resolve by an engine-native id (e.g. bvid for bilibili). Used when the
    /// chosen metadata source shares ids with this engine.
    async fn find_stream_by_id(&self, id: &str) -> Result<StreamInfo, AppError>;

    /// Search-and-resolve by track metadata. Used when metadata and engine
    /// are from different providers (e.g. QQ-music metadata → bilibili
    /// engine). The default implementation is a no-op; engines that want to
    /// support cross-source playback override it.
    async fn find_stream_by_query(
        &self,
        _query: &str,
        _hint: &MetadataTrack,
    ) -> Result<StreamInfo, AppError> {
        Err(AppError::msg(
            "该音频引擎不支持跨源搜索（仅支持按 ID 解析）",
        ))
    }

    /// Search the engine for candidate audio sources by a free-text query,
    /// returning a list of [`AudioCandidate`] *without* resolving streams.
    /// Powers the player bar's "其他音源" picker. The default implementation
    /// is a no-op; engines that want to surface alternative sources override
    /// it (the bilibili engine does).
    async fn search(&self, _query: &str) -> Result<Vec<AudioCandidate>, AppError> {
        Err(AppError::msg("该音频引擎不支持搜索候选音源"))
    }
}

#[derive(Default)]
pub struct EngineRegistry {
    engines: RwLock<HashMap<String, Arc<dyn AudioEngine>>>,
}

impl EngineRegistry {
    /// `&self` (not `&mut`) so the host can register third-party adapters
    /// discovered at `init` time, after the registry has been wrapped in `Arc`.
    pub fn register(&self, eng: Arc<dyn AudioEngine>) {
        self.engines.write().unwrap().insert(eng.info().id, eng);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn AudioEngine>> {
        self.engines.read().unwrap().get(id).cloned()
    }

    pub fn list(&self) -> Vec<PluginInfo> {
        self.engines
            .read()
            .unwrap()
            .values()
            .map(|e| e.info())
            .collect()
    }
}
