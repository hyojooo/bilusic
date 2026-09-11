# Bilusic 插件开发规范（plugin-spec）

> **项目**：Bilusic —— Tauri 桌面音乐应用，以 Bilibili 为音源，对标 Spotube
> **配套文档**：`docs/plan.md`（v1.5，架构蓝图）、`docs/PROGRESS.md`（推进节点）
> **文档性质**：插件开发者手册 + 架构契约
> **最后更新**：2026-08-18

---

## 0. 一句话定位

Bilusic 的「音源」不是一个整体，而是**两个正交的角色**，由 `Coordinator` 编排：

- **元数据源（MetadataSource）**：回答「这首歌是什么」——标题、艺人、封面、时长、歌词。
- **音频引擎（AudioEngine）**：回答「这首歌的音频字节去哪取」——把一首歌解析成可播放的流地址。

前端**永远不直接**调用元数据或引擎，只走 `Coordinator`。用户可独立选择
「元数据源 = X」与「音频引擎 = Y」，因此「QQ 音乐元数据 → Bilibili 引擎」这类跨源组合是合法的——这正是 Spotube（Spotify=元数据，YouTube=音频）的模型。

---

## 1. 两类插件 + 能力位标志

每种插件都实现基础 `Plugin` trait，并声明一组 **`PluginCapabilities`** 位标志，
让协调器与 UI 在不依赖 Rust 反射的情况下判断它能做什么。

| 能力位 | 值（bit） | 含义 | 由谁声明 |
|--|--|--|--|
| `METADATA_SEARCH` | 1 << 0 | 支持关键词搜索 | 元数据源 |
| `METADATA_GET` | 1 << 1 | 支持按 id 取完整元数据 | 元数据源 |
| `METADATA_LYRICS` | 1 << 2 | 支持取歌词（默认空实现） | 元数据源 |
| `METADATA_HOME` | 1 << 3 | 支持首页/发现 feed（歌单/新歌/专辑/歌手） | 元数据源 |
| `AUDIO_BY_ID` | 1 << 4 | 支持按引擎原生 id 解析流 | 音频引擎 |
| `AUDIO_BY_QUERY` | 1 << 5 | 支持按「标题+艺人」跨源搜索解析 | 音频引擎 |
| `AUDIO_NEEDS_AUTH` | 1 << 6 | 完整功能需登录 Cookie（如 SESSDATA） | 音频引擎 |

> 同一插件可以**同时**声明两侧能力（例如 Bilibili 既做元数据也做引擎），
> 但 UI 在「元数据源」与「音频引擎」两张卡片里会各列出一次。
> `capabilities` 在 `PluginInfo` 中以 `u32` 位掩码序列化给前端。

---

## 2. 数据契约（跨前后端共享，serde 序列化）

所有结构在 Rust 侧 `#[derive(Serialize, Deserialize)]`，前端用同名 TS 接口镜像
（`src/lib/bili.ts`）。**改字段必须双端同步**。

### 2.1 `PluginInfo`（插件名片）
```ts
interface PluginInfo {
  id: string;          // 唯一，小写 kebab（如 "bilibili" / "qq-music"）
  name: string;        // 展示名
  version: string;     // 语义化版本
  description: string; // 一句话说明
  capabilities: number; // 上文位掩码
}
```

### 2.2 `MetadataTrack`（一首歌的元数据）
```ts
interface MetadataTrack {
  source_id: string;     // 在来源内的唯一 id（B 站= bvid，QQ= 歌曲 mid…）
  source: string;        // 来源插件 id（如 "bilibili"）
  engine_hint: string | null; // 建议用哪个引擎解析（协调器优先采用）
  title: string;
  artist: string;
  album: string;         // 无专辑概念时可放「播放量 / 分类」等补充信息
  duration_ms: number;
  cover: string;         // 封面 URL（建议 https）
  lyrics_id: string | null; // 取歌词用的 canonical id
}
```
**关键规则**：`engine_hint` 由协调器在搜索时按需填写；插件自身应填自己能对应的
引擎 id（B 站填 `"bilibili"`）。跨源场景（QQ 元数据 → B 站引擎）时，
`engine_hint` 填 `"bilibili"` 即可触发跨源解析路径。

### 2.3 `StreamInfo`（音频流描述）
```ts
interface StreamInfo {
  url: string;        // 直链 CDN 地址（前端会包到本地代理后再播放）
  mime: string;       // 如 "audio/mp4"
  bitrate: number;    // 码率（bps 量级，用于展示/择优）
  duration_ms: number;
  cover: string;
  title: string;
  artist: string;
}
```

### 2.4 `Lyrics`
```ts
interface Lyrics { lines: { time_ms: number; text: string }[]; }
```

### 2.4.1 首页 feed（`HomeFeed`）
首页（发现页）从活跃元数据源拉取的浏览内容。**声明了 `METADATA_HOME` 的源必须实现 `home()`**；未声明的源返回空 feed（前端显示空态提示而非崩溃）。

```ts
interface FeedItem {        // 歌单 / 专辑 / 歌手 卡片
  id: string;
  title: string;
  subtitle: string | null;
  cover: string;
  kind: "playlist" | "album" | "artist";
  source: string;
}
interface FeedSong {        // 可播放的新歌
  source_id: string;       // 源内歌曲 id（如 songmid）
  title: string;
  artist: string;
  cover: string;
  duration_ms: number;
  source: string;
  engine_hint: string | null; // 通常由源填 "bilibili"，走跨源解析
}
interface HomeFeed {
  playlists: FeedItem[];
  new_songs: FeedSong[];
  albums: FeedItem[];
  artists: FeedItem[];
}
```
> 跨源要点：`FeedSong.engine_hint` 填 `"bilibili"` 时，点击播放会经 `Coordinator` 选 B 站引擎、走 `find_stream_by_query("标题 艺人")` 解析音频——即「QQ 元数据 → B 站音频」链路。

### 2.5 `PlaybackInfo`（协调器产出，喂给播放器）
```ts
interface PlaybackInfo {
  track: MetadataTrack;
  stream: StreamInfo;
  engine: string; // 实际选中的引擎 id
}
```

---

## 3. 内置插件（Rust trait 实现，已落地 ✅）

当前所有插件用 **Rust trait** 实现，编译期注册进 `MetadataRegistry` / `EngineRegistry`，
与 `Coordinator` 共享 `Arc`。这是 MVP 的最快路径，也是第三方插件的参照实现。

### 3.1 基础 `Plugin` trait
```rust
pub trait Plugin: Send + Sync {
    fn info(&self) -> PluginInfo;
    fn capabilities(&self) -> PluginCapabilities { /* 默认从 info() 读取 */ }
}
```

### 3.2 `MetadataSource` trait（`#[async_trait]`）
```rust
#[async_trait]
pub trait MetadataSource: Plugin {
    async fn search(&self, query: &str, page: u32) -> Result<Vec<MetadataTrack>, AppError>;
    async fn get_track(&self, id: &str) -> Result<MetadataTrack, AppError>;
    async fn get_lyrics(&self, _id: &str) -> Result<Lyrics, AppError> {
        Ok(Lyrics { lines: vec![] }) // 默认无歌词
    }
}
```

### 3.3 `AudioEngine` trait（`#[async_trait]`）
```rust
#[async_trait]
pub trait AudioEngine: Plugin {
    // 按引擎原生 id 解析（B 站=bvid）
    async fn find_stream_by_id(&self, id: &str) -> Result<StreamInfo, AppError>;

    // 按「标题+艺人」跨源解析；默认返回不支持，需跨源的引擎重写
    async fn find_stream_by_query(&self, _query: &str, _hint: &MetadataTrack)
        -> Result<StreamInfo, AppError> {
        Err(AppError::msg("该音频引擎不支持跨源搜索（仅支持按 ID 解析）"))
    }
}
```

### 3.4 注册与接线（见 `src-tauri/src/lib.rs`）
```rust
let bilibili_metadata = Arc::new(BilibiliMetadata::new());
let bilibili_engine   = Arc::new(BilibiliEngine::new());

let mut metadata_reg = MetadataRegistry::default();
metadata_reg.register(bilibili_metadata.clone());
let mut engine_reg = EngineRegistry::default();
engine_reg.register(bilibili_engine.clone());

let coordinator = Coordinator::new(Arc::new(metadata_reg), Arc::new(engine_reg));

Builder::default()
    .manage(coordinator)
    .manage(PluginCookieService { bilibili_metadata, bilibili_engine })
    .setup(|_app| { proxy::start_proxy(); Ok(()) })
    .invoke_handler(tauri::generate_handler![ /* 见 §5 */ ]);
```
**要点**：同一 `Arc` 实例同时进 registry（供发现）和 `PluginCookieService`（供运行时
注入 Cookie），保证设置页改 Cookie 立刻对两个 B 站插件生效。

> **P5 更新**：注册表构造后交给 `PluginHost::new(...)`，再 `Coordinator::new(host)`。
> `PluginHost` 在 `setup` 时额外扫描 `apps/desktop/plugins/*/plugin.json`（见 §8），
> 合并第三方清单并负责启用 / 排序 / 卸载状态。内置注册逻辑不变，只是统一由 host 持有。

### 3.5 命名与版本约定
- `id` 必须全局唯一（跨元数据源 + 音频引擎一起算），小写 kebab。
- `engine_hint` 必须等于某个已注册引擎的 `id`，否则协调器回退到用户当前活跃引擎。
- 版本号走 `CARGO_PKG_VERSION`（内置插件自动同步）。

### 3.6 内置示例（Rust trait 实现）
| 插件 | 角色 | 声明的能力 | 备注 |
|--|--|--|--|
| `qq-music`（metadata，**默认元数据源**） | 元数据源 | `METADATA_SEARCH \| METADATA_GET \| METADATA_HOME` | QQ 音乐搜索 / 歌单 / 新歌 / 专辑 / 歌手；`engine_hint` 指向 `bilibili` |
| `bilibili`（engine） | 音频引擎 | `AUDIO_BY_ID \| AUDIO_BY_QUERY \| AUDIO_NEEDS_AUTH` | DASH 音频优先，回退 `durl` MP4 |
| `bilibili`（metadata，可选保留） | 元数据源 | `METADATA_SEARCH \| METADATA_GET` | B 站视频元数据，作为元数据源备选；匿名可能触发 `v_voucher` 风控（见 §7） |

> 角色分工（Spotube 模型）：**Bilibili 只做音频引擎**；**元数据来自音乐服务插件**（QQ 内置，网易云/酷狗/酷我规划中）。默认 `active_metadata = qq-music`，`active_engine = bilibili`。

两个插件共用同一 `cookie`（SESSDATA）字段，由 `PluginCookieService` 注入。

---

## 4. `Coordinator`（唯一的对外入口）

前端只与协调器对话，它掌握当前的 `(active_metadata, active_engine)` 配对与 Cookie。

```rust
pub struct Coordinator {
    host: Arc<PluginHost>,                 // 内置注册表 + 第三方清单 + 启用/排序状态
    pub active_metadata: RwLock<String>,   // 当前激活元数据源 id
    pub active_engine:   RwLock<String>,   // 当前激活音频引擎 id
}
```

> 注册表（`MetadataRegistry` / `EngineRegistry`）的键已从 `&'static str` 改为 `String`，
> 以支持运行时动态增减（第三方清单）。`active_*` 改为 `String` 以便持久化与切换。
> 内置插件仍编译期注册进 registry；第三方清单由 `PluginHost` 在 `setup` 时扫描注入。

**`play(track)` 选引擎逻辑**：
1. 若 `track.engine_hint` 指向一个已注册引擎 → 用它；否则用 `active_engine`。
2. 若 `engine_id == track.source`（同源，如都是 bilibili）→ `find_stream_by_id(source_id)`。
3. 否则（跨源）→ `find_stream_by_query("标题 艺人", track)`。

**`search(query, page)`**：委托给 `active_metadata` 指向的源。

> 这条规则意味着：只要一个引擎实现了 `find_stream_by_query`，它就能给**任意**元数据
> 源兜底。Bilibili 引擎已支持，所以「QQ 元数据 + B 站音频」开箱即用。

---

## 5. IPC 命令（前端 ↔ 协调器）

所有命令见 `src-tauri/src/commands.rs`，前端封装见 `src/lib/bili.ts`。

| 命令 | 入参 | 返回 | 说明 |
|--|--|--|--|
| `list_plugins` | — | `StateSnapshot` | 列出元数据源 + 音频引擎 + 当前激活项 |
| `set_active_metadata` | `id: string` | `()` | 切换活跃元数据源 |
| `set_active_engine` | `id: string` | `()` | 切换活跃音频引擎 |
| `metadata_search` | `query, page?` | `MetadataTrack[]` | 走当前元数据源搜索 |
| `coordinator_play` | `track: MetadataTrack` | `PlaybackInfo` | 经协调器选引擎并解析流 |
| `play_by_bvid` | `bvid: string` | `PlaybackInfo` | 跳过元数据，直接用活跃引擎解析 BV 号（v_voucher 绕过通道） |
| `set_bilibili_cookie` | `cookie: string \| null` | `()` | 注入 SESSDATA 到两个 B 站插件 |
| `proxy_base_url` | — | `string` | 本地代理基址（见 §6） |
| `plugin_list` | — | `PluginListItem[]` | P5：列出全部插件（内置 + 第三方清单），含启用/排序/stub/内置/可卸载标记 |
| `plugin_toggle` | `kind, id, enabled` | `()` | P5：启用 / 禁用某插件（持久化到 `app_data_dir/plugin_state.json`） |
| `plugin_reorder` | `ordered: [kind, id][]` | `()` | P5：按可见顺序重排插件（分配顺序值） |
| `plugin_uninstall` | `kind, id` | `()` | P5：卸载第三方插件（删除其文件夹；内置 / 打包插件拒绝） |

`PluginListItem`（设置页管理行）：
```ts
interface PluginListItem {
  id: string;
  kind: string;           // "metadata" | "engine" | "lyrics"
  name: string;
  version: string;
  description: string;
  capabilities: number;   // 位掩码（见 §1）
  enabled: boolean;
  order: number;
  stub: boolean;          // 已发现清单但无编译适配器（第三方运行时插槽待落地）
  builtin: boolean;       // 内置 Rust 插件，不可卸载
  removable: boolean;     // 位于用户插件目录，可卸载
}
```

`StateSnapshot`：
```ts
interface StateSnapshot {
  metadata_sources: PluginInfo[];
  audio_engines: PluginInfo[];
  active_metadata: string;
  active_engine: string;
}
```

**错误契约**：所有命令返回 `Result<T, AppError>`，序列化形如 `{ code, message }`
（必要时含 `detail`）。前端按 `code` 做差异化提示（如 403 风控引导填 Cookie）。

---

## 6. 本地流代理契约（必读）

B 站等 CDN 有防盗链（Referer / Range / Cookie 校验），前端直接 `fetch` 会被拦。
Rust 侧起一个 **axum 代理**（默认 `http://127.0.0.1:9527`）：

- 前端把直链包成 `http://127.0.0.1:9527/stream?u=<encodeURIComponent(url)>`。
- 代理转发原始请求的 `Range` / `Referer` / `Cookie` 到 CDN，回传音频字节。
- 支持 `206 Partial Content`，所以**拖动进度条可用**。

**插件作者的义务**：`AudioEngine` 返回的 `StreamInfo.url` 是**直链**，
由前端统一经 `proxyStreamUrl()` 包裹，**插件不必自己处理代理**。

---

## 7. 凭据与安全（Cookie / SESSDATA）

- 登录态（如 B 站 `SESSDATA`）**不落明文文件**，由设置页经 `set_bilibili_cookie`
  暂存于进程内的 `PluginCookieService`，随插件 `Arc` 共享。
- 第三方插件（P5）**禁止自行读取任意凭据**，只能声明 `requires_auth`，由宿主从
  OS 钥匙串注入，并经宿主 API 取用。
- `AUDIO_NEEDS_AUTH` 位用于 UI 提示「该引擎需要登录」。

---

## 8. 插件系统（P5 运行时无关基础 ✅ + P5-II 原生优先 ✅，第三方运行时 = 预留插槽 ⏳）

> **状态（2026-08-18 N9 / N9b）**：
> - **P5 第一层**：`PluginHost` 扫描并加载 `plugin.json` 清单、合并内置插件、
>   持久化启用/排序/卸载状态，设置页可视化启停/排序/卸载。✅
> - **P5-II（决策：原生优先，不引入 deno_core / V8）**：6 个首发音源
>   （`lrclib` 完整歌词 + `netease`/`kugou`/`kuwo`/`spotify`/`ytmusic` 骨架）
>   均为**编译期 Rust trait 实现**（`src-tauri/src/plugins/*.rs`），启动直接注册进
>   registry——**零运行时体积、零 V8**。它们同时带 `plugin.json` 清单供展示/启停排序。
>   5 个骨架源默认**关闭**（`PluginHost::init` 的 `EXPERIMENTAL` 列表），待阶段 2 补全
>   接口后开启。
> - **第三方运行时 = 预留插槽**：`PluginHost::load_runtime_plugin()` 是唯一的插入点，
>   今天返回 `Err`（清单无编译适配器 → `stub: true`）。将来若要开放终端用户装插件，
>   只需实现这一个函数（JS 经 deno_core 或 WASM 经 extism/wasmtime 均可），产出
>   `Arc<dyn MetadataSource>`/`Arc<dyn AudioEngine>` 注册，**与原生源对称共存**。
>   当前明确**不采用 deno_core**（把整個 V8 塞进二进制，包体 +10~几十 MB、编译数分钟、
>   V8 isolate `!Send+!Sync` 需绕桥接；对本地播放器属 over-engineering）。真要开放
>   第三方运行时，WASM 是比 V8 更轻的安全替代。

### 8.1 目录与文件
```
apps/desktop/plugins/
├── README.md        # 插件目录说明 + 契约（本仓库自带，供开发参考）
├── netease/         plugin.json + index.js   # 网易云音乐（metadata）
├── kugou/           plugin.json + index.js   # 酷狗音乐（metadata）
├── kuwo/            plugin.json + index.js   # 酷我音乐（metadata）
├── spotify/         plugin.json + index.js   # Spotify（metadata，需 OAuth）
├── ytmusic/         plugin.json + index.js   # YouTube Music（metadata）
└── lrclib/          plugin.json + index.js   # LrcLib（lyrics）
```
> 用户自定义插件可放入 `<app_data_dir>/plugins/<id>/`（macOS：
> `~/Library/Application Support/<app_id>/plugins/`），重启即被发现且可卸载。

### 8.2 `plugin.json`
```json
{
  "id": "netease",
  "name": "网易云音乐",
  "version": "0.1.0",
  "type": "metadata",                       // "metadata" | "engine" | "lyrics"
  "description": "网易云音乐元数据源：搜索 / 曲目详情 / 歌词 / 首页推荐。",
  "entry": "index.js",
  "capabilities": [
    "METADATA_SEARCH", "METADATA_GET", "METADATA_LYRICS", "METADATA_HOME"
  ],
  "requires_auth": false
}
```
字段说明见 `apps/desktop/plugins/README.md`。`type` 决定插件在架构中的角色；
`capabilities` 能力名映射到 §1 的位掩码（宿主用 `capability_bits()` 转换）。

### 8.3 实现位置与 `index.js` 契约
- **首发音源（权威实现）**：均为 Rust，位于 `src-tauri/src/plugins/*.rs`
  （`qq_music_metadata` / `bilibili_*` / `lrclib` / `netease` / `kugou` / `kuwo` /
  `spotify` / `ytmusic`）。`lib.rs` 构造 `Arc` 直接注册进 registry。
- **`index.js`（仅未来第三方运行时契约）**：当前各 `plugins/<id>/index.js` 仍是
  **占位桩**，仅约定导出契约；它们**不被**原生实现使用。若将来接入 JS/WASM 运行时，
  该运行时按以下映射加载脚本并实现 trait：
  - `type=metadata` 导出：`search(query, page)`、`getTrack(id)`、`getLyrics(track)`、`home()`。
  - `type=engine` 导出：`findStreamById(id)`、`findStreamByQuery(query, hint)`。
  - `type=lyrics` 导出：`getLyrics(track)`。
  - 返回值必须与 §2 的 `MetadataTrack` / `StreamInfo` / `Lyrics` 字段一致。

### 8.4 发现与状态（`PluginHost`）
- **发现根**：用户目录 `app_data_dir/plugins`、打包资源 `resource_dir/plugins`、
  以及 `tauri dev` 时的 `cwd/plugins`（即 `apps/desktop/plugins`）。
  多根按 `type:id` 去重，用户目录优先（可覆盖同名打包示例）。
- **持久化**：启用/排序状态写入 `app_data_dir/plugin_state.json`
  （`PluginStateStore { enabled: Map, order: Map }`，键为 `type:id`）。
- **内置（首发音源）vs 第三方**：首发音源走 Rust trait 并**同时**带 `plugin.json`，
  注册进 registry 后由 `all_plugins()` 以内置行（`builtin: true, stub: false`）展示，
  且跳过清单行避免重复。第三方（用户放入 `app_data_dir/plugins` 或未来运行时）若无
  编译适配器则 `stub: true`，`removable` 取决于是否位于用户目录。
- **`EXPERIMENTAL` 默认关闭**：`netease`/`kugou`/`kuwo`/`spotify`/`ytmusic` 在
  `init()` 中默认 `enabled=false`，避免接口未补全前成为默认激活源；接口落地后从列表移除即可。
- **`type=lyrics`** 仅提供歌词，不参与元数据搜索 / 播放选择，UI 在插件页单独标注。

### 8.5 管理命令（设置页调用，见 §5 表）
`plugin_list`（拉全量）→ `plugin_toggle`（启停）→ `plugin_reorder`（排序）
→ `plugin_uninstall`（卸载第三方）。全部经 `Coordinator.host` 落地并即时持久化。

### 8.6 第三方运行时 = 预留插槽（不采用 deno_core）
**决策**：本项目不引入 deno_core / V8。理由——V8 静态库 ~30–50MB 链入二进制
（包体 +10~几十 MB、首次编译数分钟），且 V8 isolate `!Send+!Sync` 需额外线程+channel
桥接；对本地 B 站音乐播放器属 over-engineering。Rust 动态加载（dylib）亦不采用
（无稳定 ABI、卸载 segfault 雷区、打包撞 Gatekeeper/SmartScreen）。

**单一插入点**：`PluginHost::load_runtime_plugin(manifest)`。今天它返回 `Err`，
故无适配器的清单 `stub: true`。将来若需开放终端用户装插件，只需实现这一个函数——
用 deno_core（JS）或 extism/wasmtime（WASM，更轻）加载 `entry` 脚本，经**宿主 op 桥接**
（Rust `reqwest` 统一发请求、凭证隔离）产出 `Arc<dyn MetadataSource>` /
`Arc<dyn AudioEngine>` 并注册，与原生源对称共存。`all_plugins()` 已对"已注册清单"
去重，运行时插件注册后不会重复列出。

> 沙箱约束（若实现运行时仍适用）：禁止 `eval` / `Function` 构造器 / 动态 `import`；
> 无直接网络/文件权限，联网走宿主桥接；凭证只能经宿主 API 取用。

---

## 9. 插件开发 Checklist（以新增一个元数据插件为例）

- [ ] 选唯一 `id`（小写 kebab），决定 `type`（metadata / engine）。
- [ ] 声明正确的 `capabilities` 位；若需登录填 `requires_auth` / `AUDIO_NEEDS_AUTH`。
- [ ] 实现对应 trait 方法，返回的 `MetadataTrack` / `StreamInfo` 字段完整（尤其 `engine_hint`）。
- [ ] 错误统一用 `AppError`（带 `code`），不要抛裸字符串。
- [ ] 返回的流 URL 用直链即可，代理由前端统一包裹（§6）。
- [ ] 注册进对应 registry，并与 `Coordinator` 共享 `Arc`（Rust 内置）或放入
      `apps/desktop/plugins/<id>/`（P5 第三方）。
- [ ] UI 验证：设置页双卡片能列出、能切换；搜索 / 播放全链路打通。
- [ ] 跨源场景：若想让本引擎给别家元数据兜底，务必实现 `find_stream_by_query`。

---

## 10. 已知限制与陷阱

- **`v_voucher` 风控**：B 站匿名 `search/type` 会返回 `v_voucher` 挑战，搜索为空。
  已在前端给出明确提示，引导「设置 → 填 SESSDATA」或「直接用 BV 号播放」
  （`play_by_bvid` 绕过搜索）。`view` / `playurl` 匿名通常正常。完整搜索列为 P2 风险项。
- **`engine_hint` 必须命中已注册引擎 id**，否则协调器回退活跃引擎——若填错会出现
  「用错引擎解析」的静默失败。
- **直链防盗链**：切勿让前端直连 CDN，一律经 §6 代理。
- **改数据结构要双端同步**：Rust 的 `#[derive(Serialize, Deserialize)]` 与
  前端 `src/lib/bili.ts` 的接口必须一致，否则 IPC 反序列化失败。

---

_本规范 §8 已落地 P5（清单驱动：发现 / 持久化 / 管理命令）与 P5-II（原生优先：6 首发音源
编译期 Rust 实现 + 预留第三方运行时插槽 `load_runtime_plugin`）。§8.6 明确不采用
deno_core/V8，并给出未来 JS/WASM 运行时的单一插入点。_
