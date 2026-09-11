# itunes-declarative — bundled 默认音源示例

这是一个**默认随 app 一起 shipped** 的声明式音源插件示例。它的目的是：

1. 让任何用户打开 app 都能在 设置 → 插件 → 元数据 看到一个真实可用、可点开的元数据源（点搜索就能出歌）
2. 作为「如何仿写任意 REST 音源」的**模板**：仿写网易云、酷我、QQ 音乐对外接口或你的私有接口时，把 `plugin.json` 里的 `base_url` / `search.path` / `map` 字段换成目标服务的对应值即可

## 它能做什么

- 提供 **元数据搜索** 能力（`capabilities: ["METADATA_SEARCH"]`）
- 调用 iTunes Search 公开 API（`https://itunes.apple.com/search`），**无需鉴权 / 无需 API key**
- 返回的 `trackId` 同时作为 `source_id` 和 `lyrics_id`，所以搜索结果可直接进播放流（音频由内置的 B 站引擎兜底——参见 `engine_hint="bilibili"` 链路的 `Coordinator`）

## 它「不是」什么

- **不是音频引擎**。音频字节由内置的 B 站引擎（`bilibili-engine`）通过标题+艺人反查提供，见 `apps/desktop/src-tauri/src/coordinator.rs` 的「跨源播放」路径
- **不是 ws/wasm 插件**。它是 `runtime: "declarative"`——纯字段映射 + 内置 HTTP，**零运行时**、**零代码**

## 如何仿写

下面是把本插件改写成「任意 REST 音源」的最小改动表（以你自己的私有 API 为例）：

| 字段 | 当前 (iTunes) | 改写 (目标) |
|---|---|---|
| `declarative.base_url` | `https://itunes.apple.com` | `https://api.your-service.com` |
| `declarative.headers` | `{ "Accept": "application/json" }` | 加你的鉴权头：`{ "Accept": "application/json", "Authorization": "Bearer ${API_KEY}" }` |
| `declarative.search.path` | `/search?term=${query}&media=music&limit=20` | `/v1/search?q=${query}&count=20` |
| `declarative.search.list_path` | `$.results` | `$.data.items`（JSONPath） |
| `declarative.search.map.source_id` | `$.trackId` | `$.id` |
| `declarative.search.map.title` | `$.trackName` | `$.name` |
| `declarative.search.map.artist` | `$.artistName` | `$.artists[0].name` |
| ... | ... | ... |

字段映射说明（详见 `apps/desktop/src-tauri/src/declarative.rs`）：

- `${query}` 会被替换成 URL-encoded 的搜索词
- `path` / `list_path` / `map.xxx` 全部用 JSONPath 解析 iTunes 的 JSON 响应
- `duration_ms` 不填也行——播放流由 `engine_hint` 决定，本字段主要给 UI 显示用

## 调试 / 查看真实响应

```bash
curl 'https://itunes.apple.com/search?term=Daft+Punk&media=music&limit=2' | jq
```

确认 JSON 路径正确后再填进 `plugin.json`。

## 想编辑完后试试？

最快路径：

```bash
# 编辑 apps/desktop/plugins/itunes-declarative/plugin.json
# 重启 dev mode，dev 自动重新加载 manifests
pnpm --filter @bilusic/desktop tauri dev
```

## 想发布给别人用？

把这个目录打包成 zip（或 tarball），对方在 设置 → 插件 → 右上「导入文件夹」选你的压缩包解压后的目录即可。详见 `apps/desktop/plugins/README.md` 的「发布与导入」段。

## 比本插件更复杂的场景

如果目标 API 需要：

- **多步鉴权**（OAuth 2 跳、cookie 共享）→ 用 `runtime: "wasm"` 自行在 guest crate 调用 `bilusic.host_http_*` 等 host 函数
- **复杂字段变换**（如清洗富文本、合并字段、调用 JWT 解码）→ 用 `runtime: "wasm"`，declarative 表达不了

参考 `apps/desktop/plugins/example-wasm/`。
