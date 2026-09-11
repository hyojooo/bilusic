# Bilusic 开发计划

> **项目名称**：Bilusic
> **定位**：桌面音乐应用（macOS / Windows / Linux），以 Bilibili 作为音频源，对标 Spotube
> **核心原则**：开源、跨平台、去 AI 化的精致 UI、可插拔元数据源、仅个人学习研究用途
> **文档版本**：v1.7 — 已同步 N5a、N5b、N6、N7（含 N7b–N7d）、N8、N9（P5 插件系统运行时无关基础）、**N9b（P5-II 原生优先：6 首发音源编译期 Rust trait + `load_runtime_plugin` 预留插槽，不引入 deno_core/V8）**；P0–P5 基础与 P5-II 原生部分已完成，P6 待启动

---

## 0. FAQ 优先回答

### Q：Spotube 有 YouTube 引擎选项，Bilusic 需不需要对应的「Bilibili 引擎」？

**需要，但内涵不同。（✅ 已确认：内置 `bilibili-official` + `bilibili-public` 双引擎，并保留插件化扩展位）**

| | Spotube | Bilusic |
|--|--|--|
| 音频源 | YouTube（多第三方反代：NewPipe / Piped / Invidious） | **Bilibili** |
| 「引擎」含义 | 拉音频流的不同后端 | 拉音频流的不同后端 |
| 是否需要 | ✅ 有 | ✅ **有** |

**Bilusic 的引擎层设计建议（N5a 已落地为「元数据 / 音频引擎彻底解耦」）：**

1. **两个正交 trait，而非一个焊死的 `BiliSource`**（参考 Spotube 的「Spotify=元数据 / YouTube=音频」模型）：
   - `MetadataSource` —— 回答「这个 track 是什么」：`search` / `get_track` / `get_lyrics`（标题 / 艺人 / 封面 / 时长 / 歌词）。
   - `AudioEngine` —— 回答「这个 track 的音频字节去哪取」：`find_stream_by_id` / `find_stream_by_query`（DASH 优先，回退 `durl`）。
2. **`Coordinator`（协调器）是唯一对外入口**：前端永不直连元数据 / 引擎，永远经过协调器编排。它保存 `active_metadata` / `active_engine` 状态，`search()` → 元数据，`play(track)` → 自动选引擎（优先 `track.engine_hint`，回退到用户当前活跃引擎）。因此「QQ 音乐元数据 → Bilibili 引擎」这类跨源组合是合法的。
3. 内置插件按「角色」拆分；**Bilibili 仅作音频引擎**（提供音频字节），**元数据来自音乐服务插件**：
   - `qq-music`（元数据，**内置**）—— QQ 音乐搜索 / 歌单 / 新歌 / 专辑 / 歌手，`engine_hint` 指向 `bilibili`。
   - `bilibili-engine`（音频引擎）—— B 站 `playurl` 解析（DASH 优先、回退 `durl`）。
   - `bilibili-metadata`（元数据，可选保留）—— B 站视频元数据，作为元数据源的备选。
   - 规划中的元数据插件形式：**网易云、酷狗、酷我**（均走「元数据 → B 站音频」跨源链路）。
4. 插件以 Rust trait（`Plugin` + `PluginCapabilities` 位标志 METADATA_*/AUDIO_*）注册进 `MetadataRegistry` / `EngineRegistry`；**内置音源即用编译期 Rust trait 实现（MVP 优先）**。第三方运行时的插槽（`PluginHost::load_runtime_plugin`）在 P5 预留，但 **N9b 决策不引入 deno_core / V8**（对本地播放器属 over-engineering），若将来真要开放终端用户装第三方插件，WASM（extism/wasmtime）是比 V8 更轻的安全替代。
5. 设置 → 音源：用「元数据源」「音频引擎」两张卡片式选择器分别切换，与 Spotube 截图同款交互。

---

## 1. 功能模块划分

按职责将应用拆为 **8 个一级模块**，每模块再细分二级。

### M1. 核心壳层（Shell）
- Tauri 主进程、窗口管理、系统托盘、原生菜单
- 自动更新（tauri-updater）
- 全局快捷键（媒体键 / 自定义）

### M2. 音源引擎层（Source / Engine）
- **`MetadataSource` trait**：搜索 / 取元数据 / 取歌词（可为 B 站、QQ、网易云、Spotify…）
- **`AudioEngine` trait**：按 id / 关键词解析音频流（DASH / durl，B 站、第三方反代…）
- **`Coordinator`**：编排元数据 ↔ 引擎，对外唯一入口
- WBI 签名 / 风控规避 / Cookie 管理（B 站引擎后端）
- 视频流地址解析（dash / mp4 / hls）
- **「元数据源」与「音频引擎」均可独立切换**（对应 Spotube 的引擎选项）

### M3. 搜索与发现
- 关键词搜索（视频 / 番剧 / 用户）
- 搜索建议 / 热搜词
- 排序：综合 / 播放量 / 最新 / 时长
- 分页 + 无限滚动

### M4. 播放器（Player）
- 音频流播放（HTML5 `<audio>` 后端走 Rust 代理或直链）
- 视频模式（MV 全屏 / 画中画）
- 控件：播放/暂停、上一首/下一首、进度条、音量、循环模式、随机
- 媒体会话集成（macOS Now Playing / Windows SMTC）
- 可选：均衡器 / 音量标准化（Web Audio API）

### M5. 播放列表与持久化
- 创建 / 重命名 / 删除播放列表
- 添加 / 移除曲目、拖拽排序
- 队列（Queue）：插入下一首、添加到队尾
- 本地数据库（SQLite via rusqlite）+ 图片缓存（本地文件系统）

### M6. 插件系统（Plugin Runtime）
- 内置插件文件夹结构：`plugins/<id>/{plugin.json, index.js, ...}`
- 插件类型：`metadata-source` / `lyrics-source` / `engine` / `theme`
- 加载 / 卸载 / 启用 / 禁用 / 排序
- 设置 → 插件页：可视化展示、删除、刷新
- 沙箱：内置插件为编译期 Rust trait（无沙箱逃逸面）；第三方运行时插槽 `load_runtime_plugin` 预留，若未来开放则优先 WASM（extism/wasmtime）安全沙箱，不引入 V8

### M7. 设置 / 主题 / 国际化
- 布局：自适应 / 全屏单页 / 分栏（与 Spotube 对齐）
- 主题：跟随系统 / 浅色 / 深色
- 主色调：12 色板（slate / gray / zinc / neutral / stone / red / orange / yellow / green / blue / violet / rose）**（默认 slate，已确认）**
- 语言：i18n（默认 zh-CN / en-US，运行时切换）
- 字体：「Bilusic」品牌字 = Pacifico；UI 字 = 系统字体栈

### M8. 通用能力
- 下载（音频 mp3 / 视频 mp4，**仅供个人学习研究**）
- 媒体键 / 全局快捷键
- 错误日志 + 用户反馈入口
- 数据导入 / 导出（设置、播放列表）

---

## 2. 应用架构设计

### 2.1 总体架构

```
┌───────────────────────────────────────────────────────────────┐
│                 Frontend (Web - Tauri WebView)                │
│  React + TS + Vite · Tailwind · Zustand · i18next · WRouter   │
│  ┌─────────────┬──────────────┬─────────────┬──────────────┐ │
│  │  Pages /    │  Components  │ Player Core │   Plugin     │ │
│  │  Routes     │  (Impeccable │ (HTML5/     │   Manager    │ │
│  │             │   UI Kit)    │  Howler)    │   (UI)       │ │
│  └─────────────┴──────────────┴─────────────┴──────────────┘ │
└───────────────────────────────────────────────────────────────┘
            ▲                       IPC (Tauri commands)
            │                       events ↔
┌───────────┴───────────────────────────────────────────────────┐
│                  Backend (Rust - Tauri Core)                  │
│  ┌──────────┬──────────┬──────────┬──────────┬──────────────┐ │
│  │Coordinator│ Download │  Cache   │  Plugin  │  Settings /  │ │
│  │(编排)     │  Service │  Service │  Host    │  Database    │ │
│  ├──────────┴──────────┴──────────┴──────────┴──────────────┤ │
│  │ MetadataRegistry ◀──▶ EngineRegistry（按能力位标志注册）  │ │
│  └───────────────────────────────────────────────────────────┘ │
│   Plugins (内置 Rust trait；第三方运行时 = 预留插槽 load_runtime_plugin) │
│  ┌───────────────────┬─────────────────┬────────────────────┐ │
│  │ bilibili-metadata  │ qq-music        │ netease            │ │
│  │ (Rust metadata)    │ (metadata)      │ (metadata)         │ │
│  ├───────────────────┼─────────────────┼────────────────────┤ │
│  │ bilibili-engine    │ spotify-meta    │ lrclib-lyrics      │ │
│  │ (Rust audio)       │ (metadata)      │ (lyrics)           │ │
│  └───────────────────┴─────────────────┴────────────────────┘ │
└───────────────────────────────────────────────────────────────┘
```

### 2.2 目录结构

```
bilusic/
├── apps/
│   └── desktop/                      # Tauri 主项目
│       ├── src/                      # React 前端
│       │   ├── pages/                # 路由页面
│       │   ├── components/           # 通用 UI 组件
│       │   ├── features/             # 按业务切分的模块
│       │   │   ├── player/
│       │   │   ├── search/
│       │   │   ├── playlist/
│       │   │   └── settings/
│       │   ├── stores/               # Zustand 状态切片
│       │   ├── styles/               # Tailwind + Tokens
│       │   └── i18n/                 # 语言包
│       ├── src-tauri/                # Rust 后端
│       │   ├── src/
│       │   │   ├── commands.rs       # Tauri 命令入口
│       │   │   ├── metadata.rs       # MetadataSource trait + Registry
│       │   │   ├── engine.rs         # AudioEngine trait + Registry
│       │   │   ├── coordinator.rs    # Coordinator（编排，唯一对外入口）
│       │   │   ├── plugin.rs         # Plugin trait + PluginCapabilities 位标志
│       │   │   ├── wbi.rs            # B 站 WBI 签名
│       │   │   ├── proxy.rs          # 本地流代理（axum, :9527）
│       │   │   ├── plugins/          # ⭐ 内置 Rust 插件
│       │   │   │   ├── bilibili_metadata.rs
│       │   │   │   ├── bilibili_engine.rs
│       │   │   │   └── qq_music_metadata.rs
│       │   │   ├── download/
│       │   │   ├── plugin_host.rs    # P5 插件 host（清单发现/持久化/管理）；load_runtime_plugin 预留第三方运行时插槽
│       │   │   ├── plugins/          # ⭐ 内置 Rust 插件（编译期 trait 实现）
│       │   │   │   ├── bilibili_metadata.rs
│       │   │   │   ├── bilibili_engine.rs
│       │   │   │   ├── qq_music_metadata.rs
│       │   │   │   ├── lrclib.rs          # 完整歌词实现
│       │   │   │   └── netease.rs / kugou.rs / kuwo.rs / spotify.rs / ytmusic.rs  # 骨架
│       │   │   ├── db/               # rusqlite
│       │   │   └── settings/
│       │   └── Cargo.toml
│       ├── plugins/                  # ⭐ 插件清单目录（netease/kugou/kuwo/spotify/ytmusic/lrclib 的 plugin.json，供展示/启停/排序；第三方 JS/WASM 运行时插槽预留）
│       │   ├── lrclib/
│       │   │   ├── plugin.json
│       │   │   └── index.js          # 占位契约（当前未被原生实现使用）
│       │   ├── netease/  ├── kugou/  ├── kuwo/  ├── spotify/  ├── ytmusic/  （结构同上）
│       ├── package.json
│       └── tailwind.config.ts
├── docs/
│   ├── plan.md                       # 本文档
│   ├── plugin-spec.md                # 插件开发规范
│   └── architecture.md               # 架构详解
└── README.md
```

### 2.3 关键依赖选型

| 层 | 选型 | 说明 |
|--|--|--|
| 桌面壳 | **Tauri 2.x** | 用户指定 |
| 前端框架 | **React 18 + TypeScript + Vite** | 生态最熟，Impeccable 直接对接 |
| 样式 | **Tailwind CSS + CSS 变量（主题色板）** | 与 Spotube 一致 |
| 状态 | **Zustand** | 比 Redux 轻、比 Jotai 流行 |
| 路由 | **react-router v6** | 标准方案 |
| 国际化 | **i18next + react-i18next** | 与 Spotube 一致 |
| HTTP (Rust) | **reqwest** | 主流 |
| 视频流解析 | **rustyrewrite / bilibili-api-rs**（或自实现） | 解析 dash 分片 |
| 加密/编码 | `openssl`, `qmc` 解码（按需） | QQ 音乐插件用 |
| DB | **rusqlite + r2d2** | 本地持久化（✅ 已确认，手写 SQL） |
| 插件运行时 | 内置 **Rust trait**（编译期）；第三方运行时 = 预留插槽 `load_runtime_plugin`（未来若开放优先 WASM 而非 V8） | ✅ N9b 修订：不引入 deno_core/V8 |
| 包管理 | **pnpm** | ✅ 已确认；前端依赖与脚本 |
| 媒体键 | **tauri-plugin-media** 或自实现 | |

### 2.4 数据模型（核心）

```ts
// 跨前后端共享
interface Track {
  id: string;              // 源内唯一 id（"bili:av170001" 等）
  title: string;
  durationMs: number;
  artist: Artist[];
  album?: Album;
  coverUrl?: string;
  source: string;          // "bili-official"
  mediaType: "audio" | "video" | "auto";
  extra?: Record<string, unknown>;
}

interface Album { id: string; title: string; coverUrl?: string; artist: Artist[]; }
interface Artist { id: string; name: string; }

interface Playlist {
  id: string;
  name: string;
  description?: string;
  tracks: Track[];
  createdAt: number;
  updatedAt: number;
  isSmart?: boolean;       // 智能列表（按规则）
}
```

### 2.5 IPC 约定

- **命令（invoke，N5a 已落地）**：`list_plugins`, `set_active_metadata`, `set_active_engine`, `metadata_search`, `coordinator_play`, `play_by_bvid`, `set_bilibili_cookie`, `proxy_base_url`, `player.play`, `player.pause`, `player.seek`, `playlist.create`, `plugin.list`, `plugin.toggle`, `download.start`, `settings.get`, `settings.set`...
- **事件（listen）**：`player:tick`（进度）, `player:state`, `download:progress`, `plugin:loaded`...

每个命令走 Tauri `#[tauri::command]` + `Result<T, AppError>`，错误统一返回 `{ code, message, detail? }`。

---

## 3. 分阶段实现步骤（带优先级）

> 「优先级」采用 **MoSCoW**（Must / Should / Could / Won't-now）。每个阶段给出可验收产出。

### P0. 项目脚手架（M1 — Must）

**目标**：跑通一个空 Tauri + React 窗口、能设置主题/Pacifico 字样、能跨平台打包。

- [ ] Tauri 2.x 初始化 + React + TS + Vite
- [ ] Tailwind 接入，建立颜色 token（12 色板）、圆角、阴影、字号
- [ ] 字体：品牌字 Pacifico 通过 `@fontsource/pacifico` 引入；UI 字 = 系统栈
- [ ] 暗 / 亮模式切换（CSS variable strategy）
- [ ] 基础路由：`/`, `/search`, `/playlist/:id`, `/settings`
- [ ] 底部播放栏占位（无功能，UI 先就位）
- [ ] GitHub Actions：构建 macOS / Windows / Linux 三平台

**产出**：空壳可以跑起来，UI 框架与色板就位，CI 出包。

---

### P1. 音源引擎 + 播放核心（M2 + M4 — Must）✅ 已完成（N5a 架构纠偏后）

**目标**：能播一首 B 站视频的音频。

- [x] `MetadataSource` trait（`search` / `get_track` / `get_lyrics`）+ `MetadataRegistry`
- [x] `AudioEngine` trait（`find_stream_by_id` / `find_stream_by_query`）+ `EngineRegistry`
- [x] `Coordinator`：保存 `active_metadata` / `active_engine`，`search()`→元数据、`play()`→自动选引擎
- [x] `Plugin` trait + `PluginCapabilities` 位标志（`METADATA_*` / `AUDIO_*`）
- [x] 内置插件：`bilibili-metadata`（WBI 签名 + `view`）、`bilibili-engine`（DASH 优先、回退 `durl`，含 official/public 双后端）
- [x] Tauri 命令：`list_plugins` / `set_active_metadata` / `set_active_engine` / `metadata_search` / `coordinator_play` / `play_by_bvid` / `set_bilibili_cookie` / `proxy_base_url`
- [x] Rust 侧 `reqwest` + axum 本地流代理 `:9527`（转发 Referer/Range/Cookie，支持拖动进度 206），规避直链防盗链
- [x] 前端 Player Core：HTML5 `<audio>` 接 Rust 代理地址；Zustand `settings` 双字段 `metadataSource` / `audioEngine` + `biliCookie`，回灌协调器
- [x] 媒体会话（`MediaSession` 系统媒体键）
- [x] 错误日志（本地文件 + UI 反馈入口）

**产出**：开发者粘贴一个 bvid，前端能播放，按钮能暂停/继续、音量、进度；设置页可独立切换元数据源 / 音频引擎（双卡片）。

> ⚠️ 已知限制：B 站匿名 `search/type` 受 `v_voucher` 风控，匿名搜索返回空（已在前端提示，引导用官方引擎 Cookie 或 BV 号）；`view` / `playurl` 匿名正常。完整搜索列为 P2 风险项。

---

### P2. 搜索 UI + 结果列表（M3 — Must）✅ 已完成（N6）

**目标**：完整搜索闭环。

> **架构适配说明（N5b 之后）**：原计划按「B 站音源」写的「UP主 / 播放量」是 B 站概念；元数据改为音乐服务（QQ 音乐）后，对应字段改为「歌手 / 专辑」，播放量在 QQ legacy 端点不可得，故省略。其余条目均落地。

- [x] 搜索页：搜索栏、**热门建议**（预设标签）、**搜索历史**（localStorage 持久化、去重、可清除）
- [x] 结果列表组件：封面缩略图、标题、歌手、专辑、时长、播放按钮、**加入队列按钮**（调 `player.enqueue`）
- [x] 排序切换（默认 / 时长↑ / 时长↓，前端客户端排序）
- [x] 无限滚动（IntersectionObserver 哨兵 + 「加载更多」兜底，按页 `p` 追加，每页 20 条）
- [x] 结果行 hover 微动效（背景高亮 + 操作按钮淡入 + 播放键缩放）
- [x] 选中即播放（点击进入 P1 的 `resolveAndPlay` 路径）

**产出**：搜索「周杰伦」经 QQ 元数据返回 ≥ 20 条，点一下即经 B 站引擎收听；可无限下拉加载更多、可排序、可把歌曲加入队列。

---

### P3. 播放列表 + 持久化（M5 — Must）✅ 已完成（N7）

**目标**：列表循环、可管理的播放队列。

- [x] SQLite schema（Rust `rusqlite` bundled）：`playlists`, `playlist_tracks`, `tracks_cache`, `play_history`
- [x] 队列状态：当前曲目、上/下一首、循环模式（关/列表/单曲）、随机
- [x] 「加入播放列表」（行内菜单，含新建歌单）/「添加到队列」操作
- [x] 播放列表 CRUD + 重命名（点击标题行内编辑）+ 删除（确认）
- [x] 列表拖拽排序（**原生 HTML5 drag-and-drop，未引入 dnd-kit**，避免装包依赖）
- [x] 持久化：歌单/历史存 SQLite；播放队列 + 循环/随机/音量经 zustand `persist` 存盘，重启后重新解析当前曲（B 站签名 URL 会过期，不信任旧 URL）

**产出**：能建多个歌单、各塞歌曲、刷新/重启后还在；首页新增「最近播放」区块（来自 `play_history`）。

> 适配说明：原计划「列表拖拽排序（dnd-kit）」改为原生拖拽；「播放历史」除建表落库外，额外在首页以「最近播放」横向卡片展示，无需单独开页。

---

### P4. 设置中心 + 主题 + i18n（M7 — Should）✅ 已完成（N8）

**目标**：完整设置面板对标 Spotube 截图。

- [x] 设置分组：外观 / 播放 / 音源 / 插件 / 关于
- [x] 12 色板选择 + 实时预览
- [x] 主题：系统 / 浅 / 深（与 OS 同步 / 强制切换）
- [x] 布局类型：自适应 / 单页 / 分栏
- [x] i18n：zh-CN / en-US 完整词条（**P4 本阶段核心新增**：`src/i18n/{zh-CN,en-US,index}.ts` + 全量文案替换，`initialLocale()` 读 `localStorage` 首屏定语言零闪烁）
- [x] 「元数据源 / 音频引擎」双卡片选择器（设置 → 音源，对应 Spotube 引擎选项）
- [x] 缓存开关、缓存清理

> 落地说明：外观/主题/12 色板于 P0 落地；音源双卡片选择器于 N5a 落地；缓存清理（孤儿清理/刷新/压缩/退出 VACUUM）于 N7b–N7d 落地；**本阶段（P4/N8）补齐的是 i18n 基础设施与全量文案替换**，使设置页与全部页面文本可中英切换。校验：`tsc --noEmit` 0 error、`vite build` 成功。

**产出**：截图里那几张设置页一对一实现，色板 / 主题 / 引擎切换 + 中英语言切换全部生效。

---

### P5. 插件系统（M6 — Should）✅ 已完成（运行时无关基础 + P5-II 原生优先；第三方运行时 = 预留插槽）

**目标**：插件可装可卸，独立文件夹管理。

- [x] 插件规范文档（`docs/plugin-spec.md`，§8 已补实际 host 实现 + §8.6 待办）
- [x] **动态插件注册表**（运行时无关）：注册表键改 `String`，`PluginHost` 持久化启用/排序/卸载到 `app_data_dir/plugin_state.json`
- [x] **插件 host**：扫描 3 个发现根加载 `plugin.json`（用户目录 / 打包资源 / `cwd/plugins`），按 `type:id` 去重
- [x] 第三方插件目录（`apps/desktop/plugins/`）：`netease` / `kugou` / `kuwo`（metadata）、`spotify`（metadata·需 OAuth）、`ytmusic`（metadata）、`lrclib`（lyrics）+ 根 `README.md`
- [x] 设置 → 插件页：列表、启用/禁用、卸载（删除文件夹·二次确认）、排序（上移/下移），统一 Toast 反馈
- [x] 插件元数据合并规则：`Coordinator.init_defaults()` 从 host 读取启用状态决定默认 `active_metadata` / `active_engine`
- [x] **P5-II 原生优先（已完成，N9b）**：6 首发音源改为编译期 Rust trait 实现——`lrclib` 完整歌词（无鉴权公开 REST + LRC 解析）、`netease`/`kugou`/`kuwo`/`spotify`/`ytmusic` 骨架（真实 `reqwest` 客户端 + 目标能力位 + 明确 TODO endpoint），默认关闭；`PluginHost::load_runtime_plugin` 预留第三方运行时插槽（**不引入 deno_core/V8**）；歌词透传 `get_lyrics(track)` + 命令 `metadata_get_lyrics`。
- [ ] **第三方运行时（可选，未排期）**：若未来要开放终端用户装第三方插件，经 `PluginHost::load_runtime_plugin` 插入点加载 JS/WASM（优先 WASM/extism，而非 V8/deno_core），产出 `Arc<dyn ...>` 与原生源对称共存。当前原生 6 源已覆盖需求，此步非必须。

**产出（当前）**：设置页可可视化启停/排序/卸载第三方清单；禁用 QQ 音乐后元数据只剩 B 站（及任何已启用的第三方）；JS 执行接入后第三方转为真实可用音源/引擎。

---

### P6. 下载功能（M8 — Could）

**目标**：个人学习研究用途的本地下载。

- [ ] 「下载」按钮（曲目菜单 / 播放栏菜单）
- [ ] 格式：mp4 视频、mp3 音频（ffmpeg 转码）
- [ ] 下载任务队列 + 进度条 + 并发控制
- [ ] 下载目录设置、断点续传
- [ ] 仅个人使用的免责声明弹窗（首次启用时）
- [ ] 存储路径：`~/Downloads/Bilusic/`

**产出**：能下视频、能下音频、能中断、能续传。

---

### P7. 打磨 + 高级能力（M4 / M7 — Could）

- [ ] 歌词面板（左侧抽屉，启用歌词插件时显示）
- [ ] 均衡器（Web Audio API）— ⏸ 已明确不在 MVP 范围，后续再议
- [ ] 媒体键系统级接管
- [ ] 任务栏 / Dock 浮窗
- [ ] 导入 / 导出（json / m3u）
- [ ] 失败重试 + 网络代理设置
- [ ] 「去 AI 化」质感精修：阴影层级、触感反馈、字距

---

### P8. 测试 + 发布（Must — 持续）

- [ ] 单元测试：Rust 核心 / TS 工具 / 插件协议
- [ ] 集成测试：搜索 → 播放 → 加列表 全链路
- [ ] E2E：Playwright + Tauri webdriver
- [ ] 发布：`tauri build` 多平台 + 自动更新 + 签名
- [ ] 用户文档 + 插件开发文档

---

### 优先级总览

| 阶段 | MoSCoW | 目标 | 工时估时（参考） |
|--|--|--|--|
| P0 脚手架 | Must | 跑通 | 3–5 天 |
| P1 引擎 + 播放 | Must | 能播 | 7–10 天 |
| P2 搜索 UI | Must | 能搜 | 4–5 天 |
| P3 列表 + 持久化 | Must | 能列 | 5–7 天 |
| P4 设置 + 主题 + i18n | Should | 对齐截图 | 5–7 天 |
| P5 插件系统 | Should | 6 个内置插件 | 7–10 天 |
| P6 下载 | Could | 能下 | 4–5 天 |
| P7 打磨 | Could | 体验分 | 持续 |
| P8 测试 + 发布 | Must | 持续 | 持续 |

> 总估时：**MVP (P0–P4) 约 4 周**，完整版含 P5–P6 约 6–7 周。

---

## 4. 风险与对策

| 风险 | 影响 | 对策 |
|--|--|--|
| B 站风控 / WBI 签名变更 | 搜索/播放失败 | 抽象 `MetadataSource` / `AudioEngine`，单点替换；预留代理后端；`v_voucher` 风控已有降级提示 |
| 直链盗链导致前端 fetch 失败 | 播放中断 | Rust 侧做流代理 |
| 插件沙箱逃逸 | 安全隐患 | 本阶段内置插件为 Rust trait（无 JS 沙箱逃逸面）；若未来接入第三方运行时，用 WASM sandbox（extism/wasmtime）并禁用危险宿主 op，不引入 V8 |
| 去 AI 化质感难把握 | UI 同质化 | 引入 Impeccable 设计系统 + GSAP 动效；有「音频播放器」参考集 |
| 跨平台音频会话不一致 | macOS/Win 体验差异 | 抽象媒体会话 trait，按平台分发 |
| 元数据插件反爬 | 插件失败 | 失败静默 + 退到源端默认元数据 |

---

## 5. 验收标准（拟定）

> 评审通过后再补全。

### MVP 验收（P0–P4 完成后）

1. 启动 App，能看到带 Pacifico 字样的「Bilusic」标题。
2. 在设置里切换 12 种主色调，整个 UI 实时换色。
3. 搜索「周杰伦 MV」，列表渲染 ≥ 20 条结果。
4. 任意点一首能播放，进度条、音量、暂停/继续正常。
5. 创建播放列表，添加 5 首歌曲，关闭 App 再开仍然存在。
6. macOS / Windows / Linux 三平台分别能跑出 `.dmg / .msi / .AppImage`。
7. 切换中英文，文本全量翻译生效。

### 完整版验收（追加）

8. 禁用「QQ 音乐」插件，专辑封面降级为 B 站封面；启用后恢复。
9. 下载一首为 mp3 / mp4，进度条正常，磁盘可见。
10. 歌词面板可显示（启用 lrclib 后）。

---

## 6. 决策记录（已确认 ✅）

用户于 2026-08-18 确认以下 7 项，计划据此定稿，准备进入 P0：

| # | 决策点 | 结论 |
|--|--|--|
| 1 | 引擎抽象 | ✅ 内置 `bilibili-official` + `bilibili-public` 双引擎，预留插件化扩展 |
| 2 | 插件运行时 | ✅ 原定 deno_core（V8）；**N9b 修订**：内置 Rust trait 优先，不引入 deno_core/V8，第三方运行时 = 预留插槽 `load_runtime_plugin`（未来若开放优先 WASM） |
| 3 | 数据库 | ✅ **rusqlite + r2d2**（手写 SQL，同步风格） |
| 4 | 包管理 | ✅ **pnpm** |
| 5 | MVP 范围 | ✅ P0–P4 = MVP；**均衡器明确不纳入 MVP**（移至 P7 并标记延期） |
| 6 | 默认主色调 | ✅ **slate** |
| 7 | 打包目标 | ✅ **macOS / Windows / Linux 三平台同步** |

**架构纠偏记录（2026-08-18，N5a）**：P1 首版将「元数据 + 音频流」焊死在单一 `BiliSource` 抽象中，经复核与 Spotube 模型不符。已重构为 **`MetadataSource`（元数据）与 `AudioEngine`（音频流）两个正交 trait + `Coordinator` 协调器** 的范式，B 站代码下沉为 `plugins/bilibili_metadata.rs` 与 `plugins/bilibili_engine.rs`。本计划 v1.2 已据此同步。原计划「引擎插件化」保留，但明确：**内置插件用 Rust trait 实现（MVP 优先）**；第三方运行时的插槽（`load_runtime_plugin`）在 P5 预留，N9b 决策**不引入 deno_core/V8**，真要开放第三方运行时则 WASM 是比 V8 更轻的安全替代。

---

## 7. 下一步行动

> **当前进度**：P0–P4、P5（插件系统运行时无关基础，N9）、**P5-II（原生优先，N9b）** 均已完成。`docs/PROGRESS.md` 已记录 N0–N9b 节点，与代码三者一致。P5-II 的「第三方 JS/WASM 运行时」按决策降级为**可选预留插槽**（非必须）。
>
> 下一步：**P6 下载功能（M8）**：复用本地流代理（`:9527`）把音频字节落盘到用户目录（按歌单/艺人分文件夹），含队列/并发/进度/已完成列表、元数据随文件写入、设置页「下载路径」配置。待你发出指令后推进。
