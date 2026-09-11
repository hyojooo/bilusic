//! Built-in `bilibili` audio engine plugin.
//!
//! Resolves a Bilibili bvid into a playable audio stream URL. Prefers the
//! DASH audio track (audio-only, bandwidth-efficient) and falls back to the
//! legacy `durl` MP4 when DASH is unavailable.
//!
//! In the Spotube model this is the analogue of YouTube: it carries the
//! actual audio bytes. Metadata lives in a separate plugin.

use crate::engine::{AudioCandidate, AudioEngine, StreamInfo};
use crate::error::AppError;
use crate::metadata::MetadataTrack;
use crate::plugin::{Plugin, PluginCapabilities, PluginInfo};
use crate::wbi;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::OnceLock;
use tokio::sync::Mutex;

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const NAV: &str = "https://api.bilibili.com/x/web-interface/nav";
const SPI: &str = "https://api.bilibili.com/x/frontend/finger/spi";
const VIEW: &str = "https://api.bilibili.com/x/web-interface/wbi/view";
const PLAYURL: &str = "https://api.bilibili.com/x/player/wbi/playurl";
const SEARCH: &str = "https://api.bilibili.com/x/web-interface/wbi/search/type";

// Browser-equivalent Referer / Origin pairs. B 站 silently returns empty data
// (or `data: null`) for WBI requests without proper Referer — this is the
// #1 reason anonymous search "works in the browser but not in our client."
const REFERER_VIDEO: &str = "https://www.bilibili.com/";
const ORIGIN_VIDEO: &str = "https://www.bilibili.com";
const REFERER_SEARCH: &str = "https://search.bilibili.com/";
const ORIGIN_SEARCH: &str = "https://search.bilibili.com";

pub struct BilibiliEngine {
    cookie: Mutex<Option<String>>,
}

impl BilibiliEngine {
        pub fn new() -> Self {
        Self {
            cookie: Mutex::new(None),
        }
    }

    pub async fn set_cookie(&self, cookie: Option<String>) {
        *self.cookie.lock().await = cookie;
    }
}

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .cookie_store(true)
            .user_agent(UA)
            .build()
            .expect("failed to build http client")
    })
}

/// Attach browser-equivalent Referer + Origin headers. B 站 WBI endpoints
/// silently empty their responses (or return `data: null`) when these are
/// missing — that's the difference between browser anonymous search "just
/// working" and our Rust client getting zero hits.
fn with_browser_headers(
    builder: reqwest::RequestBuilder,
    endpoint: &str,
) -> reqwest::RequestBuilder {
    let (referer, origin) = if endpoint == SEARCH {
        (REFERER_SEARCH, ORIGIN_SEARCH)
    } else {
        (REFERER_VIDEO, ORIGIN_VIDEO)
    };
    builder.header("Referer", referer).header("Origin", origin)
}

/// Surface B 站's real failure modes. The API silently returns
/// `{code: 0, data: {result: []}}` or `{code: -352, data: null}` when
/// something's wrong — without this guard we'd mistake "rejected" for "no
/// results". Matches the same check the metadata plugin already does.
fn check_bili(json: &Value) -> Result<(), AppError> {
    if json["data"]["v_voucher"].is_string() {
        return Err(AppError::new(
            403,
            "B 站风控拦截：搜索接口要求 v_voucher 验证。请到「设置 → 音源」填入 SESSDATA 后重试。",
        ));
    }
    let code = json["code"].as_i64().unwrap_or(0);
    if code != 0 {
        let msg = json["message"].as_str().unwrap_or("(no message)");
        return Err(AppError::msg(format!(
            "B 站接口拒绝：code={code}, message={msg}"
        )));
    }
    Ok(())
}

fn https_cover(pic: &str) -> String {
    pic.replace("http://", "https://")
}

async fn wbi_keys(c: &reqwest::Client) -> Result<(String, String), AppError> {
    let spi = c.get(SPI).send().await?.json::<Value>().await?;
    if let (Some(b3), Some(b4)) = (spi["data"]["b_3"].as_str(), spi["data"]["b_4"].as_str()) {
        let _ = c
            .get("https://api.bilibili.com/")
            .header("Cookie", format!("buvid3={b3}; buvid4={b4}"))
            .send()
            .await;
    }
    let nav = c.get(NAV).send().await?.json::<Value>().await?;
    let img = nav["data"]["wbi_img"]["img_url"]
        .as_str()
        .unwrap_or("")
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim_end_matches(".png")
        .to_string();
    let sub = nav["data"]["wbi_img"]["sub_url"]
        .as_str()
        .unwrap_or("")
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim_end_matches(".png")
        .to_string();
    Ok((img, sub))
}

/// Fetch a video's `cid` + cover + title + author given a bvid.
async fn view_meta(c: &reqwest::Client, bvid: &str, (img, sub): &(String, String), cookie: Option<&str>) -> Result<ViewMeta, AppError> {
    let url = wbi::signed_url(VIEW, vec![("bvid".into(), bvid.into())], img, sub);
    let mut req = with_browser_headers(c.get(&url), VIEW);
    if let Some(ck) = cookie.filter(|s| !s.is_empty()) {
        req = req.header("Cookie", ck);
    }
    let json = req.send().await?.json::<Value>().await?;
    check_bili(&json)?;
    let data = &json["data"];
    if data.is_null() {
        return Err(AppError::msg(format!("找不到视频 {}", bvid)));
    }
    Ok(ViewMeta {
        cid: data["cid"].as_i64().ok_or_else(|| AppError::msg("无法获取视频 cid"))?,
        title: data["title"].as_str().unwrap_or("").to_string(),
        author: data["owner"]["name"].as_str().unwrap_or("").to_string(),
        cover: https_cover(data["pic"].as_str().unwrap_or("")),
        duration_sec: data["duration"].as_i64().unwrap_or(0),
    })
}

struct ViewMeta {
    cid: i64,
    title: String,
    author: String,
    cover: String,
    duration_sec: i64,
}

async fn pick_best_audio(
    c: &reqwest::Client,
    bvid: &str,
    cid: i64,
    (img, sub): &(String, String),
    cookie: Option<&str>,
) -> Result<StreamInfo, AppError> {
    let url = wbi::signed_url(
        PLAYURL,
        vec![
            ("bvid".into(), bvid.into()),
            ("cid".into(), cid.to_string()),
            ("qn".into(), "64".into()),
            ("fnval".into(), "16".into()),
            ("fourk".into(), "1".into()),
        ],
        img,
        sub,
    );
    let mut req = with_browser_headers(c.get(&url), PLAYURL);
    if let Some(ck) = cookie.filter(|s| !s.is_empty()) {
        req = req.header("Cookie", ck);
    }
    let json = req.send().await?.json::<Value>().await?;
    check_bili(&json)?;

    let play = &json["data"];
    if play.is_null() {
        return Err(AppError::msg("playurl 返回为空，可能需要登录 Cookie"));
    }

    // DASH audio track — pick highest bandwidth.
    if let Some(audio) = play["dash"]["audio"].as_array() {
        if !audio.is_empty() {
            let mut best = &audio[0];
            for a in audio {
                if a["bandwidth"].as_i64().unwrap_or(0) > best["bandwidth"].as_i64().unwrap_or(0) {
                    best = a;
                }
            }
            let url = best["baseUrl"]
                .as_str()
                .or_else(|| best["base_url"].as_str())
                .map(|s| s.to_string());
            let mime = best["mimeType"]
                .as_str()
                .or_else(|| best["mime_type"].as_str())
                .unwrap_or("audio/mp4");
            if let Some(url) = url {
                return Ok(StreamInfo {
                    url,
                    mime: mime.to_string(),
                    bitrate: best["bandwidth"].as_u64().unwrap_or(0) as u32,
                    duration_ms: 0,
                    cover: String::new(),
                    title: String::new(),
                    artist: String::new(),
                });
            }
        }
    }

    // Legacy `durl` MP4 fallback.
    if let Some(durl) = play["durl"].as_array() {
        if let Some(u) = durl.first().and_then(|d| d["url"].as_str()) {
            return Ok(StreamInfo {
                url: u.to_string(),
                mime: "video/mp4".into(),
                bitrate: durl[0]["size"].as_u64().unwrap_or(0) as u32,
                duration_ms: 0,
                cover: String::new(),
                title: String::new(),
                artist: String::new(),
            });
        }
    }

    Err(AppError::msg("未找到可用的音频流地址（可能需要登录 Cookie 或该视频受限）"))
}

impl Plugin for BilibiliEngine {
    fn info(&self) -> PluginInfo {
        PluginInfo::from(
            "bilibili",
            "Bilibili",
            env!("CARGO_PKG_VERSION"),
            "B 站 DASH 音频 / MP4 durl。匿名可用，可能受限；填 SESSDATA 获得完整画质与权限。",
            PluginCapabilities::AUDIO_BY_ID
                | PluginCapabilities::AUDIO_BY_QUERY
                | PluginCapabilities::AUDIO_NEEDS_AUTH,
        )
    }
}

#[async_trait]
impl AudioEngine for BilibiliEngine {
    async fn find_stream_by_id(&self, id: &str) -> Result<StreamInfo, AppError> {
        let c = client();
        let keys = wbi_keys(c).await?;
        let cookie = self.cookie.lock().await.clone();
        let meta = view_meta(c, id, &keys, cookie.as_deref()).await?;
        let mut info = pick_best_audio(c, id, meta.cid, &keys, cookie.as_deref()).await?;
        info.title = meta.title;
        info.artist = meta.author;
        info.cover = meta.cover;
        info.duration_ms = meta.duration_sec * 1000;
        Ok(info)
    }

    async fn find_stream_by_query(
        &self,
        _query: &str,
        hint: &MetadataTrack,
    ) -> Result<StreamInfo, AppError> {
        // Cross-source path: search Bilibili for the song by `title + artist`,
        // rank candidates by song-likeness, and try the top few until one
        // resolves to a playable audio stream. The raw `query` is ignored in
        // favour of the structured `hint` — much more reliable than whatever
        // the user typed in the search bar.
        let c = client();
        let keys = wbi_keys(c).await?;
        let cookie = self.cookie.lock().await.clone();
        let keyword = build_keyword(hint);

        let candidates = search_videos(c, &keyword, &keys, cookie.as_deref(), 10, false).await?;
        if candidates.is_empty() {
            return Err(AppError::msg(format!(
                "跨源搜索「{keyword}」无结果（B 站接口返回 200 但 result 为空，可能是 B 站限流或视频被删）"
            )));
        }

        // Rank, in order:
        //  ① candidates that name BOTH the song and the artist (true matches
        //     of "the artist singing the song", above covers / live versions
        //     that only name one of them),
        //  ② "original / official" quality — penalize instrumental / cover /
        //     live uploads (纯音乐 / 伴奏 / 翻唱 / 现场 / …) and boost MV /
        //     official / 原唱 / 专辑, so a genuine studio track is preferred
        //     over a same-name 纯音乐 or 翻唱 that also names both tokens,
        //  ③ raw relevance (title/UP token hits),
        //  ④ song-likeness (45s–10min) as the final tie-break.
        let mut ranked = candidates;
        ranked.sort_by(|a, b| {
            matches_all_tokens(&keyword, b)
                .cmp(&matches_all_tokens(&keyword, a))
                .then_with(|| source_quality_score(b).cmp(&source_quality_score(a)))
                .then_with(|| relevance_score(&keyword, b).cmp(&relevance_score(&keyword, a)))
                .then_with(|| {
                    let sa = if (45..=600).contains(&a.duration_sec) { 0 } else { 1 };
                    let sb = if (45..=600).contains(&b.duration_sec) { 0 } else { 1 };
                    sa.cmp(&sb)
                })
        });

        let mut last_err: Option<AppError> = None;
        for r in ranked.iter().take(3) {
            match try_resolve(c, &r.bvid, &keys, cookie.as_deref()).await {
                Ok(info) => return Ok(overlay_hint(info, hint)),
                Err(e) => last_err = Some(e),
            }
        }

        Err(last_err.unwrap_or_else(|| {
            AppError::msg(format!("跨源搜索「{keyword}」前 3 个候选均无法解析音频流"))
        }))
    }

    async fn search(&self, query: &str) -> Result<Vec<AudioCandidate>, AppError> {
        // Player bar "其他音源" picker: list B 站 videos matching the song
        // name without resolving streams. The frontend then resolves the
        // chosen one via `play_by_bvid`.
        let c = client();
        let keys = wbi_keys(c).await?;
        let cookie = self.cookie.lock().await.clone();
        search_videos(c, query, &keys, cookie.as_deref(), 20, true).await
    }
}

/// Search B 站 videos for `keyword` and parse the raw results into
/// [`AudioCandidate`]s. Shared by both `find_stream_by_query` (which ranks +
/// resolves the top few) and `search` (which lists them for the picker).
///
/// `refine` toggles the post-processing layer (`refine_candidates`): the
/// picker wants a clean, stable, song-oriented list (dedup + duration
/// filter + relevance sort), while `find_stream_by_query` keeps its own
/// ranking and must not be over-filtered (a song might only have a
/// non-song-length upload available).
async fn search_videos(
    c: &reqwest::Client,
    keyword: &str,
    keys: &(String, String),
    cookie: Option<&str>,
    limit: usize,
    refine: bool,
) -> Result<Vec<AudioCandidate>, AppError> {
    let url = wbi::signed_url(
        SEARCH,
        vec![
            ("search_type".into(), "video".into()),
            ("keyword".into(), keyword.to_string()),
            ("page".into(), "1".into()),
        ],
        &keys.0,
        &keys.1,
    );
    let mut req = with_browser_headers(c.get(&url), SEARCH);
    if let Some(ck) = cookie.filter(|s| !s.is_empty()) {
        req = req.header("Cookie", ck);
    }
    let json = req.send().await?.json::<Value>().await?;
    check_bili(&json)?;
    let arr = json["data"]["result"].as_array().ok_or_else(|| {
        AppError::msg(format!("搜索「{keyword}」无结果（B 站返回空）"))
    })?;

    let mut out = Vec::new();
    for r in arr.iter().take(limit) {
        let bvid = match r["bvid"].as_str() {
            Some(b) if !b.is_empty() => b.to_string(),
            _ => continue,
        };
        let title = strip_html(r["title"].as_str().unwrap_or(""));
        let author = match r["author"].as_str() {
            Some(a) => a.to_string(),
            None => r["author"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
        };
        let duration_sec = parse_duration(&r["duration"]);
        let cover = https_cover(r["pic"].as_str().unwrap_or(""));
        out.push(AudioCandidate {
            bvid,
            title,
            author,
            duration_sec,
            cover,
        });
    }
    if refine {
        Ok(refine_candidates(keyword, out))
    } else {
        Ok(out)
    }
}

/// Post-process raw search hits into a clean, stable, song-oriented list for
/// the "其他音源" picker:
///
/// 1. **Dedup by bvid** — the same audio uploaded by several UP 主 collapses
///    to one row (keeps the first / most-relevant occurrence).
/// 2. **Duration filter** — keep only `45s..=600s` (≈45s–10min). This drops
///    MVs that are actually films, full concerts, reactions, and other
///    non-song-length uploads so the picker shows actual songs.
/// 3. **Relevance sort** — rank by how many keyword tokens appear in the
///    title (primary) plus a smaller bonus when the UP 主 name contains a
///    keyword token (e.g. an official artist channel). Stable sort preserves
///    B 站's own order among ties, so the list doesn't jump around between
///    identical requests.
fn refine_candidates(keyword: &str, mut list: Vec<AudioCandidate>) -> Vec<AudioCandidate> {
    // 1. dedup by bvid (keep first occurrence)
    let mut seen = std::collections::HashSet::new();
    list.retain(|c| seen.insert(c.bvid.clone()));

    // 2. duration filter — songs only
    list.retain(|c| (45..=600).contains(&c.duration_sec));

    // 3. relevance sort (stable → ties keep B 站's order)
    list.sort_by(|a, b| relevance_score(keyword, b).cmp(&relevance_score(keyword, a)));
    list
}

/// Split a search keyword into lowercase tokens (whitespace-delimited).
/// For `"搁浅 周杰伦"` this yields `["搁浅", "周杰伦"]`.
fn keyword_tokens(keyword: &str) -> Vec<String> {
    keyword
        .split_whitespace()
        .map(|s| s.to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Relevance score for one candidate against the query keyword. Title token
/// hits weigh most (the song title literally appears in the video title),
/// then a smaller bonus if the UP 主 name itself contains a token (e.g. an
/// official artist channel). Higher = more likely the right song.
fn relevance_score(keyword: &str, cand: &AudioCandidate) -> i64 {
    let toks = keyword_tokens(keyword);
    if toks.is_empty() {
        return 0;
    }
    let title_l = cand.title.to_lowercase();
    let author_l = cand.author.to_lowercase();
    let mut score: i64 = 0;
    for tk in &toks {
        if title_l.contains(tk) {
            score += 10;
        }
        if author_l.contains(tk) {
            score += 3;
        }
    }
    score
}

/// Whether the candidate's title or UP 主 name covers *every* keyword token
/// (i.e. it names both the song title and the artist). Used as the top
/// ranking tier for automatic cross-source resolution so we prefer "the
/// artist singing the song" over high-play-count covers / live versions that
/// only name one of them. With a single-token keyword (e.g. artist-only) this
/// degrades to "the token is present at all".
fn matches_all_tokens(keyword: &str, cand: &AudioCandidate) -> bool {
    let toks = keyword_tokens(keyword);
    if toks.is_empty() {
        return false;
    }
    let hay = format!("{} {}", cand.title, cand.author).to_lowercase();
    toks.iter().all(|t| hay.contains(t))
}

/// Heuristic "is this the genuine studio/original track?" score, used as a
/// ranking tier for automatic cross-source resolution. Higher = more likely
/// the real song; negative = likely an instrumental / cover / live / remix
/// upload that we should prefer NOT to auto-select (but still keep as a
/// fallback so we never lose a playable source).
///
/// Bad markers dominate bad markers (instrumental / cover / live / remix),
/// otherwise we reward official / original markers (MV / 官方 / 原唱 / 专辑).
/// A plain title with neither scores 0 and stays neutral.
fn source_quality_score(cand: &AudioCandidate) -> i64 {
    let t = cand.title.to_lowercase();
    const BAD: &[&str] = &[
        "纯音乐", "伴奏", "钢琴版", "吉他版", "小提琴版", "instrumental", "bgm", "翻唱",
        "现场", "live", "cover", "remix", "dj",
    ];
    const GOOD: &[&str] = &["mv", "官方", "原唱", "专辑", "album"];
    if BAD.iter().any(|b| t.contains(*b)) {
        -50
    } else if GOOD.iter().any(|g| t.contains(*g)) {
        20
    } else {
        0
    }
}

/// Strip B 站's `<em class="keyword">…</em>` highlight tags (and a few common
/// HTML entities) from a search-result title.
fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .trim()
        .to_string()
}

/// Parse B 站's duration field, which may be an integer seconds value or a
/// `"mm:ss"` / `"h:mm:ss"` string.
fn parse_duration(v: &Value) -> i64 {
    if let Some(n) = v.as_i64() {
        return n;
    }
    if let Some(s) = v.as_str() {
        let parts: Vec<i64> = s.split(':').filter_map(|p| p.parse::<i64>().ok()).collect();
        return match parts.len() {
            2 => parts[0] * 60 + parts[1],
            3 => parts[0] * 3600 + parts[1] * 60 + parts[2],
            _ => 0,
        };
    }
    0
}

/// Build a B站 search keyword from a structured `MetadataTrack`.
/// `title + artist` is far better than either field alone.
fn build_keyword(hint: &MetadataTrack) -> String {
    let title = hint.title.trim();
    let artist = hint.artist.trim();
    match (title.is_empty(), artist.is_empty()) {
        (false, false) => format!("{title} {artist}"),
        (false, true) => title.to_string(),
        (true, false) => artist.to_string(),
        (true, true) => hint.source_id.clone(),
    }
}

/// Resolve a bvid all the way to a `StreamInfo` (view + playurl).
async fn try_resolve(
    c: &reqwest::Client,
    bvid: &str,
    keys: &(String, String),
    cookie: Option<&str>,
) -> Result<StreamInfo, AppError> {
    let meta = view_meta(c, bvid, keys, cookie).await?;
    let mut info = pick_best_audio(c, bvid, meta.cid, keys, cookie).await?;
    info.title = meta.title;
    info.artist = meta.author;
    info.cover = meta.cover;
    info.duration_ms = meta.duration_sec * 1000;
    Ok(info)
}

/// Overlay richer info from the source metadata hint when available, so the
/// player shows the QQ track's title/artist/cover rather than whatever
/// B站 returned.
fn overlay_hint(mut info: StreamInfo, hint: &MetadataTrack) -> StreamInfo {
    if !hint.title.is_empty() {
        info.title = hint.title.clone();
    }
    if !hint.artist.is_empty() {
        info.artist = hint.artist.clone();
    }
    if !hint.cover.is_empty() {
        info.cover = hint.cover.clone();
    }
    info
}
