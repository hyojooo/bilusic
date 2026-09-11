# Bilusic 插件模板（开发者参考 · 不默认发现）

> 本目录存**插件开发模板 / 教学示例**，**不是默认发现的清单**。`PluginHost::init` 只扫描 `apps/desktop/plugins/`，不会扫 `plugins-examples/`，所以这里的内容不会出现在设置页「插件」Tab 里，避免「占位/示例」噪音污染真实插件列表。

## 三个模板

| 子目录 | runtime | 说明 |
|---|---|---|
| `declarative/` | `declarative` | 最小声明式插件（纯 JSON 配置）：base_url / endpoints / 字段映射。仿 `declarative/plugin.json` 改字段即可接入新的 REST 音源，无需任何 Rust/WASM/JS。 |
| `wasm/` | `wasm` | wasm guest crate 模板：`guest/` 是一个独立 Rust crate（`[lib] name="plugin"`），被宿主 `wasmtime` 加载后通过 `bilusic.host_http_get` / `bilusic.host_log` 等 host functions 调用宿主能力。适合需要 XOR 解密、protobuf、自定义签名算法的音源。 |
| `js/` | `js` | JS 插件模板：单 `plugin.js` 导出 `main(inputJson) → string`，宿主用 rquickjs（QuickJS 引擎，默认开启，feature `js` 在 default 中）执行。适合不愿学 Rust/WASM 的胶水场景。 |

## 怎么把模板用起来

三种方式都可以：

1. **复制到默认发现路径**（推荐用于本地迭代）：
   ```bash
   cp -R apps/desktop/plugins-examples/declarative apps/desktop/plugins/my-source
   # 然后改 my-source/plugin.json 的 id / name / declarative 块
   pnpm tauri dev
   ```
   改完 `pnpm tauri dev` 自动重扫，出现在设置页「插件」列表。

2. **从设置页「导入插件」导入**：把模板目录（或解压后的子目录）拖进 macOS 文件选择器 / 选 plugin.json → 一键导入到 `app_data_dir/plugins/<id>/`，下次启动生效。

3. **从外部仓库 / 第三方来源**导入：任何符合 `plugin.json` schema 的目录都能用同样方式导入，runtime ∈ `declarative`/`wasm`/`js` 都能被宿主加载。

## 字段表与契约

完整字段说明见 `apps/desktop/plugins/README.md`（插件目录层的总览）。每个模板子目录也有自己的 README.md：

- `declarative/README.md`（如有）—— declarative 字段映射详解
- `wasm/README.md` —— wasm guest 契约 / host functions 列表 / build 教程
- `js/README.md` —— JS plugin contract（`main(inputJson) → string`）

## 为什么从 `plugins/` 挪到 `plugins-examples/`

设置页插件列表的「默认发现」只用于**真实可用**的音源（当前只有 `itunes-declarative`）。开发者模板留在这里随时查阅，需要哪个就 cp 出去，不会污染用户视角的「默认就有」清单。
