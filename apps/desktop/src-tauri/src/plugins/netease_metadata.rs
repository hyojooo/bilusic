//! Built-in `netease` metadata source — NetEase Cloud Music (网易云音乐).
//!
//! All NetEase endpoints (weapi encryption, request signing, response
//! decoding) are delegated to the [`ncm_api_rs`] SDK — a Rust port of
//! NeteaseCloudMusicApi Enhanced. We no longer hand-roll the AES-128-CBC +
//! RSA weapi scheme in this file; the SDK owns the crypto constants (the
//! part that silently breaks every clone) and the endpoint URIs.
//!
//! Endpoints used (all reached through `ncm_api_rs::ApiClient`):
//! - `cloudsearch`  → song search (`result.songs[]`).
//! - `song_detail`  → single-track metadata by id (`songs[]`).
//! - `lyric`        → synced + plain LRC lyrics (`lrc.lyric` / `lyric.lyric`).
//! - `playlist_detail` → playlist tracks (`playlist.tracks[]`).
//! - `personalized` / `personalized_newsong` → home-feed recommendations.
//!
//! Audio still resolves via the Bilibili engine (`engine_hint: "bilibili"`)
//! — the Spotube-style cross-source playback link.
//!
//! **Maintenance note:** if NetEase changes its encryption or an endpoint
//! 404s, the fix lives in `ncm_api_rs`, not here. Bump the crate version and
//! re-pin — don't re-implement weapi by hand.

use crate::error::AppError;
use crate::metadata::{
    FeedSong, HomeFeed, HomeSection, Lyrics, LyricLine, MetadataSource, MetadataTrack, SectionItem,
};
use crate::plugin::{Plugin, PluginCapabilities, PluginInfo};
use async_trait::async_trait;
use ncm_api_rs::{ApiClient, ApiResponse, CryptoType, NcmError, Query, RequestOption};
use serde_json::{json, Value};
use std::sync::OnceLock;

pub struct NeteaseMetadata;

impl NeteaseMetadata {
    pub fn new() -> Self {
        Self
    }

    /// A shared, lazily-initialized NetEase API client. `None` cookie = anonymous
    /// access (search / most metadata works; personalized feeds degrade
    /// gracefully). Reused across calls so reqwest keeps its connection pool.
    fn client() -> &'static ApiClient {
        static C: OnceLock<ApiClient> = OnceLock::new();
        C.get_or_init(|| ApiClient::new(None))
    }

    /// **Cross-source lyrics fallback**: when an active metadata source's
    /// `get_lyrics` returns empty (e.g. qq-music's trait default stub, since
    /// qq lyric requires VIP login), the Coordinator calls this entry point
    /// to search title+artist on netease and pull the LRC for the top hit.
    ///
    /// Coverage: ~90% of C-pop hits (Jay Chou / Mao Buyi / Lin Junjie /
    /// Eason Chan / …) have a netease entry with full synced lyrics, often
    /// including 翻译 (`lyric.tlyric`) — the netease API path returns synced
    /// LRC + plain + optional translation in the same payload.
    ///
    /// This deliberately bypasses the trait object so the active source
    /// doesn't need to know about other sources — cleanly supports adding
    /// future fallback sources by replicating this shape on each plugin
    /// without changing Coordinator flow.
    pub async fn lookup_lyrics(&self, title: &str, artist: &str) -> Result<Lyrics, AppError> {
        let query = format!("{} {}", title, artist);
        let data = serde_json::json!({
            "s": &query,
            "keywords": &query,
            "type": 1,
            "limit": 1,
            "offset": 0,
            "total": true,
        });
        let resp = weapi_request(Self::client(), "/api/cloudsearch/pc", data)
            .await
            .map_err(|e| AppError::msg(format!("网易云搜索失败：{e}")))?;
        let v = resp.body;
        let id = match v["result"]["songs"][0]["id"].as_i64() {
            Some(x) => x.to_string(),
            None => return Ok(Lyrics { lines: vec![] }),
        };
        let data2 = serde_json::json!({
            "id": &id,
            "tv": -1,
            "lv": -1,
            "rv": -1,
            "kv": -1,
            "_nmclfl": 1
        });
        let resp2 = weapi_request(Self::client(), "/api/song/lyric", data2)
            .await
            .map_err(|e| AppError::msg(format!("网易云歌词查询失败：{e}")))?;
        Ok(lyric_from_v(&resp2.body).unwrap_or(Lyrics { lines: vec![] }))
    }
}

// ── weapi helper ──────────────────────────────────────────────────────

/// Send a request via the **weapi** channel (`https://music.163.com/weapi/...`).
///
/// `ncm-api-rs`'s convenience methods (`cloudsearch`, `lyric`, …) default to
/// the **eapi** channel (`https://interface.music.163.com/eapi/...`), which
/// silently drops unauthenticated connections at the TCP layer — surfacing as
/// reqwest's `error sending request for url (.../eapi/cloudsearch/pc)` with no
/// HTTP response. Weapi is the Node.js NeteaseCloudMusicApi default and works
/// reliably for anonymous traffic. (`song_detail` / `lyric` / `playlist_detail`
/// already use weapi internally — only `cloudsearch` and the eapi-flavored
/// endpoints needed escaping.)
async fn weapi_request(
    client: &ApiClient,
    uri: &str,
    data: Value,
) -> Result<ApiResponse, NcmError> {
    let opts = RequestOption {
        crypto: CryptoType::Weapi,
        ..RequestOption::default()
    };
    client.request(uri, data, opts).await
}

// ── response helpers ───────────────────────────────────────────────────

fn https_url(url: &str) -> String {
    if url.starts_with("http://") {
        url.replacen("http://", "https://", 1)
    } else {
        url.to_string()
    }
}

/// Join an array of `{ name }` artists into `"A / B"`.
fn artists_str(ar: &Value) -> String {
    match ar {
        Value::Array(a) => a
            .iter()
            .filter_map(|s| s["name"].as_str())
            .collect::<Vec<_>>()
            .join(" / "),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

/// Convert a cloudsearch `songs[]` entry into `MetadataTrack`.
fn song_to_track(s: &Value) -> Option<MetadataTrack> {
    let id = s["id"].as_i64()?.to_string();
    Some(MetadataTrack {
        source_id: id,
        source: "netease".into(),
        engine_hint: Some("bilibili".into()),
        title: s["name"].as_str().unwrap_or("").to_string(),
        artist: artists_str(&s["ar"]),
        album: s["al"]["name"].as_str().unwrap_or("").to_string(),
        duration_ms: s["dt"].as_i64().unwrap_or(0),
        cover: https_url(s["al"]["picUrl"].as_str().unwrap_or("")),
        lyrics_id: Some(s["id"].as_i64()?.to_string()),
    })
}

/// Convert a `personalized/newsong` entry into `FeedSong`.
/// Convert a bare song object (as found in `playlist/tracks[]`) into `FeedSong`.
/// Mirrors `song_to_track` but drops the `MetadataTrack`-only fields (`album`,
/// `lyrics_id`) — the playlist detail page only needs the bare minimum to
/// render rows and resolve audio.
fn track_to_feed(s: &Value) -> Option<FeedSong> {
    let id = s["id"].as_i64()?.to_string();
    Some(FeedSong {
        source_id: id,
        title: s["name"].as_str().unwrap_or("").to_string(),
        artist: artists_str(&s["ar"]),
        cover: https_url(s["al"]["picUrl"].as_str().unwrap_or("")),
        duration_ms: s["dt"].as_i64().unwrap_or(0),
        source: "netease".into(),
        engine_hint: Some("bilibili".into()),
    })
}

// ── LRC parsing (mirrors lrclib.rs — NetEase returns the same LRC format) ──

fn parse_lrc_time(tag: &str) -> Option<i64> {
    let (mm_ss, frac) = tag.split_once('.')?;
    let mut parts = mm_ss.split(':');
    let mm = parts.next()?.parse::<i64>().ok()?;
    let ss = parts.next()?.parse::<i64>().ok()?;
    let frac = frac.parse::<i64>().ok()?;
    let frac_ms = if frac >= 100 { frac } else { frac * 10 };
    Some(mm * 60_000 + ss * 1_000 + frac_ms)
}

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

/// Pull a usable `Lyrics` from a `song/lyric` JSON response. Returns
/// `None` if the response is not OK / has neither synced nor plain LRC — the
/// caller treats that as "no lyrics available" (e.g. instrumental tracks,
/// censored by region, etc.).
///
/// Used by both the trait `get_lyrics` (which already has a netease `songid`
/// in `track.source_id`) and the cross-source `lookup_lyrics` helper.
fn lyric_from_v(v: &Value) -> Option<Lyrics> {
    if v["code"].as_i64() != Some(200) {
        return None;
    }
    if let Some(lrc) = v["lrc"]["lyric"].as_str() {
        if !lrc.trim().is_empty() {
            let lines = parse_synced(lrc);
            if !lines.is_empty() {
                return Some(Lyrics { lines });
            }
        }
    }
    if let Some(plain) = v["lyric"]["lyric"].as_str() {
        if !plain.trim().is_empty() {
            return Some(Lyrics {
                lines: parse_plain(plain),
            });
        }
    }
    None
}

// ── Home section helpers ─────────────────────────────────────────────
//
// 8 个并发区块（home() 用 tokio::join! 一次性抓取），每个函数独立返回
// Option<HomeSection>：网络失败/数据空就 None，不影响其它区块。

/// 个性化推荐新歌 — `personalized/newsong` 端点。
/// 单个榜单 — `toplist/detail` 端点，硬编码常用榜单 ID：
/// - 19723756: 飙升榜
/// - 3779629:  新歌榜
/// - 2884035:  原创榜
/// - 3778678:  热歌榜
async fn toplist_section(top_id: i64, section_id: &str, title: &str) -> Option<HomeSection> {
    // **重要**：NetEase `/weapi/toplist/detail` 返回的是**榜单目录**
    // (`{ id, name, coverImgUrl, updateTime, ... }`)，**不含 `tracks` 字段**。
    // 每个榜单在系统里就是一个特殊 playlist（`playlist_id == top_id`），
    // 要拿歌曲必须改调 `/api/v6/playlist/detail?id=<top_id>` 读 `playlist.tracks[]`。
    // 旧实现盲读 `entry["tracks"]` → `as_array()?` 短路 → 4 个榜单静默失踪。
    let q = Query::new().param("id", &top_id.to_string());
    let resp = match NeteaseMetadata::client().playlist_detail(&q).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "[netease] toplist {top_id} ({title}) playlist_detail failed: {e}"
            );
            return None;
        }
    };
    let tracks = match resp.body["playlist"]["tracks"].as_array() {
        Some(t) => t,
        None => {
            eprintln!(
                "[netease] toplist {top_id} ({title}) response missing playlist.tracks[]"
            );
            return None;
        }
    };
    let items: Vec<SectionItem> = tracks
        .iter()
        .take(20)
        .filter_map(|s| {
            let id = s["id"].as_i64()?.to_string();
            Some(SectionItem {
                id,
                title: s["name"].as_str().unwrap_or("").to_string(),
                subtitle: None,
                cover: https_url(s["al"]["picUrl"].as_str().unwrap_or("")),
                kind: "song".into(),
                source: "netease".into(),
                artist: Some(artists_str(&s["ar"])),
                duration_ms: Some(s["dt"].as_i64().unwrap_or(0)),
                engine_hint: Some("bilibili".into()),
            })
        })
        .collect();
    if items.is_empty() {
        return None;
    }
    Some(HomeSection {
        id: section_id.into(),
        title: title.into(),
        kind: "song".into(),
        items,
        hint: Some("每周更新".into()),
    })
}

/// 热门歌手 — `top/artists` 端点。匿名访问可用。
async fn artist_section() -> Option<HomeSection> {
    let data = json!({
        "limit": 30,
        "offset": 0,
        "total": true,
    });
    let resp = match weapi_request(NeteaseMetadata::client(), "/api/artist/top", data).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[netease] artist_section fetch failed: {e}");
            return None;
        }
    };
    let list = match resp.body["artists"].as_array() {
        Some(a) => a,
        None => {
            eprintln!("[netease] artist_section missing artists[]");
            return None;
        }
    };
    let items: Vec<SectionItem> = list
        .iter()
        .filter_map(|s| {
            let id = s["id"].as_i64()?.to_string();
            Some(SectionItem {
                id,
                title: s["name"].as_str().unwrap_or("").to_string(),
                subtitle: None,
                cover: https_url(s["picUrl"].as_str().unwrap_or("")),
                kind: "artist".into(),
                source: "netease".into(),
                artist: None,
                duration_ms: None,
                engine_hint: None,
            })
        })
        .collect();
    if items.is_empty() {
        return None;
    }
    Some(HomeSection {
        id: "artists".into(),
        title: "热门歌手".into(),
        kind: "card".into(),
        items,
        hint: None,
    })
}

/// 热门新碟 — 网易云"新碟上架"板块的「热门」子视图 (`/top/album?type=hot`)。
///
/// 实现要点：
/// - 真正的「热门新碟」是 `/top/album?type=hot`（`type=new` 才是"全部新碟"）。
/// - 走**裸 `weapi_request`** 而不是 SDK `top_album()`：SDK 默认会塞
///   `year/month/rcmd` 参数（基于客户端 `chrono::Utc::now()`），匿名场景下
///   NetEase 返回空 `albums[]`。这里只传 `type=hot` + area/limit/offset，
///   让服务端用默认时间窗口，匿名可用。
/// - 主请求失败/空时 **fallback 到 `album_new`（全部新碟）**，避免专辑区块空白。
async fn album_section() -> Option<HomeSection> {
    // 1) 主：热门新碟
    if let Some(s) = album_try(true).await {
        return Some(s);
    }
    // 2) 兜底：全部新碟（用户已验证此端点匿名可用，至少不空白）
    album_try(false).await
}

/// `hot=true`  → `/api/discovery/new/albums/area?type=hot`（热门新碟）
/// `hot=false` → `/api/album/new`（全部新碟，兜底）
async fn album_try(hot: bool) -> Option<HomeSection> {
    let (endpoint, data, sec_id, title, hint) = if hot {
        (
            "/api/discovery/new/albums/area",
            json!({ "area": "ALL", "limit": 30, "offset": 0, "type": "hot" }),
            "hot_albums",
            "热门新碟",
            "本周热门",
        )
    } else {
        (
            "/api/album/new",
            json!({ "area": "ALL", "limit": 30 }),
            "all_albums",
            "全部新碟",
            "近期发行",
        )
    };

    let resp = match weapi_request(NeteaseMetadata::client(), endpoint, data).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[netease] album_try(hot={hot}) fetch {endpoint} failed: {e}");
            return None;
        }
    };

    // 全部新碟端点返回 {weekData[], monthData[], albums[]}；热门端点返回 {albums[]}
    let list = if hot {
        resp.body["albums"].as_array()
    } else {
        resp.body["weekData"]
            .as_array()
            .or_else(|| resp.body["monthData"].as_array())
            .or_else(|| resp.body["albums"].as_array())
    };
    let list = match list {
        Some(a) if !a.is_empty() => a,
        _ => {
            eprintln!("[netease] album_try(hot={hot}) response missing albums[]");
            return None;
        }
    };

    let items: Vec<SectionItem> = list
        .iter()
        .take(20)
        .filter_map(|a| {
            let id = a["id"].as_i64()?.to_string();
            let artist = a["artists"]
                .as_array()
                .and_then(|arr| arr.first())
                .and_then(|au| au["name"].as_str())
                .unwrap_or("")
                .to_string();
            Some(SectionItem {
                id,
                title: a["name"].as_str().unwrap_or("").to_string(),
                subtitle: if artist.is_empty() { None } else { Some(artist) },
                cover: https_url(a["picUrl"].as_str().unwrap_or("")),
                kind: "album".into(),
                source: "netease".into(),
                artist: None,
                duration_ms: None,
                engine_hint: None,
            })
        })
        .collect();
    if items.is_empty() {
        return None;
    }
    Some(HomeSection {
        id: sec_id.into(),
        title: title.into(),
        kind: "card".into(),
        items,
        hint: Some(hint.into()),
    })
}

/// 推荐歌单 — `personalized` 端点。
async fn playlist_section() -> Option<HomeSection> {
    let data = json!({
        "limit": 20,
        "total": true,
        "n": 1000,
    });
    let resp = match weapi_request(
        NeteaseMetadata::client(),
        "/api/personalized/playlist",
        data,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[netease] playlist_section fetch failed: {e}");
            return None;
        }
    };
    let list = match resp.body["result"].as_array() {
        Some(a) => a,
        None => {
            eprintln!("[netease] playlist_section missing result[]");
            return None;
        }
    };
    let items: Vec<SectionItem> = list
        .iter()
        .filter_map(|p| {
            let id = p["id"].as_i64()?.to_string();
            Some(SectionItem {
                id,
                title: p["name"].as_str().unwrap_or("").to_string(),
                subtitle: p["copywriter"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string()),
                cover: https_url(p["picUrl"].as_str().unwrap_or("")),
                kind: "playlist".into(),
                source: "netease".into(),
                artist: None,
                duration_ms: None,
                engine_hint: None,
            })
        })
        .collect();
    if items.is_empty() {
        return None;
    }
    Some(HomeSection {
        id: "playlists".into(),
        title: "推荐歌单".into(),
        kind: "card".into(),
        items,
        hint: None,
    })
}

// ── Plugin trait impl ──────────────────────────────────────────────────

impl Plugin for NeteaseMetadata {
    fn info(&self) -> PluginInfo {
        PluginInfo::from(
            "netease",
            "Netease Cloud",
            env!("CARGO_PKG_VERSION"),
            "netease元数据：搜索、单曲、歌词、歌单、推荐。weapi 加密由 ncm-api-rs 提供，音频由引擎提供。",
            PluginCapabilities::METADATA_SEARCH
                | PluginCapabilities::METADATA_GET
                | PluginCapabilities::METADATA_LYRICS
                | PluginCapabilities::METADATA_HOME,
        )
    }
}

// ── MetadataSource trait impl ──────────────────────────────────────────

#[async_trait]
impl MetadataSource for NeteaseMetadata {
    async fn search(&self, query: &str, page: u32) -> Result<Vec<MetadataTrack>, AppError> {
        let offset = (page.saturating_sub(1)) * 20;
        let data = serde_json::json!({
            "s": query,
            "keywords": query,
            "type": 1,
            "limit": 20,
            "offset": offset,
            "total": true,
        });

        let resp = weapi_request(Self::client(), "/api/cloudsearch/pc", data)
            .await
            .map_err(|e| AppError::msg(format!("网易云搜索失败：{e}")))?;
        let v = resp.body;

        if v["code"].as_i64() != Some(200) {
            return Err(AppError::msg(format!(
                "网易云搜索失败：code={}",
                v["code"].as_i64().unwrap_or(-1)
            )));
        }

        let songs = v["result"]["songs"]
            .as_array()
            .ok_or_else(|| AppError::msg("网易云搜索无结果或返回异常"))?;

        Ok(songs.iter().filter_map(song_to_track).collect())
    }

    async fn get_track(&self, id: &str) -> Result<MetadataTrack, AppError> {
        let q = Query::new().param("ids", id);

        let resp = Self::client()
            .song_detail(&q)
            .await
            .map_err(|e| AppError::msg(format!("网易云单曲查询失败：{e}")))?;
        let v = resp.body;

        if v["code"].as_i64() != Some(200) {
            return Err(AppError::msg(format!(
                "网易云单曲查询失败：code={}",
                v["code"].as_i64().unwrap_or(-1)
            )));
        }

        let song = v["songs"]
            .as_array()
            .and_then(|a| a.first())
            .ok_or_else(|| AppError::msg(format!("找不到歌曲 {id}")))?;

        song_to_track(song).ok_or_else(|| AppError::msg(format!("歌曲 {id} 字段缺失")))
    }

    async fn get_lyrics(&self, track: &MetadataTrack) -> Result<Lyrics, AppError> {
        let id = track
            .lyrics_id
            .as_deref()
            .or(Some(&track.source_id))
            .ok_or_else(|| AppError::msg("缺少歌曲 ID，无法获取歌词"))?;
        let q = Query::new().param("id", id);
        let resp = Self::client()
            .lyric(&q)
            .await
            .map_err(|e| AppError::msg(format!("网易云歌词查询失败：{e}")))?;
        // Same parsing rules as `lookup_lyrics`: prefer synced LRC, fall back
        // to plain text. Empty / non-OK responses return `Lyrics { vec![] }`
        // so callers (and the coordinator fallback chain) see "no lyrics"
        // uniformly.
        Ok(lyric_from_v(&resp.body).unwrap_or(Lyrics { lines: vec![] }))
    }

    async fn get_playlist(&self, id: &str) -> Result<Vec<FeedSong>, AppError> {
        let q = Query::new().param("id", id);
        let resp = Self::client()
            .playlist_detail(&q)
            .await
            .map_err(|e| AppError::msg(format!("网易云歌单查询失败：{e}")))?;
        let v = resp.body;

        if v["code"].as_i64() != Some(200) {
            // 404 / 401 / 460 (resource out of licence) → return empty rather
            // than bubbling an error, so the frontend can render "该歌单无
            // 可播放歌曲" without a scary error toast.
            return Ok(vec![]);
        }

        let tracks = match v["playlist"]["tracks"].as_array() {
            Some(t) => t,
            None => return Ok(vec![]),
        };
        Ok(tracks.iter().filter_map(track_to_feed).collect())
    }

    async fn home(&self) -> Result<HomeFeed, AppError> {
        // 并发抓取所有区块 —— 任何单个区块失败降级为 None，不影响其它区块。
        // 单区块最坏 ~15s 内返回（依赖 ncm-api-rs 内置 client 无超时；
        // SDK 的 weapi_request 走默认 reqwest builder）。如果某区块 hang 住
        // 拖累整体返回，后续可给 weapi_request 加 .timeout(15s) client 包装。
        //
        // 顺序由产品定义：
        //   1)  欧美R&B榜（特色榜单）
        //   2-6) 其余 5 个特色榜单（UK / 热歌 / 黑胶VIP / Billboard / Beatport）
        //   7)  热门歌手
        //   8)  新碟上架
        //   9)  推荐歌单（深度消费内容；放末尾更耐逛）
        //
        // **榜单 ID 选型**（硬编码常用榜的 playlist_id；榜单在 NetEase 里就是特殊 playlist，
        // playlist_id == top_id，由 `toplist_section` 通过 `playlist_detail` 拉取 tracks）：
        //   - 12225155968 欧美R&B榜
        //   - 180106   UK排行榜周榜
        //   - 3778678  热歌榜
        //   - 5458050495 黑胶VIP爱听榜（较新榜单，ID 可能随版本变化）
        //   - 60198    美国 Billboard Hot 100 榜
        //   - 3812895  Beatport 全球电子舞曲榜
        // 若某 ID 已被网易/下架或 ID 不对，对应 section 静默跳过。
        let (
            rnb,
            uk,
            hot,
            vinyl,
            billboard,
            beatport,
            artists,
            new_albums,
            playlists,
        ) = tokio::join!(
            toplist_section(12225155968, "rnb", "欧美R&B榜"),
            toplist_section(180106, "uk", "UK排行榜周榜"),
            toplist_section(3778678, "hot", "热歌榜"),
            toplist_section(5458050495, "vinyl_vip", "黑胶VIP爱听榜"),
            toplist_section(60198, "billboard", "美国Billboard榜"),
            toplist_section(3812895, "beatport", "Beatport全球电子舞曲榜"),
            artist_section(),
            album_section(),
            playlist_section(),
        );

        let mut sections: Vec<HomeSection> = Vec::new();
        if let Some(s) = rnb {
            sections.push(s);
        }
        if let Some(s) = uk {
            sections.push(s);
        }
        if let Some(s) = hot {
            sections.push(s);
        }
        if let Some(s) = vinyl {
            sections.push(s);
        }
        if let Some(s) = billboard {
            sections.push(s);
        }
        if let Some(s) = beatport {
            sections.push(s);
        }
        if let Some(s) = artists {
            sections.push(s);
        }
        if let Some(s) = new_albums {
            sections.push(s);
        }
        if let Some(s) = playlists {
            sections.push(s);
        }

        Ok(HomeFeed { sections })
    }
}

// ── Unit tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn song_to_track_parses_cloudsearch_entry() {
        let s = json!({
            "id": 1813826,
            "name": "test song",
            "ar": [{ "id": 1, "name": "Artist A" }, { "id": 2, "name": "Artist B" }],
            "al": { "id": 99, "name": "Album X", "picUrl": "http://p1.music.126.net/cover.jpg" },
            "dt": 240000
        });
        let t = song_to_track(&s).unwrap();
        assert_eq!(t.source_id, "1813826");
        assert_eq!(t.source, "netease");
        assert_eq!(t.title, "test song");
        assert_eq!(t.artist, "Artist A / Artist B");
        assert_eq!(t.album, "Album X");
        assert_eq!(t.duration_ms, 240000);
        assert_eq!(t.cover, "https://p1.music.126.net/cover.jpg");
        assert_eq!(t.engine_hint.as_deref(), Some("bilibili"));
        assert_eq!(t.lyrics_id.as_deref(), Some("1813826"));
    }

    #[test]
    fn parse_synced_handles_multiple_tags() {
        let lrc = "[00:01.00]Line 1\n[00:03.50][00:05.00]Shared\n[01:00.00]Line 3";
        let lines = parse_synced(lrc);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].time_ms, 1000);
        assert_eq!(lines[0].text, "Line 1");
        assert_eq!(lines[1].time_ms, 3500);
        assert_eq!(lines[1].text, "Shared");
        assert_eq!(lines[2].time_ms, 5000);
        assert_eq!(lines[2].text, "Shared");
        assert_eq!(lines[3].time_ms, 60000);
    }

    }

