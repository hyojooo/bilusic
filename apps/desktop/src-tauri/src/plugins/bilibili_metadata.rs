//! Built-in `bilibili` metadata source plugin.
//!
//! Searches Bilibili's public web-interface search API and returns normalized
//! `MetadataTrack`s. Audio comes from a separate engine plugin
//! (`bilibili_engine`).
//!
//! Bilibili recently started requiring a `v_voucher` challenge for anonymous
//! searches; the official (cookie-authenticated) backend usually bypasses it,
//! or the user can paste a bvid directly into the search bar to avoid the
//! issue.

use crate::error::AppError;
use crate::metadata::{Lyrics, MetadataSource, MetadataTrack};
use crate::plugin::{Plugin, PluginCapabilities, PluginInfo};
use crate::wbi;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::OnceLock;
use tokio::sync::Mutex;

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const NAV: &str = "https://api.bilibili.com/x/web-interface/nav";
const SPI: &str = "https://api.bilibili.com/x/frontend/finger/spi";
const SEARCH: &str = "https://api.bilibili.com/x/web-interface/wbi/search/type";

pub struct BilibiliMetadata {
    cookie: Mutex<Option<String>>,
}

impl BilibiliMetadata {
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

fn strip_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ => {
                if !in_tag {
                    out.push(ch);
                }
            }
        }
    }
    out
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

impl Plugin for BilibiliMetadata {
    fn info(&self) -> PluginInfo {
        PluginInfo::from(
            "bilibili",
            "Bilibili",
            env!("CARGO_PKG_VERSION"),
            "B 站搜索与元数据（标题/UP主/封面/时长/播放量）。匿名模式可能触发 v_voucher 风控。",
            PluginCapabilities::METADATA_SEARCH | PluginCapabilities::METADATA_GET,
        )
    }
}

#[async_trait]
impl MetadataSource for BilibiliMetadata {
    async fn search(&self, query: &str, page: u32) -> Result<Vec<MetadataTrack>, AppError> {
        let c = client();
        let (img, sub) = wbi_keys(c).await?;
        let url = wbi::signed_url(
            SEARCH,
            vec![
                ("search_type".into(), "video".into()),
                ("keyword".into(), query.into()),
                ("page".into(), page.to_string()),
            ],
            &img,
            &sub,
        );

        let cookie = self.cookie.lock().await.clone();
        let mut req = c.get(&url);
        if let Some(ck) = cookie.as_deref().filter(|s| !s.is_empty()) {
            req = req.header("Cookie", ck);
        }
        let resp = req.send().await?.json::<Value>().await?;

        if resp["data"]["v_voucher"].is_string() {
            return Err(AppError::new(
                403,
                "B 站风控拦截：搜索接口要求验证（v_voucher）。请在「设置 → Bilibili」勾选「官方登录」并填入 SESSDATA，或直接用 BV 号播放。",
            ));
        }

        let arr = resp["data"]["result"]
            .as_array()
            .ok_or_else(|| AppError::msg("搜索无结果或返回结构异常"))?;

        Ok(arr
            .iter()
            .filter_map(|r| {
                let bvid = r["bvid"].as_str()?.to_string();
                let title = strip_html(r["title"].as_str().unwrap_or(""));
                let author = r["author"].as_str().unwrap_or("").to_string();
                let duration_sec = r["duration"].as_i64().unwrap_or(0);
                let play_count = r["play"].as_i64().unwrap_or(0);
                let cover = https_cover(r["pic"].as_str().unwrap_or(""));
                Some(MetadataTrack {
                    source_id: bvid,
                    source: "bilibili".into(),
                    engine_hint: Some("bilibili".into()),
                    title,
                    artist: author,
                    album: format!("{} 次播放", play_count),
                    duration_ms: duration_sec * 1000,
                    cover,
                    lyrics_id: None,
                })
            })
            .collect())
    }

    async fn get_track(&self, id: &str) -> Result<MetadataTrack, AppError> {
        // Live lookup via wbi/view — keeps the metadata source authoritative.
        let (img, sub) = wbi_keys(client()).await?;
        let url = wbi::signed_url(
            "https://api.bilibili.com/x/web-interface/wbi/view",
            vec![("bvid".into(), id.into())],
            &img,
            &sub,
        );
        let cookie = self.cookie.lock().await.clone();
        let mut req = client().get(&url);
        if let Some(ck) = cookie.as_deref().filter(|s| !s.is_empty()) {
            req = req.header("Cookie", ck);
        }
        let v = req.send().await?.json::<Value>().await?;
        let data = &v["data"];
        if data.is_null() {
            return Err(AppError::msg(format!("找不到视频 {}", id)));
        }
        Ok(MetadataTrack {
            source_id: id.to_string(),
            source: "bilibili".into(),
            engine_hint: Some("bilibili".into()),
            title: data["title"].as_str().unwrap_or("").to_string(),
            artist: data["owner"]["name"].as_str().unwrap_or("").to_string(),
            album: format!(
                "{} 次播放 · {}",
                data["stat"]["view"].as_i64().unwrap_or(0),
                data["tname"].as_str().unwrap_or("")
            ),
            duration_ms: data["duration"].as_i64().unwrap_or(0) * 1000,
            cover: https_cover(data["pic"].as_str().unwrap_or("")),
            lyrics_id: None,
        })
    }

    async fn get_lyrics(&self, _track: &MetadataTrack) -> Result<Lyrics, AppError> {
        // Bilibili has no synced lyrics API for arbitrary videos. P7 will
        // optionally pair the bilibili metadata plugin with the
        // `lrclib` lyrics source to fill this in.
        Ok(Lyrics { lines: vec![] })
    }
}
