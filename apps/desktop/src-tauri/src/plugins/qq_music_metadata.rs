//! Built-in `qq-music` metadata source plugin.
//!
//! Provides music metadata (search + home feed) from QQ Music's public web
//! endpoints. Audio is resolved elsewhere by the Bilibili audio engine, so
//! every track carries `engine_hint: "bilibili"` for cross-source playback —
//! the Spotube-style "QQ metadata → Bilibili audio" link.
//!
//! Two endpoint families are used:
//! - Legacy `c.y.qq.com/v8/fcg-bin/...` — JSONP-wrapped, no `sign` required.
//!   Only the toplist (`fcg_v8_toplist_cp.fcg`) and search
//!   (`soso/fcgi-bin/client_search_cp`) survivors remain here; the
//!   `commend_playlist` / `new_album_info` / `singer_cmd` legacy endpoints
//!   were all retired by QQ and now 404.
//! - Modern `u.y.qq.com/cgi-bin/musicu.fcg` — POST with JSON body, no
//!   `sign` required for the read-only modules we use (singer list, hot
//!   playlists, new albums). Returns pure JSON (no JSONP wrapper).
//!
//! Both families need a `Referer: https://y.qq.com/` header.

use crate::error::AppError;
use crate::metadata::{
    FeedSong, HomeFeed, HomeSection, LyricLine, Lyrics, MetadataSource, MetadataTrack, SectionItem,
};
use crate::plugin::{Plugin, PluginCapabilities, PluginInfo};
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use serde_json::Value;
use std::sync::OnceLock;
use std::time::Duration;

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const REFERER: &str = "https://y.qq.com/";

pub struct QqMusicMetadata;

impl QqMusicMetadata {
    pub fn new() -> Self {
        Self
    }
}

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(UA)
            // 关键护栏：任何单个 QQ 接口若 hang 住（网络抖动 / 风控无响应），
            // 没有 timeout 时请求会无限挂起，进而拖垮整个首页 home()（N.stop）。
            // 15s 足够正常响应，超时即失败、对应区块降级为空，不影响其它区块。
            .timeout(Duration::from_secs(15))
            .build()
            .expect("failed to build http client")
    })
}

/// QQ returns JSONP (`callback({...})`) for some endpoints and plain JSON for
/// others. Find the first `{` and last `}` and parse that — works for both.
fn strip_jsonp(s: &str) -> Option<Value> {
    let s = s.trim();
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&s[start..=end]).ok()
}

async fn get_json_with(base: &str, params: &[(&str, &str)]) -> Result<Value, AppError> {
    let resp = client()
        .get(base)
        .query(params)
        .header("Referer", REFERER)
        .send()
        .await
        .map_err(|e| AppError::msg(format!("QQ 请求失败：{e}")))?;
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::msg(format!("QQ 读取失败：{e}")))?;
    strip_jsonp(&text)
        .ok_or_else(|| AppError::msg("QQ 返回无法解析（接口可能已变更或被网络限制）"))
}

/// Parse LRC text into timed lines. Tolerates multiple timestamps per line
/// (`[00:12.34][00:13.34]text`) and skips metadata tags (`[ti:]`, `[ar:]`).
fn parse_lrc(lrc: &str) -> Vec<LyricLine> {
    let mut out = Vec::new();
    for line in lrc.lines() {
        let bytes = line.as_bytes();
        let mut idx = 0;
        let mut stamps: Vec<i64> = Vec::new();
        while idx + 1 < bytes.len() && bytes[idx] == b'[' {
            let close = match line[idx + 1..].find(']') {
                Some(c) => idx + 1 + c,
                None => break,
            };
            let tag = &line[idx + 1..close];
            match parse_lrc_time(tag) {
                Some(ms) => stamps.push(ms),
                None => break,
            }
            idx = close + 1;
        }
        let text = line[idx..].trim().to_string();
        if stamps.is_empty() || text.is_empty() {
            continue;
        }
        for t in stamps {
            out.push(LyricLine {
                time_ms: t,
                text: text.clone(),
            });
        }
    }
    out.sort_by_key(|l| l.time_ms);
    out
}

/// Parse a single LRC time tag `mm:ss.xx` (also `mm:ss.x` / `mm:ss.xxx`) into
/// milliseconds. Returns `None` for non-time tags like `ti:`, `ar:`, `al:`.
fn parse_lrc_time(tag: &str) -> Option<i64> {
    let (mm, rest) = tag.split_once(':')?;
    let mm: i64 = mm.parse().ok()?;
    let (ss, frac) = match rest.split_once('.') {
        Some((s, f)) => (s, f),
        None => (rest, "0"),
    };
    let ss: i64 = ss.parse().ok()?;
    let ms: i64 = match frac.len() {
        3 => frac.parse().ok()?,
        2 => frac.parse::<i64>().ok()? * 10,
        1 => frac.parse::<i64>().ok()? * 100,
        0 => 0,
        _ => {
            let mut t = frac.to_string();
            t.truncate(3);
            t.parse().ok()?
        }
    };
    Some(mm * 60_000 + ss * 1_000 + ms)
}

/// POST a JSON payload to a modern `u.y.qq.com/cgi-bin/musicu.fcg` style
/// endpoint. Returns pure JSON (no JSONP wrapper to strip). The caller
/// passes a pre-built payload (often a `comm` + module-method envelope)
/// and reads the matching module's response.
async fn post_json(base: &str, payload: &Value) -> Result<Value, AppError> {
    let resp = client()
        .post(base)
        .header("Referer", REFERER)
        .header("User-Agent", UA)
        .header("Content-Type", "application/json")
        .json(payload)
        .send()
        .await
        .map_err(|e| AppError::msg(format!("QQ 请求失败：{e}")))?;
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::msg(format!("QQ 读取失败：{e}")))?;
    serde_json::from_str(&text)
        .map_err(|e| AppError::msg(format!("QQ 返回无法解析：{e}")))
}

fn https_cover(pic: &str) -> String {
    pic.replace("http://", "https://")
}

fn album_cover(album_mid: &str) -> String {
    format!("https://y.gtimg.cn/music/photo_new/T002R300x300M000{album_mid}.jpg")
}

fn singer_cover(singer_mid: &str) -> String {
    format!("https://y.gtimg.cn/music/photo_new/T001R300x300M000{singer_mid}.jpg")
}

/// `singer` may be an array of `{name}` or a plain string.
fn singers_str(singer: &Value) -> String {
    match singer {
        Value::Array(a) => a
            .iter()
            .filter_map(|s| s["name"].as_str())
            .collect::<Vec<_>>()
            .join(" / "),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

/// Human-readable play count for a recommended playlist's subtitle,
/// e.g. `145_860_792` → `"1.4亿"` / `12_300` → `"1.2万"`.
fn format_play_count(n: i64) -> String {
    if n >= 100_000_000 {
        format!("{:.1}亿", n as f64 / 100_000_000.0)
    } else if n >= 10_000 {
        format!("{:.1}万", n as f64 / 10_000.0)
    } else {
        n.to_string()
    }
}

/// Fetch one of QQ Music's toplists (新歌榜/欧美/韩国/日本/抖音…) by `topid`.
/// Returns up to `page_size` songs with real album covers. Network/render
/// failures degrade to an empty list rather than bubbling up — a single chart
/// being unavailable shouldn't take down the whole home feed.
async fn fetch_toplist(topid: &str) -> Vec<FeedSong> {
    let v = match get_json_with(
        "https://c.y.qq.com/v8/fcg-bin/fcg_v8_toplist_cp.fcg",
        &[
            ("type", "top"),
            ("topid", topid),
            ("format", "json"),
            ("page_no", "1"),
            ("page_size", "20"),
            ("song_begin", "0"),
        ],
    )
    .await
    {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let list = match v["songlist"].as_array() {
        Some(l) => l,
        None => return vec![],
    };
    list.iter()
        .filter_map(|s| {
            let d = &s["data"];
            let songmid = d["songmid"].as_str()?.to_string();
            let albummid = d["albummid"].as_str().unwrap_or("").to_string();
            Some(FeedSong {
                source_id: songmid,
                title: d["songname"].as_str().unwrap_or("").to_string(),
                artist: singers_str(&d["singer"]),
                cover: if albummid.is_empty() {
                    String::new()
                } else {
                    album_cover(&albummid)
                },
                duration_ms: d["interval"].as_i64().unwrap_or(0) * 1000,
                source: "qq-music".into(),
                engine_hint: Some("bilibili".into()),
            })
        })
        .collect()
}

/// Wrap a QQ toplist as a `kind:"song"` home section. Returns `None` when
/// the fetch produced no rows (network down / topid retired) so callers can
/// `if let Some(sec) = ... .await` and a single dead chart can't take down
/// the whole feed.
async fn toplist_section(
    topid: &str,
    section_id: &str,
    title: &str,
    hint: Option<&str>,
) -> Option<HomeSection> {
    let songs = fetch_toplist(topid).await;
    if songs.is_empty() {
        return None;
    }
    let items: Vec<SectionItem> = songs
        .into_iter()
        .map(|s| SectionItem {
            id: s.source_id,
            title: s.title,
            subtitle: None,
            cover: s.cover,
            kind: "song".into(),
            source: s.source,
            artist: Some(s.artist),
            duration_ms: Some(s.duration_ms),
            engine_hint: s.engine_hint,
        })
        .collect();
    Some(HomeSection {
        id: section_id.into(),
        title: title.into(),
        kind: "song".into(),
        items,
        hint: hint.map(|h| h.to_string()),
    })
}

/// 推荐歌单 — 现代 musicu.fcg 端点（`playlist.HotRecommendServer.get_hot_recommend`）。
/// 返回 `Option<HomeSection>`：网络失败 / 无数据时为 `None`，不拖累其它区块。
/// 设计为 free async fn 以便 `home()` 用 `tokio::join!` 并发抓取。
async fn playlist_section() -> Option<HomeSection> {
    let payload = serde_json::json!({
        "comm": { "ct": 24, "cv": 0 },
        "recomPlaylist": {
            "module": "playlist.HotRecommendServer",
            "method": "get_hot_recommend",
            "param": { "async": 1, "cmd": 2 }
        }
    });
    let v = match post_json("https://u.y.qq.com/cgi-bin/musicu.fcg", &payload).await {
        Ok(v) => v,
        Err(_) => return None,
    };
    let list = v["recomPlaylist"]["data"]["v_hot"].as_array()?;
    let mut playlists: Vec<SectionItem> = Vec::new();
    for p in list {
        let id = match p["content_id"].as_i64() {
            Some(x) => x.to_string(),
            None => continue,
        };
        let listen = p["listen_num"].as_i64().unwrap_or(0);
        let subtitle = if listen > 0 {
            Some(format_play_count(listen))
        } else {
            p["username"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        };
        playlists.push(SectionItem {
            id,
            title: p["title"].as_str().unwrap_or("").to_string(),
            subtitle,
            cover: https_cover(p["cover"].as_str().unwrap_or("")),
            kind: "playlist".into(),
            source: "qq-music".into(),
            artist: None,
            duration_ms: None,
            engine_hint: None,
        });
    }
    if playlists.is_empty() {
        return None;
    }
    Some(HomeSection {
        id: "playlists".into(),
        title: "推荐歌单".into(),
        kind: "card".into(),
        items: playlists,
        hint: Some("编辑精选".into()),
    })
}

/// 数字专辑 — `QQMusic.MusichallServer.GetNewAlbum`。返回 `Option<HomeSection>`。
async fn album_section() -> Option<HomeSection> {
    let payload = serde_json::json!({
        "comm": { "ct": 24, "cv": 0 },
        "new_album": {
            "module": "QQMusic.MusichallServer",
            "method": "GetNewAlbum",
            "param": {
                "type": 0,
                "category": "-1",
                "genre": 0,
                "year": 1,
                "company": -1,
                "sort": 1,
                "start": 0,
                "end": 39
            }
        }
    });
    let v = match post_json("https://u.y.qq.com/cgi-bin/musicu.fcg", &payload).await {
        Ok(v) => v,
        Err(_) => return None,
    };
    let list = v["new_album"]["data"]["album_list"].as_array()?;
    let mut albums: Vec<SectionItem> = Vec::new();
    for a in list {
        let mid = match a["album"]["mid"].as_str() {
            Some(x) => x.to_string(),
            None => continue,
        };
        let name = a["album"]["name"].as_str().unwrap_or("").to_string();
        let artist = a["author"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|au| au["name"].as_str())
            .unwrap_or("")
            .to_string();
        albums.push(SectionItem {
            id: mid.clone(),
            title: name,
            subtitle: if artist.is_empty() { None } else { Some(artist) },
            cover: album_cover(&mid),
            kind: "album".into(),
            source: "qq-music".into(),
            artist: None,
            duration_ms: None,
            engine_hint: None,
        });
    }
    if albums.is_empty() {
        return None;
    }
    Some(HomeSection {
        id: "albums".into(),
        title: "数字专辑".into(),
        kind: "card".into(),
        items: albums,
        hint: Some("近期发行".into()),
    })
}

/// 热门歌手 — `Music.SingerListServer.get_singer_list`（全分类总榜）。
/// 返回 `Option<HomeSection>`。
async fn artist_section() -> Option<HomeSection> {
    let payload = serde_json::json!({
        "comm": { "ct": 24, "cv": 0 },
        "singerList": {
            "module": "Music.SingerListServer",
            "method": "get_singer_list",
            "param": {
                "area": -100,
                "sex": -100,
                "genre": -100,
                "index": -100,
                "sin": 0,
                "cur_page": 1
            }
        }
    });
    let v = match post_json("https://u.y.qq.com/cgi-bin/musicu.fcg", &payload).await {
        Ok(v) => v,
        Err(_) => return None,
    };
    let list = v["singerList"]["data"]["singerlist"].as_array()?;
    let mut artists: Vec<SectionItem> = Vec::new();
    for s in list {
        let mid = match s["singer_mid"].as_str() {
            Some(x) => x.to_string(),
            None => continue,
        };
        let cover = s["singer_pic"]
            .as_str()
            .filter(|p| !p.is_empty())
            .map(https_cover)
            .unwrap_or_else(|| singer_cover(&mid));
        artists.push(SectionItem {
            id: mid,
            title: s["singer_name"].as_str().unwrap_or("").to_string(),
            subtitle: None,
            cover,
            kind: "artist".into(),
            source: "qq-music".into(),
            artist: None,
            duration_ms: None,
            engine_hint: None,
        });
    }
    if artists.is_empty() {
        return None;
    }
    Some(HomeSection {
        id: "artists".into(),
        title: "热门歌手".into(),
        kind: "card".into(),
        items: artists,
        hint: Some("本周热度".into()),
    })
}

impl Plugin for QqMusicMetadata {
    fn info(&self) -> PluginInfo {
        PluginInfo::from(
            "qq-music",
            "QQ Music",
            env!("CARGO_PKG_VERSION"),
            "QQ 音乐元数据：搜索、歌单、新歌、专辑、歌手。音频由引擎提供。",
            PluginCapabilities::METADATA_SEARCH
                | PluginCapabilities::METADATA_GET
                | PluginCapabilities::METADATA_HOME,
        )
    }
}

#[async_trait]
impl MetadataSource for QqMusicMetadata {
    async fn search(&self, query: &str, page: u32) -> Result<Vec<MetadataTrack>, AppError> {
        let v = get_json_with(
            "https://c.y.qq.com/soso/fcgi-bin/client_search_cp",
            &[
                ("aggr", "1"),
                ("cr", "1"),
                ("flag_qc", "0"),
                ("p", &page.max(1).to_string()),
                ("n", "20"),
                ("w", query),
            ],
        )
        .await?;
        let list = v["data"]["song"]["list"]
            .as_array()
            .ok_or_else(|| AppError::msg("QQ 搜索无结果或返回异常"))?;
        Ok(list
            .iter()
            .filter_map(|s| {
                let songmid = s["songmid"].as_str()?.to_string();
                let title = s["songname"].as_str().unwrap_or("").to_string();
                let artist = singers_str(&s["singer"]);
                let albummid = s["albummid"].as_str().unwrap_or("").to_string();
                let cover = if albummid.is_empty() {
                    String::new()
                } else {
                    album_cover(&albummid)
                };
                let duration_ms = s["interval"].as_i64().unwrap_or(0) * 1000;
                Some(MetadataTrack {
                    source_id: songmid.clone(),
                    source: "qq-music".into(),
                    engine_hint: Some("bilibili".into()),
                    title,
                    artist,
                    album: s["albumname"].as_str().unwrap_or("").to_string(),
                    duration_ms,
                    cover,
                    lyrics_id: Some(songmid),
                })
            })
            .collect())
    }

    async fn get_track(&self, id: &str) -> Result<MetadataTrack, AppError> {
        // Reuse search with the songmid as the query — cheap and sufficient for
        // resolving a single known track back into metadata.
        let v = get_json_with(
            "https://c.y.qq.com/soso/fcgi-bin/client_search_cp",
            &[
                ("aggr", "1"),
                ("cr", "1"),
                ("flag_qc", "0"),
                ("p", "1"),
                ("n", "1"),
                ("w", id),
            ],
        )
        .await?;
        let list = v["data"]["song"]["list"]
            .as_array()
            .and_then(|a| a.first())
            .ok_or_else(|| AppError::msg(format!("找不到歌曲 {id}")))?;
        let albummid = list["albummid"].as_str().unwrap_or("").to_string();
        Ok(MetadataTrack {
            source_id: id.to_string(),
            source: "qq-music".into(),
            engine_hint: Some("bilibili".into()),
            title: list["songname"].as_str().unwrap_or("").to_string(),
            artist: singers_str(&list["singer"]),
            album: list["albumname"].as_str().unwrap_or("").to_string(),
            duration_ms: list["interval"].as_i64().unwrap_or(0) * 1000,
            cover: if albummid.is_empty() {
                String::new()
            } else {
                album_cover(&albummid)
            },
            lyrics_id: Some(id.to_string()),
        })
    }

    async fn get_lyrics(&self, track: &MetadataTrack) -> Result<Lyrics, AppError> {
        // QQ lyrics live behind the *public* `musicu.fcg` gateway
        // (`music.musichallSong.PlayLyricInfo`). This is NOT the legacy
        // `c.y.qq.com/lyric/fcgi-bin/*` endpoint that requires a login cookie —
        // the musicu gateway is open and only needs Referer + Origin headers.
        // The `lyric` field in the response is **base64-encoded LRC text**, not
        // plaintext. See N11r.
        //
        // Prefer `track.lyrics_id` (filled with the songMID at search / get_track
        // time), fall back to `source_id` (also the songMID for QQ).
        let songmid = track
            .lyrics_id
            .clone()
            .or_else(|| Some(track.source_id.clone()))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AppError::msg("QQ 歌词：缺少 songMID"))?;

        let body = serde_json::json!({
            "req_1": {
                "module": "music.musichallSong.PlayLyricInfo",
                "method": "GetPlayLyricInfo",
                "param": {
                    "songMID": songmid,
                    "songID": 0,
                }
            }
        });

        let resp = client()
            .post("https://u.y.qq.com/cgi-bin/musicu.fcg")
            .header("Referer", REFERER)
            .header("Origin", "https://y.qq.com")
            .header("Content-Type", "application/json; charset=utf-8")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| AppError::msg(format!("QQ 歌词请求失败：{e}")))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(AppError::msg(format!("QQ 歌词 HTTP 错误：{status}")));
        }
        let text = resp
            .text()
            .await
            .map_err(|e| AppError::msg(format!("QQ 歌词读取失败：{e}")))?;
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| AppError::msg(format!("QQ 歌词 JSON 解析失败：{e}")))?;

        let r1 = v
            .pointer("/req_1")
            .ok_or_else(|| AppError::msg("QQ 歌词：响应缺少 req_1"))?;
        if r1.get("code").and_then(|c| c.as_i64()) != Some(0) {
            return Err(AppError::msg("QQ 歌词：接口返回非 0 状态码"));
        }
        let data = r1
            .get("data")
            .ok_or_else(|| AppError::msg("QQ 歌词：响应缺少 data"))?;
        let lyric_b64 = data.get("lyric").and_then(|x| x.as_str()).unwrap_or("");
        if lyric_b64.is_empty() {
            return Err(AppError::msg("QQ 歌词：无歌词数据（该曲可能无版权词）"));
        }
        let lrc_bytes = general_purpose::STANDARD
            .decode(lyric_b64)
            .map_err(|e| AppError::msg(format!("QQ 歌词 base64 解码失败：{e}")))?;
        let lrc_text = String::from_utf8(lrc_bytes)
            .map_err(|e| AppError::msg(format!("QQ 歌词 UTF-8 解码失败：{e}")))?;

        let lines = parse_lrc(&lrc_text);
        if lines.is_empty() {
            return Err(AppError::msg("QQ 歌词：解析后无有效时间轴"));
        }
        Ok(Lyrics { lines })
    }

    async fn home(&self) -> Result<HomeFeed, AppError> {
        // 并发抓取所有区块 —— 取代之前的「顺序 await」。
        // 之前每个区块先后 await：只要 QQ 任意一个接口 hang 住（网络抖动 /
        // 风控无 ack），整个 home() 就永远不返回，前端首页永久转圈。
        // 现在用 `tokio::join!` 一次性并发；配合 `client()` 的 15s 超时，单个
        // 接口最坏 15s 后失败、对应区块降级为空，整体保证在 ~15s 内返回。
        let (
            new_songs,
            playlists,
            albums,
            artists,
            billboard,
            melon,
            uk,
            oricon,
            douyin,
        ) = tokio::join!(
            toplist_section(
                "27",
                "new_songs",
                "新歌首发",
                Some("每周更新"),
            ),
            playlist_section(),
            album_section(),
            artist_section(),
            toplist_section(
                "108",
                "billboard",
                "美国公告牌榜",
                Some("每周更新"),
            ),
            toplist_section(
                "129",
                "melon",
                "韩国 Melon 榜",
                Some("每周更新"),
            ),
            toplist_section(
                "107",
                "uk",
                "英国 UK 榜",
                Some("每周更新"),
            ),
            toplist_section(
                "105",
                "oricon",
                "日本公信榜",
                Some("每周更新"),
            ),
            toplist_section(
                "60",
                "douyin",
                "抖音热歌榜",
                Some("每周更新"),
            ),
        );

        let mut sections: Vec<HomeSection> = Vec::new();
        // 任意区块 None（接口失败 / 无数据）都优雅跳过，不影响其它区块。
        //
        // 顺序由产品定义：
        //   1) 热门新歌首发（追新 — 实时性强）
        //   2-7) 6 个榜单（Billboard / Melon / UK / Oricon / 抖音 — 也是追新）
        //   8) 热门歌手（编辑挑选；和榜单交接平稳）
        //   9-10) 推荐歌单 + 数字专辑（深度消费内容；放末尾更耐逛）
        if let Some(s) = new_songs { sections.push(s); }
        if let Some(s) = billboard { sections.push(s); }
        if let Some(s) = melon { sections.push(s); }
        if let Some(s) = uk { sections.push(s); }
        if let Some(s) = oricon { sections.push(s); }
        if let Some(s) = douyin { sections.push(s); }
        if let Some(s) = artists { sections.push(s); }
        if let Some(s) = playlists { sections.push(s); }
        if let Some(s) = albums { sections.push(s); }

        Ok(HomeFeed { sections })
    }


    async fn toplist(&self, topid: i32) -> Result<Vec<FeedSong>, AppError> {
        Ok(fetch_toplist(&topid.to_string()).await)
    }

    async fn get_playlist(&self, id: &str) -> Result<Vec<FeedSong>, AppError> {
        // `disstid` (numeric QQ playlist id from the home feed's `content_id`)
        // is fetched from the legacy `qzone/fcg-bin/fcg_ucc_getcdinfo_byids_cp.fcg`
        // JSONP endpoint. Returns `cdlist[0].songlist[]` with the same shape as
        // the toplist songs, so we reuse the same field-extraction pattern.
        // Failures (dead id, rate limit, signature rotation) degrade to empty
        // so a single broken playlist can't take down the whole detail page.
        let v = match get_json_with(
            "https://c.y.qq.com/qzone/fcg-bin/fcg_ucc_getcdinfo_byids_cp.fcg",
            &[
                ("type", "1"),
                ("json", "1"),
                ("utf8", "1"),
                ("onlysong", "0"),
                ("nosign", "1"),
                ("format", "json"),
                ("tpl", "1"),
                ("disstid", id),
            ],
        )
        .await
        {
            Ok(v) => v,
            Err(_) => return Ok(vec![]),
        };
        let list = match v["cdlist"].as_array().and_then(|a| a.first()) {
            Some(c) => c["songlist"].as_array(),
            None => return Ok(vec![]),
        };
        let Some(list) = list else {
            return Ok(vec![]);
        };
        Ok(list
            .iter()
            .filter_map(|s| {
                let songmid = s["songmid"].as_str()?.to_string();
                let albummid = s["albummid"].as_str().unwrap_or("").to_string();
                Some(FeedSong {
                    source_id: songmid.clone(),
                    title: s["songname"].as_str().unwrap_or("").to_string(),
                    artist: singers_str(&s["singer"]),
                    cover: if albummid.is_empty() {
                        String::new()
                    } else {
                        album_cover(&albummid)
                    },
                    duration_ms: s["interval"].as_i64().unwrap_or(0) * 1000,
                    source: "qq-music".into(),
                    engine_hint: Some("bilibili".into()),
                })
            })
            .collect())
    }
}
