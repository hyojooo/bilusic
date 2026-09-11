//! C · JS runtime escape hatch (feature = `js`).
//!
//! Plugins with `"runtime": "js"` are loaded through this module: each plugin
//! has a `.js` entry file (default `plugin.js`) that exports a single
//! `function main(inputJson)` returning JSON. The host calls back via
//! `bilusic.log(level, msg)` for diagnostics and a small set of host
//! functions registered as synchronous JS-callable closures:
//!
//! * `bilusic.httpGet(url, optionsJson) → { status, headers, body }`
//! * `bilusic.httpPost(url, bodyJson, optionsJson) → { status, headers, body }`
//! * `bilusic.b64decode_alphabet(str, alphabet, offset) → string`
//! * `bilusic.log(level, msg)` — print to host console
//!
//! ## rquickjs + Send/Sync caveat
//!
//! `rquickjs::Context` (and `Runtime`) hold raw `NonNull<JSContext>` pointers
//! that are **`!Send` and `!Sync`** — QuickJS's design. Our `Plugin` trait
//! requires `Arc<Self>: Send + Sync`, so we cannot wrap a long-lived Context
//! in the adapter. Instead we **rebuild the Context on every call** inside a
//! `spawn_blocking` thread:
//!
//! * `JsInstance` only stores the plugin source (`Arc<str>`, Send + Sync) and
//!   a small frozen boot record (id / name / version / capabilities bits /
//!   shared `HttpHost`).
//! * Each trait method spawns a blocking task, constructs a fresh
//!   `Runtime + Context`, evaluates the prelude + plugin module, registers the
//!   host globals (`__host_http_get`, `__host_http_post`,
//!   `__host_b64decode_alphabet`), calls `main(JSON.stringify(payload))`,
//!   and JSON-stringifies the result back to Rust.
//!
//! Rebuilding the context per call is cheap relative to the typical plugin
//! HTTP roundtrip and means we don't need any cross-thread state.
//!
//! ## Sync vs async host bindings
//!
//! The host bindings are **synchronous**: the spawn_blocking thread that runs
//! QuickJS already lives on tokio's blocking pool (default 512), and we
//! `Handle::block_on` inside it for the reqwest round-trip. This dodges the
//! rquickjs `Promise` + `Async<Fn>` machinery (which is well-tested but
//! heavier than we need) and keeps the JS surface plain. Plugins get a
//! natural blocking feel — `const resp = bilusic.httpGet(url, opts)` — with
//! HTTP roundtrips blocking exactly one blocking-pool thread at a time. If
//! profiling shows this matters, switching to `rquickjs::Async<Fn>` is a
//! local change.
//!
//! ## Plugin contract
//!
//! The plugin file MUST export `main` as a global function. Argument JSON
//! shape by call site (the `op` field is added in Phase IV so all five
//! endpoints can be disambiguated; legacy plugins that detect search by
//! `query !== undefined` / get_track by `id !== undefined` still work):
//!
//! * `search(query, page)` → `{"op":"search","query":"...","page": N}`
//!   * Expected return: `{"tracks":[{"source_id","title","artist",...}]}`
//!   * Bare array also accepted.
//! * `get_track(id)` → `{"op":"get_track","id":"..."}`
//!   * Expected return: `{"track":{...}}` or a bare MetadataTrack object.
//! * `get_lyrics(track)` → `{"op":"get_lyrics","track":{...full MetadataTrack...}}`
//!   * Expected return: `{"lyrics":{"lines":[{"time_ms":N,"text":"..."}]}}`
//!   * Bare array of lines also accepted. Empty `lines` array if no lyrics.
//! * `home()` → `{"op":"home"}`
//!   * Expected return: `{"home":{"playlists":[...],"new_songs":[...],"new_albums":[...],"artists":[...]}}`
//!   * Bare `{"playlists":...}` also accepted.
//! * `toplist(topid)` → `{"op":"toplist","topid":N}`
//!   * Expected return: `{"songs":[{"track":{...},"reason":"..."}]}` or bare array.

#![cfg(feature = "js")]

use crate::engine::AudioEngine;
use crate::error::AppError;
use crate::metadata::{FeedSong, HomeFeed, HomeSection, LyricLine, Lyrics, MetadataSource, MetadataTrack, SectionItem};
use crate::plugin::{Plugin, PluginCapabilities, PluginInfo};
use crate::plugin_host::PluginManifest;

use rquickjs::{Context, Function, Runtime};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;
use tokio::runtime::Handle;

/// Shared HTTP dependencies captured at boot time so the synchronous host
/// bindings can drive reqwest from inside a `spawn_blocking` thread.
///
/// * `client` — pooled HTTP client reused across all calls (connection
///   pool, default UA matching the built-in Rust plugins).
/// * `handle` — captured in an async context, then `block_on`'d inside the
///   blocking thread. The blocking thread itself has no tokio Handle by
///   default, so we must remember one before scheduling.
/// * `app_handle` — optional Tauri handle. When present, `bilusic.log(...)`
///   is also pushed to the webview's DevTools Console via
///   `WebviewWindow::eval` so the user can see diagnostics without
///   reaching for a terminal. Without it, logs only land in the host's
///   stderr (still useful in `tauri dev` but invisible to the webview).
///
/// Cheap to clone (Arc guts / AppHandle is itself cheaply cloneable):
/// stored in `JsPluginBoot` (also `Arc`'d).
pub struct HttpHost {
    pub client: reqwest::Client,
    pub handle: Handle,
    pub app_handle: Option<tauri::AppHandle>,
}

impl HttpHost {
    /// Build an HTTP host with the canonical UA + 15 s timeout used by the
    /// built-in Rust plugins. The tokio runtime handle is captured from
    /// `tauri::async_runtime::handle()` — Tauri's global, thread-agnostic
    /// accessor (it lazy-initializes a fresh tokio runtime if none exists
    /// and bypasses the thread-local `Handle::current()` limitation that
    /// bites the sync `.setup` closure on tao's main thread).
    ///
    /// Usable from anywhere: `init()` (sync, tao main), tokio workers,
    /// `spawn_blocking` threads, and tests.
    pub fn with_default_client() -> Self {
        // TokioHandle in `tauri::async_runtime` is just
        // `tokio::runtime::Handle` re-exported (see tauri::async_runtime
        // line: `pub use tokio::runtime::Handle as TokioHandle`), so passing
        // it through where a `tokio::runtime::Handle` is expected compiles
        // by type identity.
        let handle: tauri::async_runtime::TokioHandle =
            tauri::async_runtime::handle().inner().clone();
        let client = reqwest::Client::builder()
            .user_agent(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            )
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("failed to build default reqwest client");
        Self {
            client,
            handle,
            app_handle: None,
        }
    }

    /// Same as [`with_default_client`] but also captures the Tauri
    /// `AppHandle` so plugin log calls are mirrored to the webview's
    /// DevTools Console. Call from inside the `.setup` closure (only place
    /// an `AppHandle` is reachable synchronously).
    pub fn with_default_client_and_app(app: tauri::AppHandle) -> Self {
        let mut host = Self::with_default_client();
        host.app_handle = Some(app);
        host
    }
}

/// Frozen boot record for a JS plugin: everything the host needs to
/// instantiate the plugin metadata adapter without holding on to a
/// non-`Send` QuickJS context.
pub struct JsPluginBoot {
    pub source_id: String,
    pub name: String,
    pub version: String,
    /// Plugin's `description` from `plugin.json`. Used by `info()`. Empty
    /// string means the host falls back to a generic label (mirrors the
    /// "JS plugin: <name>" wording used for plugins that ship without a
    /// manifest-supplied description).
    pub description: String,
    pub capabilities: u32,
    /// Pre-baked helper script that defines `bilusic.log()` and the host
    /// function wrappers (`bilusic.httpGet`, `bilusic.httpPost`, ...).
    /// Inlined because the plugin's `bilusic` namespace has to be set up
    /// *before* the plugin module evaluates.
    pub prelude: String,
    /// Shared HTTP dependencies (`HttpHost`). Captured once so each
    /// `call_in_fresh_context` invocation can drive reqwest synchronously
    /// without juggling additional state through the plugin payload.
    pub http: Arc<HttpHost>,
}

/// `Send + Sync` wrapper around a JS plugin's source + boot info. Cheap to
/// clone (Arc<str>s + Arc boot); the heavy QuickJS context is built per-call.
pub struct JsInstance {
    /// Plugin source code as a `Send + Sync` Arc<str>. Wrapped as Arc so
    /// every `spawn_blocking` task gets a shared reference cheaply.
    source: Arc<str>,
    boot: Arc<JsPluginBoot>,
    /// Path to the JS entry file on disk (for diagnostics only). Not used to
    /// re-read the file — `source` is captured at boot time.
    #[allow(dead_code)]
    entry_path: PathBuf,
}

impl JsInstance {
    /// Read the JS source from disk and freeze the boot record. Actual
    /// QuickJS compilation happens lazily inside `call_in_fresh_context`.
    pub fn load(boot: JsPluginBoot, entry_path: PathBuf, source: String) -> Arc<Self> {
        Arc::new(Self {
            source: Arc::from(source),
            boot: Arc::new(boot),
            entry_path,
        })
    }
}

impl Plugin for JsMetadataSource {
    fn info(&self) -> PluginInfo {
        // Prefer the manifest-supplied description; fall back to a generic
        // JS-plugin label only when the author left plugin.json's
        // `description` blank. (wasm_runtime uses m.description.clone()
        // unconditionally — JS is no longer an outlier.)
        let description = if self.instance.boot.description.trim().is_empty() {
            format!("JS plugin: {}", self.instance.boot.name)
        } else {
            self.instance.boot.description.clone()
        };
        PluginInfo {
            id: self.instance.boot.source_id.clone(),
            name: self.instance.boot.name.clone(),
            version: self.instance.boot.version.clone(),
            description,
            capabilities: self.instance.boot.capabilities,
        }
    }
}

/// Metadata-source adapter that defers every call into a JS plugin.
pub struct JsMetadataSource {
    instance: Arc<JsInstance>,
}

impl JsMetadataSource {
    pub fn new(
        boot: JsPluginBoot,
        entry_path: PathBuf,
        module: String,
    ) -> Arc<Self> {
        Arc::new(Self {
            instance: JsInstance::load(boot, entry_path, module),
        })
    }

    /// Build a fresh QuickJS context, register host globals, evaluate the
    /// prelude + plugin module, then invoke `main(payloadJson)` returning a
    /// JSON string. Errors carry enough context to debug a malformed plugin
    /// without needing a JS-side stack trace parser.
    ///
    /// Contract: `main(input) → string`. The plugin is responsible for
    /// `JSON.stringify`'ing its result (and un-stringifying on the Rust
    /// side via serde_json). This dodges rquickjs 0.12's value-conversion
    /// surface area and keeps the JSON boundary 100% JS-controlled.
    fn call_in_fresh_context(
        instance: &JsInstance,
        payload_json: &str,
    ) -> Result<String, AppError> {
        let runtime = Runtime::new()
            .map_err(|e| AppError::msg(format!("JS runtime 初始化失败：{e}")))?;
        // rquickjs 0.12 renamed `Context::new` to `Context::full`/`Context::base`.
        // `full` registers the standard JS host functions (console / JSON /
        // Promise) which the plugin's own `JSON.stringify(...)` calls depend on.
        let ctx = Context::full(&runtime)
            .map_err(|e| AppError::msg(format!("JS context 创建失败：{e}")))?;
        let http = instance.boot.http.clone();
        ctx.with(|c| -> Result<String, AppError> {
            // 1. Register host bindings BEFORE the prelude evaluates, so the
            //    prelude's `globalThis.__host_http_get(...)` references can
            //    bind to them. Returns a JSON string `{status, headers, body}`.
            //    We use sync closures and `Handle::block_on` inside the
            //    spawn_blocking thread — see module docs.
            c.globals()
                .set("__host_http_get", make_http_get_fn(c.clone(), http.clone())?)
                .map_err(|e| AppError::msg(format!("注册 __host_http_get：{e}")))?;
            c.globals()
                .set("__host_http_post", make_http_post_fn(c.clone(), http.clone())?)
                .map_err(|e| AppError::msg(format!("注册 __host_http_post：{e}")))?;
            c.globals()
                .set(
                    "__host_b64decode_alphabet",
                    make_b64decode_alphabet_fn(c.clone())?,
                )
                .map_err(|e| AppError::msg(format!(
                    "注册 __host_b64decode_alphabet：{e}"
                )))?;
            // N11i: `bilusic.log` now flows through a host binding so we
            // can mirror the message into the webview's DevTools Console.
            // Without this, plugin `console.log(...)` calls (rquickjs's
            // built-in console) only land in the host stderr — invisible
            // inside the app. With the AppHandle we run
            // `WebviewWindow::eval("console.{level}(...)")` to surface
            // diagnostics where the user can see them.
            c.globals()
                .set(
                    "__host_log",
                    make_log_fn(c.clone(), http.app_handle.clone())?,
                )
                .map_err(|e| AppError::msg(format!("注册 __host_log：{e}")))?;

            // 2. Combine prelude + plugin source so the host's `bilusic`
            //    namespace is available before user code runs.
            let combined = format!("{}\n{}", instance.boot.prelude, instance.source);
            c.eval::<(), _>(combined.as_str())
                .map_err(|e| AppError::msg(format!("JS 插件加载失败：{e}")))?;

            // 3. Look up `main` and invoke it. Missing `main` becomes a
            //    helpful error.
            let main_fn = c
                .globals()
                .get::<_, Function>("main")
                .map_err(|e| {
                    AppError::msg(format!(
                        "JS 插件未声明 `function main(input)`: {e}"
                    ))
                })?;
            let result: String = main_fn
                .call::<_, String>((payload_json.to_string(),))
                .map_err(|e| AppError::msg(format!("JS 插件 main() 抛错：{e}")))?;
            Ok(result)
        })
    }
}

#[async_trait::async_trait]
impl MetadataSource for JsMetadataSource {
    async fn search(&self, query: &str, page: u32) -> Result<Vec<MetadataTrack>, AppError> {
        let instance = self.instance.clone();
        let payload = serde_json::json!({
            "op": "search",
            "query": query,
            "page": page,
        })
        .to_string();
        let source_id = instance.boot.source_id.clone();
        tokio::task::spawn_blocking(move || {
            let raw = JsMetadataSource::call_in_fresh_context(&instance, &payload)?;
            let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
                AppError::msg(format!("JS 插件 search 返回 JSON 非法：{e}"))
            })?;
            let arr = match v {
                serde_json::Value::Array(_) => v,
                serde_json::Value::Object(mut map) => map
                    .remove("tracks")
                    .unwrap_or(serde_json::Value::Array(Vec::new())),
                _ => serde_json::Value::Array(Vec::new()),
            };
            let mut out = Vec::new();
            if let serde_json::Value::Array(items) = arr {
                for item in items {
                    let track =
                        serde_json_to_track(&item, &source_id);
                    out.push(track);
                }
            }
            Ok::<Vec<MetadataTrack>, AppError>(out)
        })
        .await
        .map_err(|e| AppError::msg(format!("JS 插件搜索任务 join 失败：{e}")))?
    }

    async fn get_track(&self, id: &str) -> Result<MetadataTrack, AppError> {
        let instance = self.instance.clone();
        let payload = serde_json::json!({
            "op": "get_track",
            "id": id,
        })
        .to_string();
        let source_id = instance.boot.source_id.clone();
        tokio::task::spawn_blocking(move || {
            let raw = JsMetadataSource::call_in_fresh_context(&instance, &payload)?;
            let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
                AppError::msg(format!("JS 插件 get_track 返回 JSON 非法：{e}"))
            })?;
            let obj = match v {
                serde_json::Value::Object(map) => map,
                _ => return Err(AppError::msg("JS 插件 get_track 返回非对象")),
            };
            let track_v = obj
                .get("track")
                .cloned()
                .unwrap_or(serde_json::Value::Object(obj));
            Ok(serde_json_to_track(&track_v, &source_id))
        })
        .await
        .map_err(|e| AppError::msg(format!("JS 插件 get_track 任务 join 失败：{e}")))?
    }

    async fn get_lyrics(&self, track: &MetadataTrack) -> Result<Lyrics, AppError> {
        let instance = self.instance.clone();
        let payload = serde_json::json!({
            "op": "get_lyrics",
            "track": track,
        })
        .to_string();
        tokio::task::spawn_blocking(move || {
            let raw = JsMetadataSource::call_in_fresh_context(&instance, &payload)?;
            let v: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|e| AppError::msg(format!("JS 插件 get_lyrics 返回 JSON 非法：{e}")))?;
            // Accepted shapes:
            //   {"lyrics":{"lines":[...]}}
            //   {"lines":[...]}            (legacy, OK)
            //   [...]                       (bare array of line objects)
            let lines_v = match v {
                serde_json::Value::Array(_) => v,
                serde_json::Value::Object(mut map) => map
                    .remove("lyrics")
                    .and_then(|l| {
                        if l.is_object() {
                            l.get("lines").cloned()
                        } else if l.is_array() {
                            Some(l)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(serde_json::Value::Array(Vec::new())),
                _ => serde_json::Value::Array(Vec::new()),
            };
            let lines = parse_lyrics_lines(&lines_v);
            Ok(Lyrics { lines })
        })
        .await
        .map_err(|e| AppError::msg(format!("JS 插件 get_lyrics 任务 join 失败：{e}")))?
    }

    async fn home(&self) -> Result<HomeFeed, AppError> {
        let instance = self.instance.clone();
        let source_id = instance.boot.source_id.clone();
        let payload = serde_json::json!({ "op": "home" }).to_string();
        tokio::task::spawn_blocking(move || {
            let raw = JsMetadataSource::call_in_fresh_context(&instance, &payload)?;
            let v: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|e| AppError::msg(format!("JS 插件 home 返回 JSON 非法：{e}")))?;
            let mut feed = HomeFeed::default();
            if let serde_json::Value::Object(mut map) = v {
                // JS 插件契约：结果可能整体包在 `home` 键下
                // ({"home":{...}}，见 js_runtime 模块文档与示例插件)，
                // 或扁平地直接返回顶层字段。存在 `home` 且为对象时优先取内层。
                if let Some(serde_json::Value::Object(home)) = map.remove("home") {
                    map = home;
                }
                // 主路径：插件声明式返回 `sections` 数组（home-architecture 改造）。
                if let Some(sections) = map.remove("sections") {
                    feed.sections = parse_sections(sections, &source_id);
                } else {
                    // 过渡兼容：旧四字段形状（playlists/new_songs/new_albums/artists）
                    // 合成 sections，避免未迁移的 JS 插件首页瞬间空白。
                    let mut secs: Vec<HomeSection> = Vec::new();
                    if let Some(x) = map.remove("playlists") {
                        let items = parse_feed_items(x, "playlist", &source_id);
                        if !items.is_empty() {
                            secs.push(HomeSection { id: "playlists".into(), title: "歌单".into(), kind: "card".into(), items, hint: None });
                        }
                    }
                    if let Some(x) = map.remove("new_songs") {
                        let items = parse_feed_songs_as_items(x, &source_id);
                        if !items.is_empty() {
                            secs.push(HomeSection { id: "new_songs".into(), title: "新歌".into(), kind: "song".into(), items, hint: None });
                        }
                    }
                    if let Some(x) = map.remove("new_albums") {
                        let items = parse_feed_items(x, "album", &source_id);
                        if !items.is_empty() {
                            secs.push(HomeSection { id: "albums".into(), title: "新碟".into(), kind: "card".into(), items, hint: None });
                        }
                    }
                    if let Some(x) = map.remove("artists") {
                        let items = parse_feed_items(x, "artist", &source_id);
                        if !items.is_empty() {
                            secs.push(HomeSection { id: "artists".into(), title: "艺人".into(), kind: "card".into(), items, hint: None });
                        }
                    }
                    feed.sections = secs;
                }
            }
            Ok(feed)
        })
        .await
        .map_err(|e| AppError::msg(format!("JS 插件 home 任务 join 失败：{e}")))?
    }

    async fn toplist(&self, topid: i32) -> Result<Vec<FeedSong>, AppError> {
        let instance = self.instance.clone();
        let source_id = instance.boot.source_id.clone();
        let payload = serde_json::json!({
            "op": "toplist",
            "topid": topid,
        })
        .to_string();
        tokio::task::spawn_blocking(move || {
            let raw = JsMetadataSource::call_in_fresh_context(&instance, &payload)?;
            let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
                AppError::msg(format!("JS 插件 toplist 返回 JSON 非法：{e}"))
            })?;
            let arr = match v {
                serde_json::Value::Array(_) => v,
                serde_json::Value::Object(mut map) => map
                    .remove("songs")
                    .unwrap_or(serde_json::Value::Array(Vec::new())),
                _ => serde_json::Value::Array(Vec::new()),
            };
            let mut out = Vec::new();
            if let serde_json::Value::Array(items) = arr {
                for item in items {
                    if let serde_json::Value::Object(ref o) = item {
                        if let Some(inner) = o.get("track") {
                            if inner.is_object() {
                                let track =
                                    serde_json_to_track(inner, &source_id);
                                let reason = o
                                    .get("reason")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                out.push(FeedSong {
                                    source_id: track.source_id.clone(),
                                    title: track.title.clone(),
                                    artist: track.artist.clone(),
                                    cover: track.cover.clone(),
                                    duration_ms: track.duration_ms,
                                    source: track.source.clone(),
                                    engine_hint: track.engine_hint.clone(),
                                });
                                let _ = reason; // kept for symmetry; not used by FeedSong
                                continue;
                            }
                        }
                    }
                    // Bare track fallback.
                    let track = serde_json_to_track(&item, &source_id);
                    out.push(FeedSong {
                        source_id: track.source_id.clone(),
                        title: track.title.clone(),
                        artist: track.artist.clone(),
                        cover: track.cover.clone(),
                        duration_ms: track.duration_ms,
                        source: track.source.clone(),
                        engine_hint: track.engine_hint.clone(),
                    });
                }
            }
            Ok(out)
        })
        .await
        .map_err(|e| AppError::msg(format!("JS 插件 toplist 任务 join 失败：{e}")))?
    }
}

/// Build a per-call prelude string. The host bindings are already
/// registered as `__host_*` globals before this is `eval`'d, so the
/// JS-side wrappers see them.
fn build_prelude(
    source_id: &str,
    name: &str,
    version: &str,
    cap_bits: u32,
) -> String {
    format!(
        r#"
        // P5-III/P5-IV host prelude. Plugin may freely overwrite individual fields
        // (e.g. for stricter type coercion) — `bilusic` is intentionally a
        // loose namespace mirroring the JS host capability surface.
        globalThis.bilusic = globalThis.bilusic || {{
            source: '{source_id}',
            name: {name_json},
            version: {version_json},
            capabilities: {cap_bits},
            // N11i: route plugin diagnostics through `__host_log`
            // instead of rquickjs's built-in console.*. The host binding
            // also mirrors into the webview's DevTools Console (via
            // WebviewWindow::eval) so a user inspecting the page can see
            // why a search returned no rows, why a CSRF token expired,
            // etc. Without this, `bilusic.log(...)` calls would only
            // show up on the host's stderr (relevant in `tauri dev`'s
            // terminal but invisible to the webview's user).
            log: function(level, msg) {{
                var m = msg;
                if (typeof m !== 'string') {{
                    try {{ m = JSON.stringify(msg); }} catch (_) {{ m = String(msg); }}
                }}
                try {{ globalThis.__host_log(level, m); }} catch (_) {{ /* never let logging break the plugin */ }}
            }},
            // Sync HTTP via host bindings (each call blocks on the
            // currently-running QuickJS thread; the host Handle is captured
            // at boot time, so reqwest is driven via `Handle::block_on`).
            // Returns {{ status, headers, body }} as a parsed object.
            httpGet: function(url, opts) {{
                return JSON.parse(globalThis.__host_http_get(url, opts || '{{}}'));
            }},
            httpPost: function(url, body, opts) {{
                return JSON.parse(globalThis.__host_http_post(url, typeof body === 'string' ? body : JSON.stringify(body || ''), opts || '{{}}'));
            }},
            // Custom-alphabet base64 decode (for sources like Kuwo whose
            // lyrics use a non-standard alphabet with a char-level offset).
            // `alphabet` is a 64-char string; `offset` is the integer shift
            // applied to each char's alphabet index before re-base64'ing.
            b64decode_alphabet: function(s, alphabet, offset) {{
                return globalThis.__host_b64decode_alphabet(
                    typeof s === 'string' ? s : String(s),
                    typeof alphabet === 'string' ? alphabet : String(alphabet),
                    Number(offset) || 0,
                );
            }},
        }};
        "#,
        source_id = source_id.replace('\'', "\\'"),
        name_json = serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string()),
        version_json = serde_json::to_string(version).unwrap_or_else(|_| "\"0.0.0\"".to_string()),
        cap_bits = cap_bits,
    )
}

/// Adapter factory: parse the manifest, read the entry file, and build a
/// `JsMetadataSource`. Mirrors `wasm_runtime::build_wasm_plugin`.
pub fn build_js_plugin(
    m: &PluginManifest,
    http: Arc<HttpHost>,
) -> Result<(Arc<dyn MetadataSource>, Option<Arc<dyn AudioEngine>>), AppError> {
    let entry = if m.entry.is_empty() {
        "plugin.js".to_string()
    } else {
        m.entry.clone()
    };
    let entry_path = m.folder.join(&entry);
    let module = std::fs::read_to_string(&entry_path).map_err(|e| {
        AppError::msg(format!("读取 JS 入口 {} 失败：{e}", entry_path.display()))
    })?;
    let caps = PluginCapabilities::from_bits(crate::plugin_host::capability_bits(&m.capabilities))
        .unwrap_or(PluginCapabilities::empty());
    let prelude = build_prelude(&m.id, &m.name, &m.version, caps.bits());
    let boot = JsPluginBoot {
        source_id: m.id.clone(),
        name: m.name.clone(),
        version: if m.version.is_empty() {
            "0.0.0".to_string()
        } else {
            m.version.clone()
        },
        // Carry the manifest's `description` field through. Empty string
        // is the signal for `info()` to fall back to the generic label.
        description: m.description.clone(),
        capabilities: caps.bits(),
        prelude,
        http,
    };
    let meta = JsMetadataSource::new(boot, entry_path, module);
    Ok((meta, None))
}

// --- host binding constructors (registered once per call_into_fresh_context) ---

fn make_http_get_fn(
    ctx: rquickjs::Ctx<'_>,
    http: Arc<HttpHost>,
) -> Result<Function<'_>, AppError> {
    Function::new(
        ctx,
        move |url: String, options_json: String| -> String {
            run_http(http.clone(), "GET", &url, None, &options_json)
        },
    )
    .map_err(|e| AppError::msg(format!("构造 __host_http_get：{e}")))
}

fn make_http_post_fn(
    ctx: rquickjs::Ctx<'_>,
    http: Arc<HttpHost>,
) -> Result<Function<'_>, AppError> {
    Function::new(
        ctx,
        move |url: String, body: String, options_json: String| -> String {
            run_http(http.clone(), "POST", &url, Some(body), &options_json)
        },
    )
    .map_err(|e| AppError::msg(format!("构造 __host_http_post：{e}")))
}

fn make_b64decode_alphabet_fn(ctx: rquickjs::Ctx<'_>) -> Result<Function<'_>, AppError> {
    Function::new(
        ctx,
        move |input: String, alphabet: String, offset: i64| -> String {
            match b64decode_alphabet(&input, &alphabet, offset) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[js plugin] b64decode_alphabet 失败：{e}");
                    String::new()
                }
            }
        },
    )
    .map_err(|e| AppError::msg(format!("构造 __host_b64decode_alphabet：{e}")))
}

/// N11i: Construct the host-side sink for `bilusic.log(level, msg)`.
///
/// Always prints to the host stderr (so `tauri dev`'s terminal / release
/// log files still see plugin diagnostics). If an `AppHandle` was
/// captured at boot time, also mirrors the message into the main
/// webview's DevTools Console via `WebviewWindow::eval(...)` so the user
/// can see why a search returned no rows without leaving the UI.
///
/// `level` is whitelisted to `error` / `warn` / `log` — it's interpolated
/// into the JS snippet as a function name, so any untrusted value would
/// be an injection sink otherwise.
fn make_log_fn(
    ctx: rquickjs::Ctx<'_>,
    app_handle: Option<tauri::AppHandle>,
) -> Result<Function<'_>, AppError> {
    Function::new(
        ctx,
        move |level: String, msg: String| -> () {
            // 1. Host stderr — always. Independent of whether webview
            //    mirroring succeeded; useful for `tauri dev` terminal
            //    and bundled release logs (production diagnostics).
            match level.as_str() {
                "error" => eprintln!("[js plugin][error] {}", msg),
                "warn" => eprintln!("[js plugin][warn] {}", msg),
                _ => eprintln!("[js plugin][{}] {}", level, msg),
            }

            // 2. Webview DevTools Console — best-effort. The eval
            //    happens on Tauri's main thread under the hood; we just
            //    fire-and-forget from this spawn_blocking thread.
            if let Some(app) = app_handle.as_ref() {
                if let Some(window) = app.get_webview_window("main") {
                    // Whitelist level so the JS snippet is safe.
                    let js_level = match level.as_str() {
                        "error" => "error",
                        "warn" => "warn",
                        _ => "log",
                    };
                    // JSON-encode both args to defang any quotes in msg.
                    let prefix_json = serde_json::to_string("[js plugin]")
                        .unwrap_or_else(|_| "\"\"".to_string());
                    let msg_json =
                        serde_json::to_string(&msg).unwrap_or_else(|_| "\"\"".to_string());
                    let js = format!(
                        "console.{level}({prefix},{msg})",
                        level = js_level,
                        prefix = prefix_json,
                        msg = msg_json,
                    );
                    if let Err(e) = window.eval(&js) {
                        eprintln!("[js plugin] 推送到 webview console 失败：{e}");
                    }
                }
            }
        },
    )
    .map_err(|e| AppError::msg(format!("构造 __host_log：{e}")))
}

/// Drive the underlying reqwest call synchronously from the spawn_blocking
/// thread, returning a JSON string. `body` is Some for POST/PUT requests.
fn run_http(http: Arc<HttpHost>, method: &str, url: &str, body: Option<String>, options_json: &str) -> String {
    let opts: serde_json::Value =
        serde_json::from_str(options_json).unwrap_or_else(|_| serde_json::json!({}));
    let headers = opts
        .get("headers")
        .and_then(|v| v.as_object())
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let client = http.client.clone();
    let handle = http.handle.clone();
    let method = method.to_string();
    let url = url.to_string();

    // We're on the blocking pool — drive the future via Handle::block_on.
    // Errors are returned to the JS side as `{ ok: false, error }` rather
    // than thrown: keeps the JS path linear and lets the plugin decide
    // whether to retry, return empty, or surface up.
    let result: Result<serde_json::Value, String> = handle.block_on(async move {
        let mut req = match method.as_str() {
            "POST" => client.post(&url),
            "PUT" => client.put(&url),
            "DELETE" => client.delete(&url),
            _ => client.get(&url),
        };
        for (k, v) in &headers {
            // Avoid String→HeaderName/HeaderValue conversion issues by
            // parsing once into typed headers (header_map approach):
            // we use `req.header(key, value)` only if both convert cleanly.
            if let (Ok(name), Ok(value)) = (
                reqwest::header::HeaderName::try_from(k.as_str()),
                reqwest::header::HeaderValue::try_from(v.as_str()),
            ) {
                req = req.header(name, value);
            }
        }
        if let Some(b) = body.clone() {
            req = req.body(b);
        }
        match req.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let headers_out: std::collections::BTreeMap<String, String> = resp
                    .headers()
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect();
                match resp.text().await {
                    Ok(text) => Ok(serde_json::json!({
                        "ok": true,
                        "status": status,
                        "headers": headers_out,
                        "body": text,
                    })),
                    Err(e) => Err(format!("读取响应体失败：{e}")),
                }
            }
            Err(e) => Err(format!("HTTP 请求失败：{e}")),
        }
    });

    match result {
        Ok(v) => v.to_string(),
        Err(e) => serde_json::json!({
            "ok": false,
            "status": 0,
            "headers": {},
            "body": "",
            "error": e,
        })
        .to_string(),
    }
}

/// Decode a string written in a non-standard 64-char base64 alphabet where
/// each char's alphabet-index has been shifted by `offset` (wrapping).
///
/// Use case: Kuwo lyrics — base64 alphabet permuted + every char's index
/// shifted by a constant. The decoding re-maps the bytes back to the
/// standard base64 alphabet (then the host side `base64` crate decodes).
fn b64decode_alphabet(input: &str, alphabet: &str, offset: i64) -> Result<String, String> {
    if alphabet.chars().count() != 64 {
        return Err(format!(
            "alphabet 长度必须为 64，得到 {}",
            alphabet.chars().count()
        ));
    }
    let std_alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let alphabet_bytes: Vec<u8> = alphabet.bytes().collect();
    let len = alphabet_bytes.len() as i64;
    let mut out = String::with_capacity(input.len());
    for &c in input.as_bytes() {
        if c == b'=' {
            out.push('=');
            continue;
        }
        let pos = alphabet_bytes.iter().position(|&x| x == c).ok_or_else(|| {
            format!("字符 {:?} 不在自定义字母表中", c as char)
        })? as i64;
        let shifted = ((pos + offset) % len + len) % len;
        out.push(std_alphabet[shifted as usize] as char);
    }
    // Decode the rewritten standard base64 string in place.
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(out.as_bytes())
        .map_err(|e| format!("base64 解码失败：{e}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Convert a JSON object to a `MetadataTrack`, filling required defaults and
/// copying `source_id` to `source` so the engine_hint pipeline can route it.
fn serde_json_to_track(item: &serde_json::Value, fallback_source: &str) -> MetadataTrack {
    fn s(v: Option<&serde_json::Value>) -> String {
        v.and_then(|x| x.as_str()).unwrap_or("").to_string()
    }
    fn o(v: Option<&serde_json::Value>) -> Option<String> {
        v.and_then(|x| x.as_str().map(|s| s.to_string()))
    }
    fn i(v: Option<&serde_json::Value>) -> i64 {
        v.and_then(|x| x.as_i64())
            .or_else(|| v.and_then(|x| x.as_str().and_then(|s| s.parse().ok())))
            .unwrap_or(0)
    }
    MetadataTrack {
        source: s(item.get("source"))
            .split_whitespace()
            .next()
            .map(|s| s.to_string())
            .unwrap_or_else(|| fallback_source.to_string()),
        source_id: o(item.get("source_id"))
            .or_else(|| o(item.get("id")))
            .unwrap_or_default(),
        title: s(item.get("title")).trim().to_string(),
        artist: s(item.get("artist")).trim().to_string(),
        album: s(item.get("album")).trim().to_string(),
        duration_ms: i(item.get("duration_ms")).max(0),
        cover: s(item.get("cover")),
        lyrics_id: o(item.get("lyrics_id")),
        engine_hint: o(item.get("engine_hint")),
    }
}

/// Convert a JSON array of `{id, title, subtitle?, cover, ...}` browse cards
/// into `SectionItem`s of the given `kind` ("playlist" | "album" | "artist").
fn parse_feed_items(v: serde_json::Value, kind: &str, source_id: &str) -> Vec<SectionItem> {
    let mut out = Vec::new();
    let arr = match v {
        serde_json::Value::Array(_) => v,
        _ => return out,
    };
    if let serde_json::Value::Array(items) = arr {
        for item in items {
            if let serde_json::Value::Object(o) = item {
                let id = o.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let title = o.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
                if id.is_empty() && title.is_empty() {
                    continue;
                }
                let subtitle = o
                    .get("subtitle")
                    .and_then(|x| x.as_str())
                    .map(String::from);
                let cover = o.get("cover").and_then(|x| x.as_str()).unwrap_or("").to_string();
                out.push(SectionItem {
                    id,
                    title,
                    subtitle,
                    cover,
                    kind: kind.to_string(),
                    source: source_id.to_string(),
                    artist: None,
                    duration_ms: None,
                    engine_hint: None,
                });
            }
        }
    }
    out
}

/// Convert a JSON array of `MetadataTrack`-shaped objects into `SectionItem`s
/// with `kind = "song"` (so the frontend renders them as playable cards).
fn parse_feed_songs_as_items(v: serde_json::Value, source_id: &str) -> Vec<SectionItem> {
    let mut out = Vec::new();
    let arr = match v {
        serde_json::Value::Array(_) => v,
        _ => return out,
    };
    if let serde_json::Value::Array(items) = arr {
        for item in items {
            if let serde_json::Value::Object(ref o) = item {
                if let Some(inner) = o.get("track") {
                    if inner.is_object() {
                        let track = serde_json_to_track(inner, source_id);
                        out.push(track_to_section_item(track));
                        continue;
                    }
                }
            }
            let track = serde_json_to_track(&item, source_id);
            out.push(track_to_section_item(track));
        }
    }
    out
}

/// Build a `SectionItem` (kind = "song") from a `MetadataTrack`.
fn track_to_section_item(t: MetadataTrack) -> SectionItem {
    SectionItem {
        id: t.source_id.clone(),
        title: t.title.clone(),
        subtitle: None,
        cover: t.cover.clone(),
        kind: "song".to_string(),
        source: t.source.clone(),
        artist: Some(t.artist.clone()),
        duration_ms: Some(t.duration_ms),
        engine_hint: t.engine_hint.clone(),
    }
}

/// Parse the `sections` array returned by a JS plugin's `home` op into the
/// internal `Vec<HomeSection>`. Each section must carry `id`, `title`, and
/// `items`; `kind` defaults to "card", `hint` is optional. Items are parsed
/// as songs when the section `kind == "song"`, otherwise as browse cards.
fn parse_sections(v: serde_json::Value, source_id: &str) -> Vec<HomeSection> {
    let mut out = Vec::new();
    let arr = match v {
        serde_json::Value::Array(_) => v,
        _ => return out,
    };
    if let serde_json::Value::Array(sections) = arr {
        for sec in sections {
            if let serde_json::Value::Object(o) = sec {
                let id = o.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let title = o.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
                if id.is_empty() && title.is_empty() {
                    continue;
                }
                let kind = o
                    .get("kind")
                    .and_then(|x| x.as_str())
                    .unwrap_or("card")
                    .to_string();
                let items_val = o.get("items").cloned().unwrap_or(serde_json::Value::Array(vec![]));
                let items = if kind == "song" {
                    parse_feed_songs_as_items(items_val, source_id)
                } else {
                    parse_feed_items(items_val, &kind, source_id)
                };
                let hint = o.get("hint").and_then(|x| x.as_str()).map(String::from);
                out.push(HomeSection {
                    id,
                    title,
                    kind,
                    items,
                    hint,
                });
            }
        }
    }
    out
}

/// Convert a JSON array of `MetadataTrack`-shaped objects into `FeedSong`s.
/// Accepts both bare tracks and `[{track: {...}, reason: "..."}]` envelopes.
fn parse_feed_songs(v: serde_json::Value, source_id: &str) -> Vec<FeedSong> {
    let mut out = Vec::new();
    let arr = match v {
        serde_json::Value::Array(_) => v,
        _ => return out,
    };
    if let serde_json::Value::Array(items) = arr {
        for item in items {
            if let serde_json::Value::Object(ref o) = item {
                if let Some(inner) = o.get("track") {
                    if inner.is_object() {
                        let track = serde_json_to_track(inner, source_id);
                        out.push(FeedSong {
                            source_id: track.source_id.clone(),
                            title: track.title.clone(),
                            artist: track.artist.clone(),
                            cover: track.cover.clone(),
                            duration_ms: track.duration_ms,
                            source: track.source.clone(),
                            engine_hint: track.engine_hint.clone(),
                        });
                        continue;
                    }
                }
            }
            let track = serde_json_to_track(&item, source_id);
            out.push(FeedSong {
                source_id: track.source_id.clone(),
                title: track.title.clone(),
                artist: track.artist.clone(),
                cover: track.cover.clone(),
                duration_ms: track.duration_ms,
                source: track.source.clone(),
                engine_hint: track.engine_hint.clone(),
            });
        }
    }
    out
}

/// Convert a JSON array of `{time_ms, text}` (or `{time, text}`) objects
/// into the internal `Lyrics.lines` shape.
fn parse_lyrics_lines(v: &serde_json::Value) -> Vec<LyricLine> {
    let mut out = Vec::new();
    if let serde_json::Value::Array(items) = v {
        for item in items {
            let text = item.get("text").and_then(|x| x.as_str()).unwrap_or("");
            let time_ms = item
                .get("time_ms")
                .and_then(|x| x.as_i64())
                .or_else(|| {
                    // Some sources use seconds instead of milliseconds.
                    item.get("time")
                        .and_then(|x| x.as_f64())
                        .map(|f| (f * 1000.0) as i64)
                })
                .unwrap_or(0);
            if text.is_empty() {
                continue;
            }
            out.push(LyricLine {
                time_ms,
                text: text.to_string(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc as StdArc;

    const MIN_PLUGIN: &str = r#"
        // Bilusic JS plugin contract: `main(input_json_string) → json_string`.
        // The plugin is responsible for JSON.stringify on both ends; this
        // keeps the Rust ↔ JS contract a single string type and avoids
        // rquickjs 0.12's value-conversion surface area.
        function main(inputJson) {
            const input = JSON.parse(inputJson);
            const op = input.op;
            if (op === 'search' || (op === undefined && input.query !== undefined)) {
                return JSON.stringify({
                    tracks: [
                        {
                            source_id: 'js-test-1',
                            title: 'JS Test: ' + input.query,
                            artist: 'JS Plugin',
                            album: '',
                            duration_ms: 1000,
                            cover: '',
                            lyrics_id: null
                        }
                    ]
                });
            }
            if (op === 'get_track' || (op === undefined && input.id !== undefined)) {
                return JSON.stringify({
                    track: {
                        source_id: input.id,
                        title: 'JS Test Track',
                        artist: 'JS Plugin',
                        album: '',
                        duration_ms: 2000,
                        cover: '',
                        lyrics_id: null
                    }
                });
            }
            return '{}';
        }
    "#;

    fn make_http() -> StdArc<HttpHost> {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let client = reqwest::Client::new();
        StdArc::new(HttpHost {
            client,
            handle: rt.handle().clone(),
            // N11i: tests don't run inside a Tauri app, so there's no
            // AppHandle to mirror plugin logs into a webview. Falls back
            // to host-stderr only — fine for unit tests.
            app_handle: None,
        })
    }

    fn make_plugin(http: StdArc<HttpHost>) -> Arc<JsMetadataSource> {
        let boot = JsPluginBoot {
            source_id: "js-test".into(),
            name: "JS Test".into(),
            version: "0.1.0".into(),
            description: "".into(), // exercises the fallback path in tests
            capabilities: PluginCapabilities::METADATA_SEARCH.bits(),
            prelude: build_prelude(
                "js-test",
                "JS Test",
                "0.1.0",
                PluginCapabilities::METADATA_SEARCH.bits(),
            ),
            http,
        };
        JsMetadataSource::new(boot, PathBuf::from("(in-memory)"), MIN_PLUGIN.to_string())
    }

    #[test]
    fn js_plugin_search_returns_track() {
        let plugin = make_plugin(make_http());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let res = rt.block_on(async { plugin.search("hello", 1).await });
        let tracks = res.expect("search ok");
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].source_id, "js-test-1");
        assert!(tracks[0].title.contains("hello"));
        assert_eq!(tracks[0].source, "js-test");
    }

    #[test]
    fn js_plugin_get_track_returns_metadata() {
        let plugin = make_plugin(make_http());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let res = rt.block_on(async { plugin.get_track("id-42").await });
        let t = res.expect("get_track ok");
        assert_eq!(t.source_id, "id-42");
        assert_eq!(t.title, "JS Test Track");
    }

    #[test]
    fn js_plugin_without_main_fails_clean() {
        let http = make_http();
        let boot = JsPluginBoot {
            source_id: "js-broken".into(),
            name: "Broken".into(),
            version: "0.0.0".into(),
            description: "".into(),
            capabilities: PluginCapabilities::METADATA_SEARCH.bits(),
            prelude: build_prelude(
                "js-broken",
                "Broken",
                "0.0.0",
                PluginCapabilities::METADATA_SEARCH.bits(),
            ),
            http,
        };
        // Booting does NOT eagerly check for main() — it's only checked at
        // call time, when we can report a helpful message. That round-trip
        // still falls into AppError.
        let plugin = JsMetadataSource::new(
            boot,
            PathBuf::from("(in-memory)"),
            "// no main".to_string(),
        );
        let rt = tokio::runtime::Runtime::new().unwrap();
        let res = rt.block_on(async { plugin.search("x", 1).await });
        assert!(res.is_err());
        let msg = res.unwrap_err().to_string();
        assert!(msg.contains("main"), "error message: {msg}");
    }

    /// Regression for the bug the user hit on 2026-08-21: when a JS plugin's
    /// `plugin.json` carries a `description`, the Settings page must show
    /// the manifest value — not the hard-coded "JS plugin: <name>" fallback.
    /// wasm_runtime.rs already does `m.description.clone()`; parity here.
    #[test]
    fn info_uses_manifest_description_when_present() {
        let plugin = JsMetadataSource::new(
            JsPluginBoot {
                source_id: "kuwo".into(),
                name: "酷我音乐".into(),
                version: "0.2.0".into(),
                description: "酷我音乐元数据 · 搜索 / 单曲 / 歌词 / 推荐歌单 / 热门歌手".into(),
                capabilities: PluginCapabilities::METADATA_SEARCH.bits(),
                prelude: build_prelude(
                    "kuwo",
                    "酷我音乐",
                    "0.2.0",
                    PluginCapabilities::METADATA_SEARCH.bits(),
                ),
                http: make_http(),
            },
            PathBuf::from("(in-memory)"),
            MIN_PLUGIN.to_string(),
        );
        let info = plugin.info();
        assert_eq!(
            info.description,
            "酷我音乐元数据 · 搜索 / 单曲 / 歌词 / 推荐歌单 / 热门歌手"
        );
    }

    /// Blank `description` in plugin.json still falls back to the generic
    /// JS-plugin label, so user-facing copy never renders as an empty
    /// second line.
    #[test]
    fn info_falls_back_when_manifest_description_blank() {
        let plugin = JsMetadataSource::new(
            JsPluginBoot {
                source_id: "x".into(),
                name: "X".into(),
                version: "0.0.0".into(),
                description: "   ".into(), // whitespace-only also blank
                capabilities: PluginCapabilities::empty().bits(),
                prelude: build_prelude(
                    "x",
                    "X",
                    "0.0.0",
                    PluginCapabilities::empty().bits(),
                ),
                http: make_http(),
            },
            PathBuf::from("(in-memory)"),
            MIN_PLUGIN.to_string(),
        );
        let info = plugin.info();
        assert_eq!(info.description, "JS plugin: X");
    }

    /// Kuwo-style decode: alphabet `ABC...XYZabc...xyz01...9/:`, offset 0 is
    /// identity. With offset=0 the input keeps the standard alphabet (since
    /// the std alphabet is the source for alphabet indices); offset=N just
    /// reverses by rotating. This test pins behaviour: a no-op round-trip
    /// for offset=0 on plain base64.
    #[test]
    fn b64decode_alphabet_roundtrip_identity_offset() {        let std = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        // Encode "Hello!" in standard base64 → "SGVsbG8h"
        let src = "SGVsbG8h";
        let decoded = b64decode_alphabet(src, std, 0).unwrap();
        assert_eq!(decoded, "Hello!");
    }

    /// Kuwo's actual scheme: custom alphabet + offset=0 maps every char by
    /// position in the custom alphabet, then maps to the corresponding
    /// position in the std alphabet — which inverts Kuwo's encoding.
    #[test]
    fn b64decode_alphabet_inverts_custom_alphabet() {
        // Use a deliberately shifted alphabet (1-char rotation) and a
        // matching offset so an identity decode recovers the original.
        let mut alpha = String::from("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/");
        // rotate by 1: first char → last
        let bytes = unsafe { alpha.as_bytes_mut() };
        let first = bytes[0];
        for i in 0..bytes.len() - 1 {
            bytes[i] = bytes[i + 1];
        }
        *bytes.last_mut().unwrap() = first;
        // Encoding: take std-base64 of "Hello!" = "SGVsbG8h".
        // Apply the *inverse* of how the (rotated) alphabet would have
        // been used to scramble: take the original src and swap into the
        // rotated alphabet. Then `b64decode_alphabet(src_rot, rotated, 0)`
        // should recover "Hello!".
        let std_src = "SGVsbG8h";
        let rotated_alpha: Vec<u8> = alpha.bytes().collect();
        let std_alpha = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut scrambled = String::new();
        for &c in std_src.as_bytes() {
            if c == b'=' {
                scrambled.push('=');
                continue;
            }
            let pos = std_alpha.iter().position(|&x| x == c).unwrap();
            scrambled.push(rotated_alpha[pos] as char);
        }
        // Decoding scrambled through (rotated, offset 0) → should invert
        // because each char's position in the rotated alphabet gives the
        // std-alphabet index directly.
        let decoded = b64decode_alphabet(&scrambled, &alpha, 0).unwrap();
        assert_eq!(decoded, "Hello!");
    }
}
