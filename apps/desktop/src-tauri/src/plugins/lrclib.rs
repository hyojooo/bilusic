//! Built-in `lrclib` lyrics source (P5-II reference implementation).
//!
//! LrcLib (https://lrclib.net) is a free, **auth-free** lyrics database with a
//! simple REST API. This plugin is the "proves the non-stub path" reference:
//! it is a first-party Rust `MetadataSource` carrying only `METADATA_LYRICS`,
//! so it shows up in the plugin management page as a usable (non-stub) plugin
//! without ever being selectable as the active *search* source.
//!
//! API: `GET https://lrclib.net/api/get?artist_name=&track_name=&album_name=`
//! returns a JSON array; each entry has `plainLyrics` and `syncedLyrics`
//! (LRC). We prefer the synced form and parse it into `LyricLine`s.

use crate::error::AppError;
use crate::metadata::{Lyrics, LyricLine, MetadataSource, MetadataTrack};
use crate::plugin::{Plugin, PluginCapabilities, PluginInfo};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::OnceLock;

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const BASE: &str = "https://lrclib.net/api/get";

pub struct Lrclib;

impl Lrclib {
    pub fn new() -> Self {
        Self
    }
}

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(UA)
            .build()
            .expect("failed to build http client")
    })
}

/// Parse an LRC timestamp tag (`MM:SS.xx` or `MM:SS.xxx`) into milliseconds.
fn parse_lrc_time(tag: &str) -> Option<i64> {
    let (mm_ss, frac) = tag.split_once('.')?;
    let mut parts = mm_ss.split(':');
    let mm = parts.next()?.parse::<i64>().ok()?;
    let ss = parts.next()?.parse::<i64>().ok()?;
    let frac = frac.parse::<i64>().ok()?;
    // 2-digit centiseconds → ms; 3-digit already ms.
    let frac_ms = if frac >= 100 { frac } else { frac * 10 };
    Some(mm * 60_000 + ss * 1_000 + frac_ms)
}

/// Parse synced LRC text into `LyricLine`s (handles multiple `[..]` tags per
/// line and sorts by time).
fn parse_synced(s: &str) -> Vec<LyricLine> {
    let mut lines: Vec<LyricLine> = Vec::new();
    for raw in s.lines() {
        let bytes = raw.as_bytes();
        let mut idx = 0usize;
        let mut times: Vec<i64> = Vec::new();
        while idx < bytes.len() && bytes[idx] == b'[' {
            let end = match raw[idx..].find(']') {
                Some(e) => e,
                None => break,
            };
            let tag = &raw[idx + 1..idx + end];
            if let Some(ms) = parse_lrc_time(tag) {
                times.push(ms);
            }
            idx += end + 1;
        }
        let text = raw[idx..].trim().to_string();
        for t in times {
            lines.push(LyricLine {
                time_ms: t,
                text: text.clone(),
            });
        }
    }
    lines.sort_by_key(|l| l.time_ms);
    lines
}

/// Fallback: split plain lyrics by newline into zero-timestamp lines.
fn parse_plain(s: &str) -> Vec<LyricLine> {
    s.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(|l| LyricLine {
            time_ms: 0,
            text: l.to_string(),
        })
        .collect()
}

impl Plugin for Lrclib {
    fn info(&self) -> PluginInfo {
        PluginInfo::from(
            "lrclib",
            "LrcLib 歌词",
            env!("CARGO_PKG_VERSION"),
            "免费无鉴权歌词库（lrclib.net）。仅提供歌词，不参与搜索/元数据。",
            PluginCapabilities::METADATA_LYRICS,
        )
    }
}

#[async_trait]
impl MetadataSource for Lrclib {
    async fn search(&self, _query: &str, _page: u32) -> Result<Vec<MetadataTrack>, AppError> {
        Err(AppError::msg("LrcLib 为歌词源，不支持搜索"))
    }

    async fn get_track(&self, _id: &str) -> Result<MetadataTrack, AppError> {
        Err(AppError::msg("LrcLib 为歌词源，不支持取单曲元数据"))
    }

    async fn get_lyrics(&self, track: &MetadataTrack) -> Result<Lyrics, AppError> {
        let resp = client()
            .get(BASE)
            .query(&[
                ("artist_name", track.artist.as_str()),
                ("track_name", track.title.as_str()),
                ("album_name", track.album.as_str()),
            ])
            .send()
            .await
            .map_err(|e| AppError::msg(format!("LrcLib 请求失败：{e}")))?;
        let text = resp
            .text()
            .await
            .map_err(|e| AppError::msg(format!("LrcLib 读取失败：{e}")))?;
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| AppError::msg(format!("LrcLib 响应解析失败：{e}")))?;
        let arr = match v.as_array() {
            Some(a) if !a.is_empty() => a,
            _ => return Ok(Lyrics { lines: vec![] }),
        };
        // Prefer the first entry with synced lyrics; fall back to plain.
        for entry in arr {
            if let Some(synced) = entry["syncedLyrics"].as_str() {
                if !synced.trim().is_empty() {
                    let lines = parse_synced(synced);
                    if !lines.is_empty() {
                        return Ok(Lyrics { lines });
                    }
                }
            }
        }
        for entry in arr {
            if let Some(plain) = entry["plainLyrics"].as_str() {
                if !plain.trim().is_empty() {
                    return Ok(Lyrics {
                        lines: parse_plain(plain),
                    });
                }
            }
        }
        Ok(Lyrics { lines: vec![] })
    }
}
