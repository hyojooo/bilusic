# Bilusic 插件目录

本目录存放**第三方插件清单**（`plugin.json`）。每个子目录对应一个插件，宿主（`PluginHost`）在启动时扫描并合并到设置页「插件」管理中。

## 加载与状态

> `plugin_host` 已能**发现 / 展示 / 启用 / 禁用 / 排序 / 卸载**这些清单。`runtime: declarative`
> 与 `runtime: wasm` 走通：declarative 由内置 `DeclarativeMetadataSource` / `DeclarativeAudioEngine`
> 按字段映射发 HTTP；wasm 由 `wasmtime` 加载 guest crate 并通过 host functions
> (`bilusic.host_http_get` 等) 转发 trait 调用。runtime 字段值为 JS / Python / 其他未识别值
> 的清单会落进「占位」徽章行，仅展示、不可用。

## 目录结构

```
plugins/
├── itunes-declarative/  plugin.json + README.md   iTunes Search 默认音源（真实可用，最佳仿写模板）
├── kuwo/                plugin.json               酷我音乐（metadata，已接入，真实可用，仿写模板）
├── netease/             plugin.json               网易云音乐（metadata，待接入）
├── kugou/               plugin.json               酷狗音乐（metadata，待接入）
├── spotify/             plugin.json               Spotify（metadata，需 OAuth）
├── ytmusic/             plugin.json               YouTube Music（metadata，待接入）
└── lrclib/              plugin.json               LrcLib（lyrics）
```

> **`itunes-declarative`（真实可用）是任何 REST 音源仿写的最佳起点**——开箱即搜，导出 80 余
> 首结果。开发者模板（declarative / wasm / js 最小示例）已移出 `plugins/` 到
> `apps/desktop/plugins-examples/`，避免污染用户视角的「默认就有」清单；想试 declarative /
> wasm / js 写自己音源，参考那里按目录复制到 `plugins/<id>/` 或从设置页「导入插件」导入。
> 仿写指南见 `apps/desktop/plugins/itunes-declarative/README.md`。

## plugin.json 字段

| 字段            | 说明                                                                            |
| --------------- | ------------------------------------------------------------------------------- |
| `id`            | 插件唯一 id（与 `type` 组成复合键 `type:id`）                                   |
| `name`          | 展示名                                                                          |
| `version`       | 语义化版本（默认 `0.0.0`）                                                      |
| `type`          | `metadata` / `engine` / `lyrics` —— 决定插件在架构中的角色                      |
| `runtime`       | `declarative` / `wasm` / 未识别值（=占位）；缺省 = 占位                         |
| `description`   | 简介（设置页展示）                                                              |
| `entry`         | wasm 运行时：guest crate 名；其他：留空                                         |
| `capabilities`  | 能力位名数组，见下表                                                            |
| `requires_auth` | 是否需要用户授权（如 Spotify OAuth）                                            |
| `declarative`   | `runtime: declarative` 时必填：base_url / headers / search.{path,list_path,map} |
| `wasm`          | `runtime: wasm` 时必填：`{ entry }` 指向 guest crate 名                         |

### capabilities 能力名

`METADATA_SEARCH` · `METADATA_GET` · `METADATA_LYRICS` · `METADATA_HOME`
`AUDIO_BY_ID` · `AUDIO_BY_QUERY` · `AUDIO_NEEDS_AUTH`

## runtime 详解

### `runtime: "declarative"` — 零代码、纯配置

宿主内置 `DeclarativeMetadataSource` / `DeclarativeAudioEngine` 按 `declarative` 块发 HTTP
并把响应按 JSONPath 映射成 `MetadataTrack` / `StreamInfo`。详见
`apps/desktop/src-tauri/src/declarative.rs`。**不需要任何 plugin 文件夹里的代码**。

仿写时复制 `itunes-declarative/plugin.json` 改字段即可。

### `runtime: "wasm"` — wasmtime guest crate

插件自带一个 `[lib] crate`，通过 host functions 调用宿主能力：

| host 函数                                  | 用途                                     |
| ------------------------------------------ | ---------------------------------------- |
| `bilusic.host_http_get(url, headers_json)` | 异步发 HTTP GET，返回 status code + body |
| `bilusic.host_log(level, msg)`             | 写日志                                   |
| `bilusic.host_*`                           | 持续扩展中                               |

完整契约见 `apps/desktop/plugins-examples/wasm/README.md`。

### runtime 字段未识别 / 缺省 → 占位

清单仍被发现并展示，但 `stub: true`，不可用作音源。常见原因：

- `runtime` 字段缺失 / 未在宿主识别列表中（如 `"nodejs"`、`"python"`、`"lua"`）
- `declarative` / `wasm` 必填字段缺失或语法错误

## 用户自定义插件

两种途径：

1. **导入**：把插件文件夹（或解压后的目录）放入 `<app_data_dir>/plugins/<id>/`，
   macOS 路径 `~/Library/Application Support/com.bilusic.app/plugins/`。
   也可以在 设置 → 插件 → 右上「**导入文件夹**」/「**导入 plugin.json**」按钮处一键导入。
2. **默认发现**：bundled 真实音源在 `apps/desktop/plugins/<id>/` 下，dev 模式扫 cwd 的 `plugins/`，
   生产模式扫 `resource_dir/plugins/`（即随安装包 shipped）。开发者模板（declarative / wasm / js）
   不再 bundled，移到了 `apps/desktop/plugins-examples/`，不在默认发现列表里。

默认发现的插件**可以被卸载**：卸载后从 `app_data_dir/plugins/<id>/` 删除，不会影响下次启动
的 bundled discovery。同名 id 冲突时，**用户副本 wins**（bundled 是退路）。
