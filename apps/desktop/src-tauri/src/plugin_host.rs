//! Plugin host: discovers third-party `plugin.json` manifests from the
//! `plugins/` directory, merges them with the built-in Rust plugins, and
//! tracks per-plugin enable / order / uninstall state (persisted to
//! `app_data_dir/plugin_state.json`).
//!
//! **Design (P5-II, 2026-08-18): native-first, reserved runtime slot.**
//! All first-party sources (QQ / Bilibili / NetEase / Kugou / Kuwo / Spotify /
//! YTMusic / LrcLib) are compiled-in Rust trait implementations registered
//! directly into the registries — zero runtime weight, no V8. A manifest with
//! *no* compiled adapter is reported as `stub: true`. The single insertion
//! point for a future third-party runtime (JS via deno_core, or WASM via
//! extism/wasmtime) is [`PluginHost::load_runtime_plugin`]: it takes a manifest
//! and its entry and produces `Arc<dyn MetadataSource>` / `Arc<dyn
//! AudioEngine>` adapters, symmetric with the built-ins. Until that lands, the
//! seam returns `Err` and manifests stay `stub`.

use crate::engine::{AudioEngine, EngineRegistry};
use crate::error::AppError;
use crate::metadata::{MetadataRegistry, MetadataSource};
use crate::declarative::{DeclarativeAudioEngine, DeclarativeConfig, DeclarativeMetadataSource};
use crate::plugin::{PluginCapabilities, PluginInfo};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// Bundled plugins that ship with the release and may not be uninstalled by
/// the user. `init()`'s discovery keeps user-space copies of these ids as
/// candidates but `uninstall()` always refuses, so even if a user-imported
/// manifest wins the id collision it stays read-only.
const BUNDLED_NON_REMOVABLE_IDS: &[&str] = &["itunes-declarative"];

/// The role a plugin plays in Bilusic's decoupled architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    Metadata,
    Audio,
    Lyrics,
}

impl PluginKind {
    fn as_str(&self) -> &'static str {
        match self {
            PluginKind::Metadata => "metadata",
            PluginKind::Audio => "engine",
            PluginKind::Lyrics => "lyrics",
        }
    }
}

/// A discovered third-party plugin manifest (`plugins/<id>/plugin.json`).
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    pub r#type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_entry")]
    pub entry: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub requires_auth: bool,
    /// Runtime kind: "" (unknown / native-only), "declarative" (config-driven,
    /// zero runtime), "wasm" (Phase 2). Drives `load_runtime_plugin`.
    #[serde(default)]
    pub runtime: String,
    /// Declarative HTTP config block (present when `runtime == "declarative"`).
    #[serde(default)]
    pub declarative: Option<DeclarativeConfig>,
    /// Absolute path to the plugin folder (filled in at discovery time; not
    /// serialized to disk).
    #[serde(skip)]
    pub folder: PathBuf,
}

fn default_version() -> String {
    "0.0.0".into()
}
fn default_entry() -> String {
    "index.js".into()
}

/// A single row in the plugin management list (frontend-facing).
#[derive(Debug, Clone, Serialize)]
pub struct PluginListItem {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub capabilities: u32,
    pub enabled: bool,
    pub order: i32,
    /// true = discovered manifest with no compiled adapter yet (not usable as
    /// an active source/engine until JS execution lands).
    pub stub: bool,
    /// true = shipped built-in (Rust trait); cannot be uninstalled.
    pub builtin: bool,
    /// true = lives under the user plugin dir (`app_data_dir/plugins`) and may
    /// be uninstalled.
    pub removable: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PluginStateStore {
    enabled: HashMap<String, bool>,
    order: HashMap<String, i32>,
}

impl PluginStateStore {
    fn key(kind: &str, id: &str) -> String {
        format!("{kind}:{id}")
    }
    fn is_enabled(&self, kind: &str, id: &str, default: bool) -> bool {
        self.enabled
            .get(&Self::key(kind, id))
            .copied()
            .unwrap_or(default)
    }
    fn order_of(&self, kind: &str, id: &str) -> Option<i32> {
        self.order.get(&Self::key(kind, id)).copied()
    }
}

/// Convert capability name strings (as written in `plugin.json`) to a bitmask.
pub fn capability_bits(names: &[String]) -> u32 {
    let mut bits = 0u32;
    for n in names {
        bits |= match n.as_str() {
            "METADATA_SEARCH" => PluginCapabilities::METADATA_SEARCH.bits(),
            "METADATA_GET" => PluginCapabilities::METADATA_GET.bits(),
            "METADATA_LYRICS" => PluginCapabilities::METADATA_LYRICS.bits(),
            "METADATA_HOME" => PluginCapabilities::METADATA_HOME.bits(),
            "AUDIO_BY_ID" => PluginCapabilities::AUDIO_BY_ID.bits(),
            "AUDIO_BY_QUERY" => PluginCapabilities::AUDIO_BY_QUERY.bits(),
            "AUDIO_NEEDS_AUTH" => PluginCapabilities::AUDIO_NEEDS_AUTH.bits(),
            _ => 0,
        };
    }
    bits
}

/// Owns the built-in registries plus discovered third-party manifests and the
/// persisted enable/order state. Shared (via `Arc`) by the [`Coordinator`] and
/// the plugin-management Tauri commands.
pub struct PluginHost {
    pub metadata: Arc<MetadataRegistry>,
    pub engines: Arc<EngineRegistry>,
    manifests: RwLock<Vec<PluginManifest>>,
    state: RwLock<PluginStateStore>,
    state_path: RwLock<Option<PathBuf>>,
    user_plugins_dir: RwLock<Option<PathBuf>>,
    /// Rust 编译期注册的 plugin id 集合（lib.rs 里 `metadata.register(...)`
    /// / `engines.register(...)` 注册的）。这些 id 的 plugin 永远 `builtin=true`，
    /// 哪怕 bundled manifest 有同 id 的 stub 也不能翻它。在 init() 末尾填一次。
    builtin_ids: RwLock<HashSet<String>>,
    /// 会话内被「卸载」（隐藏）的 bundled / dev stub 的 `kind:id` 集合。
    /// 打包插件的文件夹属于 app 安装包，无法真正删除，所以 uninstall() 对
    /// 这类 stub 只做「当次会话隐藏」——init() 下次启动重新扫描磁盘时它会
    /// 再次出现。user 插件目录下的是真删除，不进这个集合。
    hidden: RwLock<HashSet<String>>,
    /// Shared HTTP dependencies for JS plugins (`bilusic.httpGet`/`httpPost`).
    /// Set during `init()` (called from Tauri `.setup()` — inside the tokio
    /// runtime context) via `Handle::try_current`. None means JS plugins will
    /// fail to load (host HTTP binding unavailable). Cheap to clone — `Arc`.
    #[cfg(feature = "js")]
    js_http: RwLock<Option<Arc<crate::js_runtime::HttpHost>>>,
}

impl PluginHost {
    pub fn new(metadata: Arc<MetadataRegistry>, engines: Arc<EngineRegistry>) -> Arc<Self> {
        Arc::new(Self {
            metadata,
            engines,
            manifests: RwLock::new(vec![]),
            state: RwLock::new(PluginStateStore::default()),
            state_path: RwLock::new(None),
            user_plugins_dir: RwLock::new(None),
            builtin_ids: RwLock::new(HashSet::new()),
            hidden: RwLock::new(HashSet::new()),
            #[cfg(feature = "js")]
            js_http: RwLock::new(None),
        })
    }

    /// Resolve plugin directories and load persisted state. Must be called
    /// from the Tauri `setup` hook (needs `AppHandle` for path resolution).
    ///
    /// `app_handle` is captured so that JS plugins' `bilusic.log(...)`
    /// can be mirrored into the main webview's DevTools Console. Without
    /// it, plugin diagnostics only reach the host stderr.
    pub fn init(
        &self,
        app_data_dir: &Path,
        resource_dir: Option<&Path>,
        app_handle: &tauri::AppHandle,
    ) {
        // N11c fix: HttpHost::with_default_client_*() internally calls
        // `tauri::async_runtime::handle()`, which is Tauri 2's thread-
        // agnostic, lazy-initializing global handle accessor (see Tauri
        // source: `RUNTIME.get_or_init(default_runtime)`). It returns a
        // tokio::runtime::Handle **without** requiring a Handle::current()
        // thread-local — so calling it from the sync `.setup` closure (which
        // runs on the tao main thread, NOT in any tokio runtime context)
        // actually succeeds. The previous design called Handle::try_current
        // here, which failed for that reason (N11b symptom: "there is no
        // reactor running").
        #[cfg(feature = "js")]
        {
            *self.js_http.write().unwrap() = Some(Arc::new(
                crate::js_runtime::HttpHost::with_default_client_and_app(app_handle.clone()),
            ));
        }

        let user_dir = app_data_dir.join("plugins");
        let _ = std::fs::create_dir_all(&user_dir);
        *self.user_plugins_dir.write().unwrap() = Some(user_dir.clone());

        let state_path = app_data_dir.join("plugin_state.json");
        if let Ok(text) = std::fs::read_to_string(&state_path) {
            if let Ok(s) = serde_json::from_str::<PluginStateStore>(&text) {
                *self.state.write().unwrap() = s;
            }
        }
        *self.state_path.write().unwrap() = Some(state_path);

        // First-party sources that ship with unimplemented endpoints (阶段 2)
        // start disabled, so they never become the default active search source
        // before their APIs are wired. Drop an id from this list once its
        // endpoints land. (netease graduated 2026-08-21 — weapi wired.)
        //
        // N14 (2026-08-27): `kugou` / `spotify` / `ytmusic` skeleton 已被彻底
        // 删除（lib.rs 不再注册它们），故此数组清空；保留注释是为了记录演进
        // 痕迹。如果将来需要再次引入「默认关闭的实验源」，在这里追加即可。
        const EXPERIMENTAL: &[&str] = &[];
        {
            let mut st = self.state.write().unwrap();
            for id in EXPERIMENTAL {
                st.enabled.entry(format!("metadata:{id}")).or_insert(false);
            }
        }

        // Candidate discovery roots: user dir (writable), bundled resources,
        // and the dev cwd (so `apps/desktop/plugins` is picked up in `tauri dev`).
        let mut candidates: Vec<PathBuf> = vec![user_dir.clone()];
        if let Some(r) = resource_dir {
            candidates.push(r.join("plugins"));
        }
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join("plugins"));
        }
        // Dev-mode safety net: when running `tauri dev` the working directory
        // is not guaranteed to be the workspace root, so cwd.join("plugins")
        // may miss bundled templates. Anchor on `CARGO_MANIFEST_DIR`
        // (compile-time absolute path of `src-tauri/`) and probe the
        // workspace's `apps/desktop/plugins/` next to it — the canonical
        // home of bundled templates.
        {
            let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
            // src-tauri is at apps/desktop/src-tauri → ../plugins = apps/desktop/plugins.
            let workspace_bundle = manifest_dir.join("../plugins");
            if let Ok(canon) = workspace_bundle.canonicalize() {
                if canon.is_dir() {
                    candidates.push(canon);
                }
            }
        }
        // One-shot discovery log so a cold-start config issue (bundled plugins
        // not landing in resource_dir) is immediately visible in `tauri dev` /
        // `tauri build` output without bisecting through release binaries.
        eprintln!("[bilusic] plugin discovery: candidates={:?}", candidates);
        for c in &candidates {
            let exists = c.is_dir();
            let n = if exists {
                std::fs::read_dir(c).map(|d| d.flatten().count()).unwrap_or(0)
            } else {
                0
            };
            eprintln!("[bilusic]   {} → exists={} entries={}", c.display(), exists, n);
        }

        let mut by_key: HashMap<String, PluginManifest> = HashMap::new();
        for dir in candidates {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if !p.is_dir() {
                    continue;
                }
                let manifest_path = p.join("plugin.json");
                if !manifest_path.exists() {
                    continue;
                }
                match std::fs::read_to_string(&manifest_path)
                    .ok()
                    .and_then(|t| serde_json::from_str::<PluginManifest>(&t).ok())
                {
                    Some(mut m) => {
                        m.folder = p.clone();
                        // User-space plugin wins on id collision (lets a user
                        // override a bundled example of the same id).
                        by_key.insert(format!("{}:{}", m.r#type, m.id), m);
                    }
                    None => eprintln!("[bilusic] 跳过无法解析的插件清单：{}", manifest_path.display()),
                }
            }
        }
        *self.manifests.write().unwrap() = by_key.into_values().collect();

        // ── dev-mode stale-folder warning ───────────────────────────────
        // When running `tauri dev`, workspace plugins (apps/desktop/plugins/*)
        // can drift from the user-space copy under app_data_dir/plugins/* if
        // the user-space version was installed from an older release (e.g. an
        // early declarative skeleton). The two have the same id; whichever
        // was inserted last into the manifest map wins — but a stale
        // user-space *folder* can mask later workspace changes silently.
        //
        // Detect the drift between file mtimes and emit a one-shot warning.
        // We never delete on the user's behalf (some setups intentionally
        // override), just surface the divergence with a one-line fix.
        #[cfg(debug_assertions)]
        {
            let workspace_root =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins");
            let workspace_root = workspace_root.canonicalize().ok();
            let user_root = self.user_plugins_dir.read().unwrap().clone();

            for m in self.manifests.read().unwrap().iter() {
                let mt_w = workspace_root
                    .as_ref()
                    .map(|r| r.join(&m.id).join("plugin.json"))
                    .and_then(|p| std::fs::metadata(p).ok())
                    .and_then(|s| s.modified().ok());
                let mt_u = user_root
                    .as_ref()
                    .map(|r| r.join(&m.id).join("plugin.json"))
                    .and_then(|p| std::fs::metadata(p).ok())
                    .and_then(|s| s.modified().ok());

                if let (Some(w), Some(u)) = (mt_w, mt_u) {
                    if u != w {
                        eprintln!(
                            "[bilusic] plugin \"{}\" has a stale user-space copy \
                             (user mtime != workspace mtime). Workspace version \
                             will take effect, but the leftover folder at\n         {} \
                             \n still shadows newer edits. Remove it once to keep \
                             dev iterations clean.",
                            m.id,
                            user_root
                                .as_ref()
                                .map(|r| r.join(&m.id).display().to_string())
                                .unwrap_or_default(),
                        );
                    }
                }
            }
        }

        // Snapshot the Rust-compiled plugin ids (lib.rs registered these
        // before init ran). `all_plugins` uses this set — not manifest_ids —
        // to decide `builtin`: a manifest with the same id as a Rust source
        // can't flip that source to `third-party`.
        let mut ids = HashSet::new();
        for info in self.metadata.list_all() {
            ids.insert(info.id);
        }
        for info in self.engines.list() {
            ids.insert(info.id);
        }
        *self.builtin_ids.write().unwrap() = ids;
    }

    fn kind_of_info(info: &PluginInfo) -> PluginKind {
        let caps = PluginCapabilities::from_bits(info.capabilities)
            .unwrap_or(PluginCapabilities::empty());
        // A plugin that provides lyrics and nothing else is its own category
        // (e.g. lrclib). This keeps the Settings → Plugins "歌词" tab meaningful.
        let lyrics_only = (caps & PluginCapabilities::METADATA_LYRICS)
            != PluginCapabilities::empty()
            && (caps
                & (PluginCapabilities::METADATA_SEARCH
                    | PluginCapabilities::METADATA_GET
                    | PluginCapabilities::METADATA_HOME))
                == PluginCapabilities::empty()
            && (caps
                & (PluginCapabilities::AUDIO_BY_ID
                    | PluginCapabilities::AUDIO_BY_QUERY
                    | PluginCapabilities::AUDIO_NEEDS_AUTH))
                == PluginCapabilities::empty();
        if lyrics_only {
            PluginKind::Lyrics
        } else if (caps
            & (PluginCapabilities::METADATA_SEARCH
                | PluginCapabilities::METADATA_GET
                | PluginCapabilities::METADATA_LYRICS
                | PluginCapabilities::METADATA_HOME))
            != PluginCapabilities::empty()
        {
            PluginKind::Metadata
        } else if (caps
            & (PluginCapabilities::AUDIO_BY_ID
                | PluginCapabilities::AUDIO_BY_QUERY
                | PluginCapabilities::AUDIO_NEEDS_AUTH))
            != PluginCapabilities::empty()
        {
            PluginKind::Audio
        } else {
            PluginKind::Lyrics
        }
    }

    /// All plugins known to the host: built-ins (always present) + discovered
    /// third-party manifests. Sorted by `order` then id. Powers the Settings →
    /// Plugins management page.
    pub fn all_plugins(&self) -> Vec<PluginListItem> {
        let state = self.state.read().unwrap();

        // The **source of truth for `builtin`** is the Rust-compiled id set
        // captured in `builtin_ids` at init() end. Earlier revisions anchored
        // on `manifest_ids` (the inverse: "not in manifests ⇒ built-in"),
        // but a bundled manifest shipping with the same id as a Rust source
        // (e.g. `qq_music/` plugin.json + `plugins/qq_music_metadata.rs`)
        // flipped the built-in to `third-party`. `builtin_ids` wins for the
        // **built-in row** (pass 2) — but a same-id bundled manifest is ALSO
        // emitted as a separate third-party **stub** row in pass 3, so both
        // the compiled source and its bundled template are visible in the UI
        // (the stub gets the third-party badge because `builtin=false`).
        let builtin_ids: HashSet<String> = self.builtin_ids.read().unwrap().clone();

        let mut items: Vec<PluginListItem> = Vec::new();

        // Pass 1: actually load+register adapters for manifests so that
        // search/play can use them. We do NOT classify them here — pass 2
        // decides builtin vs third-party from `builtin_ids`.
        for m in self.manifests.read().unwrap().iter() {
            if self.metadata.get(&m.id).is_some() || self.engines.get(&m.id).is_some() {
                continue;
            }
            if let Ok((meta, engine)) = self.load_runtime_plugin(m) {
                self.metadata.register(meta);
                if let Some(e) = engine {
                    self.engines.register(e);
                }
            }
        }

        // Pass 2: enumerate every registry entry, classify each by whether
        // its id is in the Rust-compiled `builtin_ids` set. Stub rows (pass 3)
        // are unconditionally third-party.
        for info in self.metadata.list_all() {
            let k = Self::kind_of_info(&info).as_str();
            let builtin = builtin_ids.contains(&info.id);
            // A bundled real-release plugin (e.g. `itunes-declarative`) is a
            // manifest-derived source but cannot be uninstalled by the user —
            // treat it the same as a Rust built-in for `removable`. The
            // back-end `uninstall()` enforces this too as a safety net in case
            // a user-space manifest with the same id won the collision.
            let removable = !builtin && !BUNDLED_NON_REMOVABLE_IDS.contains(&info.id.as_str());
            items.push(PluginListItem {
                id: info.id.clone(),
                kind: k.to_string(),
                name: info.name.clone(),
                version: info.version.clone(),
                description: info.description.clone(),
                capabilities: info.capabilities,
                enabled: state.is_enabled(k, &info.id, true),
                order: state.order_of(k, &info.id).unwrap_or(0),
                stub: false,
                builtin,
                removable,
            });
        }
        for info in self.engines.list() {
            let k = Self::kind_of_info(&info).as_str();
            let builtin = builtin_ids.contains(&info.id);
            let removable = !builtin && !BUNDLED_NON_REMOVABLE_IDS.contains(&info.id.as_str());
            items.push(PluginListItem {
                id: info.id.clone(),
                kind: k.to_string(),
                name: info.name.clone(),
                version: info.version.clone(),
                description: info.description.clone(),
                capabilities: info.capabilities,
                enabled: state.is_enabled(k, &info.id, true),
                order: state.order_of(k, &info.id).unwrap_or(0),
                stub: false,
                builtin,
                removable,
            });
        }

        // Pass 3: emit stub rows for discovered manifests that did NOT load
        // into a real registered adapter. Two cases:
        //   (a) genuine third-party stubs whose runtime is unknown /
        //       unsupported (load_runtime_plugin failed in pass 1);
        //   (b) "bundled stubs" — manifests that ship with the app and
        //       intentionally share an id with a Rust-compiled built-in (e.g.
        //       a bundled `qq_music/plugin.json` beside the compiled
        //       `plugins/qq_music_metadata.rs`). The built-in id short-circuits
        //       pass 1 (its adapter is already registered), so the same-id
        //       manifest is never loaded as a real source — it is emitted here
        //       as a separate third-party stub row (builtin=false ⇒ third-party
        //       badge in the UI) so the bundled template is visible alongside
        //       the built-in.
        // Every stub (bundled or user-dir) shows the uninstall button
        // (`removable = true`). Clicking it on a user-dir stub really deletes
        // the folder; clicking it on a bundled/dev stub hides it for the
        // current session (the folder lives inside the app package and cannot
        // be deleted), and `uninstall()` adds it to `hidden` so it disappears
        // until the next launch re-scans the disk. We therefore skip any stub
        // currently in `hidden` here.
        let hidden = self.hidden.read().unwrap().clone();
        for m in self.manifests.read().unwrap().iter() {
            let registered =
                self.metadata.get(&m.id).is_some() || self.engines.get(&m.id).is_some();
            // Skip only manifests that loaded into a real adapter AND do not
            // mirror a built-in id. A same-id manifest is the bundled stub and
            // must still be emitted below.
            if registered && !builtin_ids.contains(&m.id) {
                continue;
            }
            let k = m.r#type.as_str(); // "metadata" | "engine" | "lyrics"
            let key = format!("{k}:{}", m.id);
            if hidden.contains(&key) {
                continue;
            }
            items.push(PluginListItem {
                id: m.id.clone(),
                kind: k.to_string(),
                name: m.name.clone(),
                version: m.version.clone(),
                description: m.description.clone(),
                capabilities: capability_bits(&m.capabilities),
                enabled: state.is_enabled(k, &m.id, false),
                order: state.order_of(k, &m.id).unwrap_or(100),
                stub: true,
                builtin: false,
                removable: true,
            });
        }

        items.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
        items
    }

    /// Built-in metadata sources that are currently enabled (excludes stubs).
    pub fn enabled_metadata_sources(&self) -> Vec<PluginInfo> {
        let state = self.state.read().unwrap();
        self.metadata
            .list()
            .into_iter()
            .filter(|info| state.is_enabled("metadata", &info.id, true))
            .collect()
    }

    /// Built-in audio engines that are currently enabled (excludes stubs).
    pub fn enabled_audio_engines(&self) -> Vec<PluginInfo> {
        let state = self.state.read().unwrap();
        self.engines
            .list()
            .into_iter()
            .filter(|info| state.is_enabled("engine", &info.id, true))
            .collect()
    }

    pub fn has_enabled_metadata(&self, id: &str) -> bool {
        self.enabled_metadata_sources().iter().any(|p| p.id == id)
    }
    pub fn has_enabled_engine(&self, id: &str) -> bool {
        self.enabled_audio_engines().iter().any(|p| p.id == id)
    }

    /// **Third-party plugin runtime seam.**
    ///
    /// First-party sources are compiled-in Rust and registered directly, so
    /// this is never called for them. A discovered manifest's `runtime` field
    /// selects the adapter factory:
    ///   * `"declarative"` — config-driven, zero runtime: the `declarative`
    ///     HTTP block is turned into `DeclarativeMetadataSource` /
    ///     `DeclarativeAudioEngine` adapters symmetric with the built-ins.
    ///   * `"wasm"` — Phase 2: load the `entry` `.wasm` and forward trait
    ///     calls to its exports via host functions (not yet wired).
    ///
    /// A successful build returns the adapters; the caller (`init`) registers
    /// them into the (now internally-mutable) registries. Until a runtime is
    /// implemented, manifests of that kind stay `stub: true`.
    pub fn load_runtime_plugin(
        &self,
        m: &PluginManifest,
    ) -> Result<(Arc<dyn MetadataSource>, Option<Arc<dyn AudioEngine>>), AppError> {
        match m.runtime.as_str() {
            "declarative" => {
                let cfg = m
                    .declarative
                    .clone()
                    .ok_or_else(|| AppError::msg("declarative 插件缺少 http 配置块"))?;
                let caps = PluginCapabilities::from_bits(capability_bits(&m.capabilities))
                    .unwrap_or(PluginCapabilities::empty());
                let meta = Arc::new(DeclarativeMetadataSource::new(
                    &m.id,
                    &m.name,
                    &m.version,
                    &m.description,
                    caps,
                    cfg.clone(),
                ));
                let engine = if cfg.stream_by_id.is_some() {
                    Some(
                        Arc::new(DeclarativeAudioEngine::new(
                            &m.id,
                            &m.name,
                            &m.version,
                            &m.description,
                            caps,
                            cfg,
                        )) as Arc<dyn AudioEngine>,
                    )
                } else {
                    None
                };
                Ok((meta, engine))
            }
            // Phase 2: WASM escape hatch. When built with `--features wasm`,
            // hand the manifest to the wasmtime host; otherwise tell the user
            // the runtime needs to be compiled in.
            runtime => {
                #[cfg(feature = "wasm")]
                if runtime == "wasm" {
                    return crate::wasm_runtime::build_wasm_plugin(m);
                }
                #[cfg(feature = "js")]
                if runtime == "js" {
                    let http = self
                        .js_http
                        .read()
                        .unwrap()
                        .clone()
                        .ok_or_else(|| {
                            AppError::msg("JS HTTP host 未初始化（init() 未在 tokio 上下文中调用）")
                        })?;
                    return crate::js_runtime::build_js_plugin(m, http);
                }
                Err(AppError::msg(if runtime == "wasm" {
                    "WASM 运行时需以 --features wasm 重新编译启用"
                } else if runtime == "js" {
                    "JS 运行时需以 --features js 重新编译启用"
                } else {
                    "未知/未实现的插件运行时类型"
                }))
            }
        }
    }

    fn save(&self) {
        if let Some(path) = self.state_path.read().unwrap().clone() {
            if let Ok(text) = serde_json::to_string_pretty(&*self.state.read().unwrap()) {
                let _ = std::fs::write(path, text);
            }
        }
    }

    pub fn set_enabled(&self, kind: &str, id: &str, enabled: bool) {
        self.state
            .write()
            .unwrap()
            .enabled
            .insert(format!("{kind}:{id}"), enabled);
        self.save();
    }

    /// Reorder: the frontend sends the full visible list in new order; we
    /// assign sequential order values so the list stays stable across reloads.
    pub fn set_order(&self, ordered: &[(String, String)]) {
        {
            let mut state = self.state.write().unwrap();
            for (i, (kind, id)) in ordered.iter().enumerate() {
                state.order.insert(format!("{kind}:{id}"), i as i32);
            }
        }
        self.save();
    }

    /// Remove a third-party plugin. Two cases:
    ///   - folder lives under the user plugin dir → really delete the folder;
    ///   - bundled / dev stub (folder inside the app package) → cannot be
    ///     deleted, so just hide it for the current session by adding
    ///     `kind:id` to `hidden`; init() re-scans the disk next launch and it
    ///     reappears. The front-end shows the uninstall button on every stub
    ///     (`removable = true`), so this must not error for stubs.
    /// Built-in / real-release plugins (in `BUNDLED_NON_REMOVABLE_IDS`) are
    /// rejected — they never get the button in the first place, but this is a
    /// backstop against direct IPC calls.
    pub fn uninstall(&self, kind: &str, id: &str) -> Result<(), AppError> {
        let user_dir = self
            .user_plugins_dir
            .read()
            .unwrap()
            .clone()
            .ok_or_else(|| AppError::msg("用户插件目录未初始化"))?;
        let manifests = self.manifests.read().unwrap();
        let m = manifests
            .iter()
            .find(|m| m.r#type == kind && m.id == id)
            .ok_or_else(|| AppError::msg(format!("找不到插件 {kind}:{id}")))?;
        // Defensive backstop: refuse to delete a bundled real-release plugin
        // even if a user-space manifest with the same id won the init()
        // collision. These rows never show the button (`removable = false`).
        if BUNDLED_NON_REMOVABLE_IDS.contains(&id) {
            return Err(AppError::msg(
                "该插件为产品自带的默认 release 音源，不可卸载",
            ));
        }
        let key = format!("{kind}:{id}");
        if m.folder.starts_with(&user_dir) {
            // Genuine user-installed plugin → delete the folder on disk.
            let folder = m.folder.clone();
            drop(manifests);
            std::fs::remove_dir_all(&folder)
                .map_err(|e| AppError::msg(format!("卸载失败：{e}")))?;
            let mut state = self.state.write().unwrap();
            state.enabled.remove(&key);
            state.order.remove(&key);
            drop(state);
            self.save();
        } else {
            // Bundled / dev stub: app-package folder can't be deleted, so just
            // hide it for this session. Release the read lock before writing.
            drop(manifests);
            self.hidden.write().unwrap().insert(key);
        }
        Ok(())
    }

    /// Import a third-party plugin from the user's filesystem — either a
    /// `plugin.json` file or the plugin folder that contains it. The whole
    /// folder is copied into `user_dir/<id>/` (keeping `entry` relative paths
    /// intact), then the user plugin dir is re-scanned so the next `plugin_list`
    /// discovers and registers it. Only declarative / wasm runtimes are
    /// importable (native sources are compiled in).
    pub fn import(&self, src: &std::path::Path) -> Result<String, AppError> {
        let user_dir = self
            .user_plugins_dir
            .read()
            .unwrap()
            .clone()
            .ok_or_else(|| AppError::msg("用户插件目录未初始化"))?;

        // Resolve the plugin root: a `plugin.json` file → its parent folder;
        // otherwise treat the path itself as the plugin folder.
        let root = if src.is_dir() {
            src.to_path_buf()
        } else if src.file_name().map(|n| n == "plugin.json").unwrap_or(false) {
            src.parent()
                .map(|p| p.to_path_buf())
                .ok_or_else(|| AppError::msg("无法解析插件清单路径"))?
        } else {
            return Err(AppError::msg("请选择 plugin.json 或插件文件夹"));
        };

        // Validate the manifest enough to derive `id` + `runtime`.
        let text = std::fs::read_to_string(root.join("plugin.json"))
            .map_err(|e| AppError::msg(format!("读取插件清单失败：{e}")))?;
        let manifest: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| AppError::msg(format!("插件清单不是合法 JSON：{e}")))?;
        let id = manifest
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if id.is_empty() || id.contains(['/', '\\']) || id.contains("..") {
            return Err(AppError::msg("插件清单缺少合法 id"));
        }
        let runtime = manifest
            .get("runtime")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if runtime != "declarative" && runtime != "wasm" {
            return Err(AppError::msg(format!(
                "暂不支持导入该运行时插件（runtime=\"{runtime}\"）；仅支持 declarative / wasm"
            )));
        }

        // Built-in plugins (Rust-compiled sources) are hard-locked: their
        // adapter is registered unconditionally at startup, so a same-id
        // user plugin would be silently shadowed by the built-in's pass 1
        // skip — the imported plugin would surface as a stub row but never
        // be invoked. Rather than letting the user waste cycles, reject up
        // front and steer them to rename `id` in plugin.json to a unique
        // value (e.g. `kuwo_custom`).
        if self.builtin_ids.read().unwrap().contains(&id) {
            return Err(AppError::msg(format!(
                "id `{id}` 与内置插件冲突，请将 plugin.json 里的 id 改为唯一的（如 `{id}_custom`）后重新导入"
            )));
        }

        let target = user_dir.join(&id);
        // Only block when a *real* plugin manifest already lives at the
        // target. Checking just `target.exists()` is too coarse: a stale
        // empty folder (e.g. left over from a prior partial import or an
        // older app version) blocks the import even though nothing is
        // registered for that id — and the user has no UI affordance to
        // clean it up, since there's no plugin row to "uninstall". The
        // presence of `plugin.json` is the actual "a plugin is here" signal.
        if target.join("plugin.json").exists() {
            return Err(AppError::msg(format!(
                "插件 {id} 已存在，请先卸载再导入"
            )));
        }

        copy_dir_all(&root, &target)
            .map_err(|e| AppError::msg(format!("复制插件目录失败：{e}")))?;

        // Re-scan the user plugin dir so the imported manifest is discovered.
        self.rescan_user(&user_dir);
        Ok(id)
    }

    /// Re-scan the user plugin dir and refresh `manifests` with what's actually
    /// on disk (dropping stale user-space entries first so uninstalled /
    /// overwritten plugins don't linger).
    fn rescan_user(&self, user_dir: &std::path::Path) {
        let mut manifests = self.manifests.write().unwrap();
        manifests.retain(|m| !m.folder.starts_with(user_dir));
        let Ok(entries) = std::fs::read_dir(user_dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let manifest_path = p.join("plugin.json");
            if !manifest_path.exists() {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&manifest_path) {
                if let Ok(mut m) = serde_json::from_str::<PluginManifest>(&text) {
                    m.folder = p;
                    // user-space wins on id collision (consistent with init)
                    manifests.retain(|x| !(x.r#type == m.r#type && x.id == m.id));
                    manifests.push(m);
                }
            }
        }
    }
}

/// Recursively copy a directory tree (`src` → `dst`), creating `dst` first.
fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let e = entry?;
        let ty = e.file_type()?;
        let from = e.path();
        let to = dst.join(e.file_name());
        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `PluginHost` for tests that don't actually exercise HTTP.
    ///
    /// `HttpHost` requires a live `tokio::runtime::Handle` so the JS plugin
    /// host bindings can drive reqwest synchronously — but the tests in
    /// this module never make HTTP calls, so we leak a tiny per-test
    /// runtime to satisfy the borrow checker (Handle is a reference into
    /// the runtime). Memory ceiling per test is constant (~few KB).
    fn test_host(
        metadata: Arc<crate::metadata::MetadataRegistry>,
        engines: Arc<crate::engine::EngineRegistry>,
    ) -> Arc<PluginHost> {
        // N11b: tests don't enter a tokio runtime, so init()'s
        // Handle::try_current will fail and js_http stays None — harmless
        // since none of the plugin_host tests exercise JS plugin loading.
        PluginHost::new(metadata, engines)
    }

    #[test]
    fn import_copies_rescans_rejects_duplicate_and_uninstalls() {
        let metadata = Arc::new(crate::metadata::MetadataRegistry::default());
        let engines = Arc::new(crate::engine::EngineRegistry::default());
        let host = test_host(metadata, engines);
        let tmp = std::env::temp_dir().join(format!("bilusic_plugin_import_{}", std::process::id()));
        let user_dir = tmp.join("user_plugins");
        let src = tmp.join("src_plugin");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("plugin.json"),
            r#"{"id":"testimport","name":"Test Import","version":"0.0.1","type":"metadata","runtime":"declarative","capabilities":["METADATA_SEARCH"]}"#,
        )
        .unwrap();
        std::fs::write(src.join("data.txt"), "hello").unwrap();
        *host.user_plugins_dir.write().unwrap() = Some(user_dir.clone());

        // Import the plugin folder → copied under user_dir/<id>/ + discovered.
        let id = host.import(&src).unwrap();
        assert_eq!(id, "testimport");
        let target = user_dir.join("testimport");
        assert!(target.join("plugin.json").exists(), "plugin.json copied");
        assert!(target.join("data.txt").exists(), "sidecar file copied");
        let all = host.all_plugins();
        assert!(all.iter().any(|p| p.id == "testimport"), "imported manifest discovered after rescan");
        // Imported manifest must surface as third-party (`builtin: false`,
        // `removable: true`). The two-pass `all_plugins` would otherwise hard-code
        // `builtin: true` for everything in the metadata registry.
        let imported = all
            .iter()
            .find(|p| p.id == "testimport")
            .expect("imported plugin item present");
        assert!(!imported.builtin, "imported plugin must be third-party");
        assert!(imported.removable, "imported plugin must be removable");

        // Duplicate import rejected.
        assert!(host.import(&src).is_err(), "duplicate import must fail");

        // Regression for the user-facing bug "importing a same-id-with-built-in
        // third-party plugin is blocked by a stale empty folder". An empty
        // `<user_dir>/<id>/` directory (no plugin.json inside) must NOT block
        // a fresh import — the import should populate plugin.json and proceed.
        let stale_id = "kuwo_stale";
        let stale_dir = user_dir.join(stale_id);
        std::fs::create_dir_all(&stale_dir).unwrap();
        // write a *different* id's plugin.json? No — simulate the exact failure
        // mode: folder exists, plugin.json absent.
        let stale_src = tmp.join("stale_src");
        std::fs::create_dir_all(&stale_src).unwrap();
        std::fs::write(
            stale_src.join("plugin.json"),
            format!(
                r#"{{"id":"{stale_id}","name":"Stale","version":"0.0.1","type":"metadata","runtime":"declarative","capabilities":["METADATA_SEARCH"]}}"#
            ),
        )
        .unwrap();
        // Pre-condition: stale folder exists, no plugin.json inside it.
        assert!(stale_dir.is_dir(), "stale folder pre-created");
        assert!(
            !stale_dir.join("plugin.json").exists(),
            "stale folder has no manifest yet (this is the bug condition)"
        );
        // Import must succeed despite the pre-existing empty folder.
        let imported_id = host.import(&stale_src).expect(
            "import must succeed over an empty stale folder (regression: was blocked by target.exists())",
        );
        assert_eq!(imported_id, stale_id);
        assert!(
            stale_dir.join("plugin.json").exists(),
            "import populated plugin.json into the previously-empty folder"
        );

        // Non-declarative/wasm runtime rejected.
        std::fs::write(src.join("plugin.json"), r#"{"id":"badrt","name":"Bad","version":"0.0.1","type":"metadata"}"#)
            .unwrap();
        assert!(host.import(&src).is_err(), "unknown runtime must fail");

        // Uninstall removes the folder.
        host.uninstall("metadata", "testimport").unwrap();
        assert!(!target.exists(), "folder removed after uninstall");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Regression: importing a third-party plugin whose `id` collides with a
    /// Rust-compiled built-in must be rejected up front. Otherwise the file
    /// would copy successfully, but `all_plugins()` pass 1 would skip it
    /// (`metadata.get(&id) == Some(builtin)`) and the user's plugin would
    /// silently sit as a stub row while the built-in keeps handling all
    /// traffic. The user has no in-product way to "delete" the built-in.
    ///
    /// N14 (2026-08-27): fixture id `kugou` → `netease` (still a real built-in,
    /// so the collision semantics are preserved; same conclusion holds).
    #[test]
    fn import_rejects_id_that_collides_with_builtin() {
        let metadata = Arc::new(crate::metadata::MetadataRegistry::default());
        let engines = Arc::new(crate::engine::EngineRegistry::default());
        let host = test_host(metadata, engines);

        // Register a fake built-in so `builtin_ids` is non-empty.
        struct FakeBuiltin;
        impl crate::plugin::Plugin for FakeBuiltin {
            fn info(&self) -> crate::plugin::PluginInfo {
                crate::plugin::PluginInfo::from(
                    "netease",
                    "Fake NetEase",
                    "0.0.0",
                    "fake built-in for collision test",
                    crate::plugin::PluginCapabilities::METADATA_SEARCH,
                )
            }
        }
        #[async_trait::async_trait]
        impl crate::metadata::MetadataSource for FakeBuiltin {
            async fn search(
                &self,
                _q: &str,
                _p: u32,
            ) -> Result<Vec<crate::metadata::MetadataTrack>, crate::error::AppError> {
                Ok(vec![])
            }
            async fn get_track(
                &self,
                _id: &str,
            ) -> Result<crate::metadata::MetadataTrack, crate::error::AppError> {
                Err(crate::error::AppError::msg("nope"))
            }
        }
        host.metadata.register(Arc::new(FakeBuiltin) as Arc<dyn crate::metadata::MetadataSource>);
        // Manually populate `builtin_ids` (production code does this in init()).
        host.builtin_ids.write().unwrap().insert("netease".into());

        let tmp = std::env::temp_dir().join(format!(
            "bilusic_plugin_import_builtin_collision_{}",
            std::process::id()
        ));
        let user_dir = tmp.join("user_plugins");
        let src = tmp.join("src_netease");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("plugin.json"),
            r#"{"id":"netease","name":"NetEase User","version":"0.0.1","type":"metadata","runtime":"declarative","capabilities":["METADATA_SEARCH"]}"#,
        )
        .unwrap();
        *host.user_plugins_dir.write().unwrap() = Some(user_dir.clone());

        let err = host
            .import(&src)
            .expect_err("importing id=netease (a built-in) must fail");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("netease") && (msg.contains("冲突") || msg.contains("冲突")),
            "error must name the conflicting id and surface the rename hint, got: {msg}"
        );
        assert!(
            !user_dir.join("netease").exists(),
            "rejected import must not copy anything into user_dir"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Regression test for the "imported plugin shows up as stub" failure:
    // (orphan `}` removed — pre-existing structural quirk that split the
    //  tests module is no longer needed now that `test_host` lives inside.)
    /// import a *complete* declarative manifest (mirrors the demo plugin)
    /// and assert it lands in the non-stub row — both `builtin: false` and
    /// `stub: false`. The first-pass `all_plugins` must call `register` for
    /// adapters produced by `load_runtime_plugin`; if it drops that step
    /// the manifest falls through to the stub pass.
    #[test]
    fn import_declarative_full_config_is_not_stub() {
        let metadata = Arc::new(crate::metadata::MetadataRegistry::default());
        let engines = Arc::new(crate::engine::EngineRegistry::default());
        let host = test_host(metadata, engines);

        let tmp = std::env::temp_dir().join(format!("bilusic_declarative_import_{}", std::process::id()));
        let user_dir = tmp.join("user_plugins");
        let src = tmp.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("plugin.json"),
            r#"{
                "id": "test-declarative-full",
                "name": "Test Declarative (full)",
                "version": "0.1.0",
                "type": "metadata",
                "runtime": "declarative",
                "capabilities": ["METADATA_SEARCH"],
                "declarative": {
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
                }
            }"#,
        )
        .unwrap();
        *host.user_plugins_dir.write().unwrap() = Some(user_dir.clone());

        host.import(&src).expect("import declarative plugin");
        let all = host.all_plugins();
        let item = all
            .iter()
            .find(|p| p.id == "test-declarative-full")
            .expect("imported declarative plugin in all_plugins");
        assert!(!item.builtin, "imported declarative plugin must be third-party");
        assert!(!item.stub, "imported declarative plugin with full config must not be stub");
        assert!(item.removable, "imported declarative plugin must be removable");

        // Cleanup.
        let _ = host.uninstall("metadata", "test-declarative-full");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Regression test for a design-level bug discovered on 2026-08-20 19:30:
    /// imported declarative plugins were intermittently flagged as built-in
    /// on the **second** call to `all_plugins()`. Earlier fixes used a
    /// transient `from_manifest` set populated only when pass 1 actually
    /// loaded a manifest. On the first call that worked (manifest not yet
    /// in metadata registry → pass 1 loaded it → inserted into
    /// `from_manifest`). On the second call pass 1's
    /// `if metadata.get(m.id).is_some() continue` short-circuited, so
    /// `from_manifest` never got the id — pass 2 saw it in registry but
    /// not in `from_manifest` and emitted `builtin: true`.
    ///
    /// The fix anchors `builtin` on the persistent `self.manifests` set,
    /// which doesn't change between calls. This test simulates the second
    /// call by invoking `all_plugins()` twice and asserting the imported
    /// plugin remains `!builtin && removable` both times.
    #[test]
    fn all_plugins_stable_across_repeated_calls() {
        let metadata = Arc::new(crate::metadata::MetadataRegistry::default());
        let engines = Arc::new(crate::engine::EngineRegistry::default());
        let host = test_host(metadata, engines);

        let tmp = std::env::temp_dir().join(format!(
            "bilusic_repeat_call_{}",
            std::process::id()
        ));
        let user_dir = tmp.join("user_plugins");
        let src = tmp.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("plugin.json"),
            r#"{
                "id": "test-declarative-stable",
                "name": "Test Declarative (stable)",
                "version": "0.1.0",
                "type": "metadata",
                "runtime": "declarative",
                "capabilities": ["METADATA_SEARCH"],
                "declarative": {
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
                }
            }"#,
        )
        .unwrap();
        *host.user_plugins_dir.write().unwrap() = Some(user_dir.clone());

        host.import(&src).expect("import declarative plugin");

        // First call — populates the registry from the manifest.
        let first = host.all_plugins();
        let first_item = first
            .iter()
            .find(|p| p.id == "test-declarative-stable")
            .expect("first call: imported plugin present");
        assert!(
            !first_item.builtin,
            "first call: imported plugin must be third-party (got builtin=true)"
        );
        assert!(
            first_item.removable,
            "first call: imported plugin must be removable"
        );
        assert!(
            !first_item.stub,
            "first call: imported declarative must not be stub"
        );

        // Second call — would surface the bug under the old `from_manifest`
        // design because the adapter is now in the metadata registry and
        // pass 1's `metadata.get(...).is_some()` short-circuits.
        let second = host.all_plugins();
        let second_item = second
            .iter()
            .find(|p| p.id == "test-declarative-stable")
            .expect("second call: imported plugin present");
        assert!(
            !second_item.builtin,
            "second call: imported plugin must still be third-party (regression)"
        );
        assert!(
            second_item.removable,
            "second call: imported plugin must remain removable (regression)"
        );
        assert!(
            !second_item.stub,
            "second call: imported declarative must not flip to stub (regression)"
        );

        // Cleanup.
        let _ = host.uninstall("metadata", "test-declarative-stable");
        let _ = std::fs::remove_dir_all(&tmp);
    }


    /// Bundled real-release plugins (currently `itunes-declarative`) must
    /// surface as `removable: false` and `uninstall()` must refuse them — even
    /// if a user-space manifest with the same id won the init() collision.
    /// Adding a new bundled release means appending its id to
    /// `BUNDLED_NON_REMOVABLE_IDS` and adding a parallel test here.
    #[test]
    fn bundled_real_release_is_non_removable_and_uninstall_rejected() {
        let metadata = Arc::new(crate::metadata::MetadataRegistry::default());
        let engines = Arc::new(crate::engine::EngineRegistry::default());
        let host = test_host(metadata, engines);

        let tmp = std::env::temp_dir().join(format!(
            "bilusic_protected_{}",
            std::process::id()
        ));
        let user_dir = tmp.join("user_plugins");
        let plugin_dir = user_dir.join("itunes-declarative");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{
                "id": "itunes-declarative",
                "name": "iTunes Search",
                "version": "0.1.0",
                "type": "metadata",
                "runtime": "declarative",
                "capabilities": ["METADATA_SEARCH"],
                "declarative": {
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
                }
            }"#,
        )
        .unwrap();
        *host.user_plugins_dir.write().unwrap() = Some(user_dir.clone());
        host.rescan_user(&user_dir);

        let items = host.all_plugins();
        let item = items
            .iter()
            .find(|p| p.id == "itunes-declarative")
            .expect("bundled real-release must appear in all_plugins()");
        assert!(
            !item.removable,
            "bundled real-release plugin must not be removable (got removable=true)"
        );

        let result = host.uninstall("metadata", "itunes-declarative");
        assert!(
            result.is_err(),
            "uninstall of protected bundled plugin must be rejected"
        );

        let items_after = host.all_plugins();
        assert!(
            items_after.iter().any(|p| p.id == "itunes-declarative"),
            "protected plugin must remain installed after rejected uninstall"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Regression test for the "built-in flipped to third-party" bug: when a
    /// bundled manifest ships with the same id as a Rust-compiled source
    /// (e.g. `qq_music/` plugin.json + `plugins/qq_music_metadata.rs`),
    /// `all_plugins` must still report the Rust source as `builtin: true`.
    /// The old design used `!manifest_ids.contains(id)` which flipped to
    /// `false` whenever a bundled stub shared the id — `builtin_ids`
    /// (the Rust-compiled set) is now the source of truth.
    ///
    /// N14 (2026-08-27): fixture id `kugou` → `qq_music` (still a real
    /// built-in, so the precedence semantics are preserved).
    #[test]
    fn builtin_takes_precedence_over_manifest_with_same_id() {
        let metadata = Arc::new(crate::metadata::MetadataRegistry::default());
        let engines = Arc::new(crate::engine::EngineRegistry::default());
        let host = test_host(metadata, engines);

        // Register a fake "built-in" Rust source directly (mirrors what
        // lib.rs does for `metadata.register(Arc::new(QqMusicMetadata::new()))`).
        struct FakeBuiltin;
        impl crate::plugin::Plugin for FakeBuiltin {
            fn info(&self) -> crate::plugin::PluginInfo {
                crate::plugin::PluginInfo::from(
                    "qq_music",
                    "Fake QQ Music",
                    "0.0.0",
                    "fake built-in for precedence test",
                    crate::plugin::PluginCapabilities::METADATA_SEARCH,
                )
            }
        }
        #[async_trait::async_trait]
        impl crate::metadata::MetadataSource for FakeBuiltin {
            async fn search(
                &self,
                _q: &str,
                _p: u32,
            ) -> Result<Vec<crate::metadata::MetadataTrack>, crate::error::AppError> {
                Ok(vec![])
            }
            async fn get_track(
                &self,
                _id: &str,
            ) -> Result<crate::metadata::MetadataTrack, crate::error::AppError> {
                Err(crate::error::AppError::msg("nope"))
            }
        }
        host.metadata
            .register(Arc::new(FakeBuiltin) as Arc<dyn crate::metadata::MetadataSource>);

        // Drop a bundled stub manifest with the same id under user_plugins_dir.
        let tmp = std::env::temp_dir().join(format!(
            "bilusic_precedence_{}",
            std::process::id()
        ));
        let user_dir = tmp.join("user_plugins");
        let plugin_dir = user_dir.join("qq_music");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{
                "id": "qq_music",
                "name": "QQ Music (bundled stub)",
                "version": "0.0.1",
                "type": "metadata",
                "runtime": "rust",
                "capabilities": ["METADATA_SEARCH"]
            }"#,
        )
        .unwrap();
        *host.user_plugins_dir.write().unwrap() = Some(user_dir.clone());
        host.rescan_user(&user_dir);

        // Populate builtin_ids (normally done in init()).
        let mut ids = std::collections::HashSet::new();
        for info in host.metadata.list_all() {
            ids.insert(info.id);
        }
        *host.builtin_ids.write().unwrap() = ids;

        let items = host.all_plugins();
        // TWO rows for id "qq_music": the Rust built-in AND the same-id bundled stub.
        let qq_items: Vec<_> = items.iter().filter(|p| p.id == "qq_music").collect();
        assert_eq!(
            qq_items.len(),
            2,
            "id collision: built-in + same-id bundled stub must both surface (got {})",
            qq_items.len()
        );
        // The Rust-compiled source stays builtin=true / stub=false / not removable.
        let builtin = qq_items
            .iter()
            .find(|p| p.builtin)
            .expect("builtin row must be present");
        assert!(
            builtin.builtin,
            "Rust-compiled source must remain builtin=true even with same-id manifest"
        );
        assert!(
            !builtin.stub,
            "Rust-compiled source must not be marked stub"
        );
        assert!(
            !builtin.removable,
            "builtin source must not be removable"
        );
        // The bundled stub is third-party (builtin=false) + stub=true. Because
        // it lives under user_plugins_dir in this test, it is removable.
        let stub = qq_items
            .iter()
            .find(|p| p.stub)
            .expect("bundled stub row must be present");
        assert!(
            !stub.builtin,
            "bundled stub mirroring a built-in must be third-party (builtin=false)"
        );
        assert!(stub.stub, "bundled stub must be marked stub");
        assert!(
            stub.removable,
            "bundled stub under user_plugins_dir must be removable"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Bundled stubs located outside the user plugin dir (dev cwd /
    /// resource_dir) must appear as `stub: true` rows AND carry
    /// `removable: true` so the UI shows the uninstall button. Because the
    /// folder lives inside the app package and cannot be deleted, `uninstall()`
    /// hides it for the current session (via the `hidden` set) instead of
    /// erroring — and the row then disappears from `all_plugins()`.
    #[test]
    fn bundled_stub_outside_user_dir_is_removable_and_hides_on_uninstall() {
        let metadata = Arc::new(crate::metadata::MetadataRegistry::default());
        let engines = Arc::new(crate::engine::EngineRegistry::default());
        let host = test_host(metadata, engines);

        // Drop a stub manifest under a "bundled" dir (not user_plugins_dir).
        let tmp = std::env::temp_dir().join(format!(
            "bilusic_stub_removable_{}",
            std::process::id()
        ));
        let bundled_dir = tmp.join("bundled_plugins");
        let plugin_dir = bundled_dir.join("netease_dec");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{
                "id": "netease_dec",
                "name": "NetEase (bundled stub)",
                "version": "0.0.1",
                "type": "metadata",
                "runtime": "rust",
                "capabilities": ["METADATA_SEARCH"]
            }"#,
        )
        .unwrap();
        // User dir is somewhere ELSE — this stub is "bundled" (dev/prod).
        *host.user_plugins_dir.write().unwrap() = Some(tmp.join("user_plugins"));
        *host.manifests.write().unwrap() = vec![crate::plugin_host::PluginManifest {
            id: "netease_dec".into(),
            name: "NetEase (bundled stub)".into(),
            version: "0.0.1".into(),
            description: String::new(),
            entry: String::new(),
            r#type: "metadata".into(),
            runtime: "rust".into(),
            capabilities: vec!["METADATA_SEARCH".into()],
            requires_auth: false,
            folder: bundled_dir.clone(),
            declarative: None,
        }];

        // Before uninstall: row present, removable, third-party.
        let items = host.all_plugins();
        let stub = items
            .iter()
            .find(|p| p.id == "netease_dec")
            .expect("bundled stub must appear in all_plugins()");
        assert!(stub.stub, "unimplemented runtime must produce stub row");
        assert!(
            !stub.builtin,
            "bundled stub mirroring a built-in must be third-party (builtin=false)"
        );
        assert!(
            stub.removable,
            "bundled stub must be removable so the uninstall button shows (got removable=false)"
        );

        // Uninstalling a bundled stub must NOT error — it just hides for the
        // session (the app-package folder cannot be deleted).
        host.uninstall("metadata", "netease_dec")
            .expect("uninstall on a bundled stub must succeed (session hide)");

        // After uninstall: the stub row is gone from the list.
        let items2 = host.all_plugins();
        assert!(
            !items2.iter().any(|p| p.id == "netease_dec"),
            "bundled stub must disappear from all_plugins() after uninstall()"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Regression for the front-end bug "clicking 卸载 on a same-id bundled
    /// stub also removes the built-in row". The contract this locks in: the
    /// back-end `uninstall()` MUST only hide the specific stub — it must
    /// never touch the registered adapter for a same-id built-in, so a
    /// subsequent `all_plugins()` still surfaces the built-in row.
    ///
    /// N14 (2026-08-27): fixture id `kugou` → `lrclib` (still a real built-in,
    /// and doesn't shadow the `netease_dec` stub id used by the
    /// bundled_stub_outside_user_dir_is_removable_and_hides_on_uninstall test).
    #[test]
    fn uninstalling_same_id_bundled_stub_leaves_builtin_intact() {
        let metadata = Arc::new(crate::metadata::MetadataRegistry::default());
        let engines = Arc::new(crate::engine::EngineRegistry::default());
        let host = test_host(metadata, engines);

        // Register a real built-in "lrclib" adapter.
        struct FakeBuiltin;
        impl crate::plugin::Plugin for FakeBuiltin {
            fn info(&self) -> crate::plugin::PluginInfo {
                crate::plugin::PluginInfo::from(
                    "lrclib",
                    "Fake Lrclib",
                    "0.0.0",
                    "fake built-in for regression test",
                    crate::plugin::PluginCapabilities::METADATA_SEARCH,
                )
            }
        }
        #[async_trait::async_trait]
        impl crate::metadata::MetadataSource for FakeBuiltin {
            async fn search(
                &self,
                _q: &str,
                _p: u32,
            ) -> Result<Vec<crate::metadata::MetadataTrack>, crate::error::AppError> {
                Ok(vec![])
            }
            async fn get_track(
                &self,
                _id: &str,
            ) -> Result<crate::metadata::MetadataTrack, crate::error::AppError> {
                Err(crate::error::AppError::msg("nope"))
            }
        }
        host.metadata.register(Arc::new(FakeBuiltin) as Arc<dyn crate::metadata::MetadataSource>);

        // Drop a bundled stub manifest with the same id under a non-user dir
        // (dev cwd / resource_dir style) so it routes to the session-hide branch.
        let tmp = std::env::temp_dir().join(format!(
            "bilusic_builtin_intact_{}",
            std::process::id()
        ));
        let bundled_dir = tmp.join("bundled_plugins");
        let plugin_dir = bundled_dir.join("lrclib");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{
                "id": "lrclib",
                "name": "Lrclib (bundled stub)",
                "version": "0.0.1",
                "type": "metadata",
                "runtime": "rust",
                "capabilities": ["METADATA_SEARCH"]
            }"#,
        )
        .unwrap();
        *host.user_plugins_dir.write().unwrap() = Some(tmp.join("user_plugins"));
        *host.manifests.write().unwrap() = vec![crate::plugin_host::PluginManifest {
            id: "lrclib".into(),
            name: "Lrclib (bundled stub)".into(),
            version: "0.0.1".into(),
            description: String::new(),
            entry: String::new(),
            r#type: "metadata".into(),
            runtime: "rust".into(),
            capabilities: vec!["METADATA_SEARCH".into()],
            requires_auth: false,
            folder: bundled_dir.clone(),
            declarative: None,
        }];

        // Populate builtin_ids (normally done in init()).
        let mut ids = std::collections::HashSet::new();
        for info in host.metadata.list_all() {
            ids.insert(info.id);
        }
        *host.builtin_ids.write().unwrap() = ids;

        // Sanity: two rows surface for "lrclib" (built-in + bundled stub).
        let before: Vec<_> = host
            .all_plugins()
            .into_iter()
            .filter(|p| p.id == "lrclib")
            .collect();
        assert_eq!(before.len(), 2, "expected built-in + bundled stub");
        assert!(
            before.iter().any(|p| p.builtin && !p.stub),
            "built-in row must be present before uninstall"
        );
        assert!(
            before.iter().any(|p| !p.builtin && p.stub),
            "bundled stub row must be present before uninstall"
        );

        // Uninstall the bundled stub.
        host.uninstall("metadata", "lrclib")
            .expect("uninstall on bundled stub must succeed");

        // The built-in adapter MUST remain registered.
        assert!(
            host.metadata.get("lrclib").is_some(),
            "built-in adapter must NOT be removed by uninstalling the same-id bundled stub"
        );

        // After uninstall: only the built-in row remains for "lrclib".
        let after: Vec<_> = host
            .all_plugins()
            .into_iter()
            .filter(|p| p.id == "lrclib")
            .collect();
        assert_eq!(
            after.len(),
            1,
            "expected only the built-in row after stub uninstall (got {})",
            after.len()
        );
        let only = &after[0];
        assert!(
            only.builtin && !only.stub,
            "the surviving row must be the built-in, not a stub"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
