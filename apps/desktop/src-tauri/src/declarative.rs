//! Declarative (config-driven, zero-runtime) plugin adapters.
//!
//! A third-party author describes an HTTP source entirely in `plugin.json`
//! (`base_url` + per-endpoint path templates + JSONPath field maps). This
//! module provides [`DeclarativeMetadataSource`] / [`DeclarativeAudioEngine`]
//! that implement the `MetadataSource` / `AudioEngine` traits by issuing those
//! HTTP calls and mapping responses into `MetadataTrack` / `StreamInfo`.
//!
//! **Field mapping uses a JSONPath subset** (`$`, `.key`, `[index]`) — enough
//! to address nested objects and array elements without pulling in a full
//! JSONPath engine. `list_path` points at the array of result items; each
//! `map.*` entry is a JSONPath relative to a single item.

use crate::engine::{AudioEngine, StreamInfo};
use crate::error::AppError;
use crate::metadata::{LyricLine, Lyrics, MetadataSource, MetadataTrack};
use crate::plugin::{Plugin, PluginCapabilities, PluginInfo};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

// --------------------------------------------------------------------------
// Config types (deserialized from `plugin.json`'s `declarative` block)
// --------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct DeclarativeConfig {
    pub base_url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub search: DeclarativeEndpoint,
    #[serde(default)]
    pub get_track: Option<DeclarativeEndpoint>,
    #[serde(default)]
    pub lyrics: Option<DeclarativeEndpoint>,
    /// Audio resolution (`find_stream_by_id`). Optional — many declarative
    /// sources are metadata-only and reuse a built-in engine.
    #[serde(default)]
    pub stream_by_id: Option<DeclarativeAudioEndpoint>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeclarativeEndpoint {
    /// Path template appended to `base_url`. Supports `${query}`, `${page}`,
    /// `${id}`, `${title}`, `${artist}` (values are percent-encoded).
    pub path: String,
    #[serde(default = "default_get")]
    pub method: String,
    /// JSONPath to the array of result items. If absent, the whole response
    /// body is treated as a single item.
    #[serde(default)]
    pub list_path: Option<String>,
    pub map: TrackFieldMap,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeclarativeAudioEndpoint {
    pub path: String,
    #[serde(default = "default_get")]
    pub method: String,
    pub map: AudioFieldMap,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TrackFieldMap {
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<String>,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub lyrics_id: Option<String>,
    /// Whole lyrics / LRC text (used by `get_lyrics`).
    #[serde(default)]
    pub lyrics_text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AudioFieldMap {
    pub url: String,
    #[serde(default)]
    pub mime: Option<String>,
    #[serde(default)]
    pub bitrate: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<String>,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
}

fn default_get() -> String {
    "GET".into()
}

// --------------------------------------------------------------------------
// Minimal JSONPath subset + HTTP / mapping helpers
// --------------------------------------------------------------------------

/// Minimal JSONPath subset: `$`, `.key`, `[index]`. Returns the matched value.
fn jp(value: &Value, path: &str) -> Result<Value, AppError> {
    let mut cur = value;
    let p = path.trim().trim_start_matches('$');
    for seg in p.split('.').filter(|s| !s.is_empty()) {
        let (key, idx) = if let Some(bracket) = seg.find('[') {
            let key = &seg[..bracket];
            let inner = seg[bracket + 1..].trim_end_matches(']');
            let idx: usize = inner
                .parse()
                .map_err(|_| AppError::msg(format!("JSONPath 索引非法：{seg}")))?;
            (key, Some(idx))
        } else {
            (seg, None)
        };
        if !key.is_empty() {
            cur = cur
                .get(key)
                .ok_or_else(|| AppError::msg(format!("JSONPath 找不到键：{key} (path {path})")))?;
        }
        if let Some(i) = idx {
            cur = cur
                .get(i)
                .ok_or_else(|| AppError::msg(format!("JSONPath 索引越界：{i} (path {path})")))?;
        }
    }
    Ok(cur.clone())
}

fn render(tmpl: &str, vars: &[(&str, String)]) -> String {
    let mut s = tmpl.to_string();
    for (k, v) in vars {
        s = s.replace(&format!("${{{k}}}"), v);
    }
    s
}

fn encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn stringify(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Extract a string field via an optional JSONPath. Empty when the path is
/// absent or the lookup fails.
fn pick(v: &Value, path: &Option<String>) -> String {
    match path {
        Some(p) => jp(v, p).ok().and_then(|vv| stringify(&vv)).unwrap_or_default(),
        None => String::new(),
    }
}

fn pick_num(v: &Value, path: &Option<String>) -> i64 {
    pick(v, path).parse().unwrap_or(0)
}

fn build_headers(headers: &HashMap<String, String>) -> HeaderMap {
    let mut hm = HeaderMap::new();
    for (k, v) in headers {
        if let (Ok(n), Ok(val)) = (
            HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v),
        ) {
            hm.insert(n, val);
        }
    }
    hm
}

fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let mut out = vec![];
    for raw in text.lines() {
        let mut s = raw.trim();
        if s.is_empty() {
            continue;
        }
        let mut time_ms = 0i64;
        while s.starts_with('[') {
            if let Some(end) = s.find(']') {
                let tag = &s[1..end];
                if let Some((mm, rest)) = tag.split_once(':') {
                    if let (Ok(m), Ok(sec)) = (mm.trim().parse::<i64>(), rest.trim().parse::<f64>()) {
                        time_ms = m * 60_000 + (sec * 1000.0) as i64;
                    }
                }
                s = s[end + 1..].trim_start();
            } else {
                break;
            }
        }
        if !s.is_empty() {
            out.push(LyricLine {
                time_ms,
                text: s.to_string(),
            });
        }
    }
    out
}

// --------------------------------------------------------------------------
// Metadata source adapter
// --------------------------------------------------------------------------

pub struct DeclarativeMetadataSource {
    info: PluginInfo,
    cfg: DeclarativeConfig,
    client: reqwest::Client,
}

impl DeclarativeMetadataSource {
    pub fn new(
        id: &str,
        name: &str,
        version: &str,
        description: &str,
        caps: PluginCapabilities,
        cfg: DeclarativeConfig,
    ) -> Self {
        let info = PluginInfo {
            id: id.to_string(),
            name: name.to_string(),
            version: version.to_string(),
            description: description.to_string(),
            capabilities: caps.bits(),
        };
        Self {
            info,
            cfg,
            client: reqwest::Client::new(),
        }
    }

    async fn fetch_json(
        &self,
        ep: &DeclarativeEndpoint,
        vars: &[(&str, String)],
    ) -> Result<Value, AppError> {
        let url = format!("{}{}", self.cfg.base_url, render(&ep.path, vars));
        let mut req = match ep.method.to_uppercase().as_str() {
            "POST" => self.client.post(&url),
            _ => self.client.get(&url),
        };
        if !self.cfg.headers.is_empty() {
            req = req.headers(build_headers(&self.cfg.headers));
        }
        let resp = req
            .send()
            .await
            .map_err(|e| AppError::msg(format!("declarative HTTP 失败：{e}")))?;
        let body: Value = resp
            .json()
            .await
            .map_err(|e| AppError::msg(format!("declarative 解析 JSON 失败：{e}")))?;
        Ok(body)
    }
}

fn map_track(item: &Value, m: &TrackFieldMap, source: &str) -> MetadataTrack {
    MetadataTrack {
        source_id: pick(item, &m.source_id),
        source: source.to_string(),
        engine_hint: None,
        title: pick(item, &m.title),
        artist: pick(item, &m.artist),
        album: pick(item, &m.album),
        duration_ms: pick_num(item, &m.duration_ms),
        cover: pick(item, &m.cover),
        lyrics_id: m
            .lyrics_id
            .as_ref()
            .and_then(|p| jp(item, p).ok())
            .and_then(|vv| stringify(&vv)),
    }
}

#[async_trait]
impl Plugin for DeclarativeMetadataSource {
    fn info(&self) -> PluginInfo {
        self.info.clone()
    }
}

#[async_trait]
impl MetadataSource for DeclarativeMetadataSource {
    async fn search(&self, query: &str, page: u32) -> Result<Vec<MetadataTrack>, AppError> {
        let ep = &self.cfg.search;
        let body = self
            .fetch_json(
                ep,
                &[("query", encode(query)), ("page", page.to_string())],
            )
            .await?;
        let items = match &ep.list_path {
            Some(lp) => jp(&body, lp)
                .and_then(|v| {
                    v.as_array()
                        .cloned()
                        .ok_or_else(|| AppError::msg("declarative list_path 非数组"))
                })
                .unwrap_or_default(),
            None => vec![body],
        };
        Ok(items
            .iter()
            .map(|it| map_track(it, &ep.map, &self.info.id))
            .collect())
    }

    async fn get_track(&self, id: &str) -> Result<MetadataTrack, AppError> {
        let ep = self
            .cfg
            .get_track
            .as_ref()
            .ok_or_else(|| AppError::msg("该声明式源未配置 get_track"))?;
        let body = self.fetch_json(ep, &[("id", encode(id))]).await?;
        Ok(map_track(&body, &ep.map, &self.info.id))
    }

    async fn get_lyrics(&self, track: &MetadataTrack) -> Result<Lyrics, AppError> {
        let ep = match &self.cfg.lyrics {
            Some(e) => e,
            None => return Ok(Lyrics::default()),
        };
        let body = self
            .fetch_json(
                ep,
                &[
                    ("id", encode(&track.source_id)),
                    ("title", encode(&track.title)),
                    ("artist", encode(&track.artist)),
                ],
            )
            .await?;
        let items = match &ep.list_path {
            Some(lp) => jp(&body, lp)
                .and_then(|v| {
                    v.as_array()
                        .cloned()
                        .ok_or_else(|| AppError::msg("declarative lyrics list_path 非数组"))
                })
                .unwrap_or_default(),
            None => vec![body],
        };
        let text = items
            .first()
            .and_then(|it| ep.map.lyrics_text.as_ref().and_then(|p| jp(it, p).ok()))
            .and_then(|vv| stringify(&vv))
            .unwrap_or_default();
        Ok(Lyrics {
            lines: parse_lrc(&text),
        })
    }
}

// --------------------------------------------------------------------------
// Audio engine adapter (optional — only when `stream_by_id` is configured)
// --------------------------------------------------------------------------

pub struct DeclarativeAudioEngine {
    info: PluginInfo,
    base_url: String,
    headers: HashMap<String, String>,
    endpoint: DeclarativeAudioEndpoint,
    client: reqwest::Client,
}

impl DeclarativeAudioEngine {
    pub fn new(
        id: &str,
        name: &str,
        version: &str,
        description: &str,
        caps: PluginCapabilities,
        cfg: DeclarativeConfig,
    ) -> Self {
        let info = PluginInfo {
            id: id.to_string(),
            name: name.to_string(),
            version: version.to_string(),
            description: description.to_string(),
            capabilities: caps.bits(),
        };
        let endpoint = cfg
            .stream_by_id
            .expect("DeclarativeAudioEngine 仅在 stream_by_id 配置时构造");
        Self {
            info,
            base_url: cfg.base_url,
            headers: cfg.headers,
            endpoint,
            client: reqwest::Client::new(),
        }
    }

    async fn fetch_json(&self, vars: &[(&str, String)]) -> Result<Value, AppError> {
        let url = format!("{}{}", self.base_url, render(&self.endpoint.path, vars));
        let mut req = match self.endpoint.method.to_uppercase().as_str() {
            "POST" => self.client.post(&url),
            _ => self.client.get(&url),
        };
        if !self.headers.is_empty() {
            req = req.headers(build_headers(&self.headers));
        }
        let resp = req
            .send()
            .await
            .map_err(|e| AppError::msg(format!("declarative 音频 HTTP 失败：{e}")))?;
        let body: Value = resp
            .json()
            .await
            .map_err(|e| AppError::msg(format!("declarative 音频解析 JSON 失败：{e}")))?;
        Ok(body)
    }
}

fn map_stream(v: &Value, m: &AudioFieldMap, _source: &str) -> StreamInfo {
    let url = jp(v, &m.url)
        .ok()
        .and_then(|vv| stringify(&vv))
        .unwrap_or_default();
    StreamInfo {
        url,
        mime: m.mime.clone().unwrap_or_else(|| "audio/mpeg".into()),
        bitrate: pick_num(v, &m.bitrate) as u32,
        duration_ms: pick_num(v, &m.duration_ms),
        cover: pick(v, &m.cover),
        title: pick(v, &m.title),
        artist: pick(v, &m.artist),
    }
}

#[async_trait]
impl Plugin for DeclarativeAudioEngine {
    fn info(&self) -> PluginInfo {
        self.info.clone()
    }
}

#[async_trait]
impl AudioEngine for DeclarativeAudioEngine {
    async fn find_stream_by_id(&self, id: &str) -> Result<StreamInfo, AppError> {
        let body = self.fetch_json(&[("id", encode(id))]).await?;
        Ok(map_stream(&body, &self.endpoint.map, &self.info.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::PluginCapabilities;

    /// Online smoke test — only runs with `-- --ignored` (needs network to
    /// itunes.apple.com). Uses the exact config shipped in the import demo
    /// plugin (`bilusic-plugin-demo/plugin.json`) to prove the declarative
    /// pipeline: config → HTTP → JSONPath → MetadataTrack.
    #[tokio::test]
    #[ignore]
    async fn itunes_declarative_search_smoke() {
        let cfg = serde_json::from_str::<DeclarativeConfig>(
            r#"{
                "base_url": "https://itunes.apple.com",
                "headers": { "Accept": "application/json" },
                "search": {
                    "path": "/search?term=${query}&media=music&limit=20",
                    "method": "GET",
                    "list_path": "$.results",
                    "map": {
                        "source_id": "$.trackId",
                        "title": "$.trackName",
                        "artist": "$.artistName",
                        "album": "$.collectionName",
                        "duration_ms": "$.trackTimeMillis",
                        "cover": "$.artworkUrl100",
                        "lyrics_id": "$.trackId"
                    }
                }
            }"#,
        )
        .expect("config parse");
        let caps = PluginCapabilities::METADATA_SEARCH;
        let src = DeclarativeMetadataSource::new(
            "itunes-declarative",
            "iTunes Search (declarative demo)",
            "0.1.0",
            "demo",
            caps,
            cfg,
        );
        let tracks = src.search("daft punk", 1).await.expect("declarative search");
        assert!(!tracks.is_empty(), "search returned no results");
        let t = &tracks[0];
        assert!(
            !t.title.is_empty() && !t.artist.is_empty(),
            "track fields mapped (title/artist)"
        );
        eprintln!(
            "[test] declarative itunes: {} — {} ({})",
            t.title, t.artist, t.album
        );
    }
}
