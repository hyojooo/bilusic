# example-wasm · B 线 WASM 逃生口示例

本目录演示 **P5-II 的 B 线**：当「声明式配置（A 线）」表达不了的复杂音源（非 REST 协议、自定义签名、二进制负载等）需要接入时，作者用**任意语言**编译出 `wasm32-unknown-unknown` 模块，宿主经 host functions 提供 HTTP/日志能力，适配器把 trait 调用转发到 wasm 导出函数，与原生 Rust 源在 Registry 对称共存。

> 该运行时需以 `cargo check --features wasm` / `tauri dev --features wasm` 编译启用。未开启时 `load_runtime_plugin` 会返回「需以 --features wasm 重新编译」。

## 1. 清单（plugin.json）

```json
{
  "id": "example-wasm",
  "name": "Example WASM Source",
  "version": "0.1.0",
  "type": "engine",
  "description": "...",
  "entry": "plugin.wasm",
  "runtime": "wasm",
  "capabilities": ["AUDIO_BY_ID"],
  "requires_auth": false
}
```

- `runtime` 必须为 `"wasm"`。
- `entry` 为相对本目录的 `.wasm` 文件名。
- `type: "engine"` 或 `capabilities` 含 `AUDIO_BY_ID` → 宿主额外构建 `WasmAudioEngine` 适配器。

## 2. Guest 契约（ABI）

### 必须导出的函数

模块需导出线性内存 `memory` 与 bump 分配器 `alloc`：

| 导出 | 签名 |
|------|------|
| `alloc` | `(size: i32) -> i32` 返回指针 |

每个 trait 方法对应一个导出，参数形式为 `(in_ptr, in_len, …, out_ptr, out_len)`：

| 导出 | 输入参数（顺序） | 语义 |
|------|----------------|------|
| `metadata_search` | `qptr, qlen, page_ptr, page_len` | 搜索 |
| `metadata_get_track` | `idptr, idlen` | 取单曲元数据 |
| `metadata_get_lyrics` | `idptr, idlen, titleptr, titlelen, artistptr, artistlen` | 取歌词（LRC 文本） |
| `audio_find_stream_by_id` | `idptr, idlen` | 取播放流 |

`out_ptr` / `out_len` 是两个 `i32` 暂存槽的地址（宿主用 `alloc(8)` 分配并清零），guest 把结果缓冲的「地址 + 长度」写回这两个槽。函数返回 `0` 表示成功，非 `0` 表示错误。

### 必须导入的 host functions（模块名 `bilusic`）

| 导入 | 签名 | 说明 |
|------|------|------|
| `host_http_get` | `(urlptr: i32, urllen: i32, out_ptr: i32, out_len: i32) -> i32` | 发起 HTTP GET，把响应体写入 guest 分配缓冲并填 `out_ptr/out_len`，返回 `0` 成功 |
| `host_log` | `(msgptr: i32, msglen: i32) -> i32` | 打印调试日志到 stderr，返回值被忽略 |

> wasmtime 47 移除了 `Trap::new(&str)`，故 host functions 一律返回**状态码**（0=ok），避免构造 Trap。

### JSON 数据结构

- 搜索结果：`track` 对象数组，或 `{"results":[…]}`。
- 单曲元数据：单个 `track` 对象。
- 歌词：JSON 字符串（LRC 文本，宿主按 `[mm:ss.xx]` 解析成逐行）。
- 流：单个 `stream` 对象。

```jsonc
// track 对象
{
  "source_id": "string",
  "title": "string",
  "artist": "string",
  "album": "string",
  "duration_ms": 0,
  "cover": "string",   // 封面 URL
  "lyrics_id": "string" // 可选
}
// stream 对象
{
  "url": "string",
  "mime": "audio/mpeg", // 可选，默认 audio/mpeg
  "bitrate": 320,
  "duration_ms": 0,
  "cover": "string",
  "title": "string",
  "artist": "string"
}
```

## 3. 真实 guest 示例（iTunes，子目录 `guest/`）

本目录附带一个**可编译、可端到端验证**的真实 guest：`guest/` 是一个独立 crate，包装公开的
[iTunes Search / Lookup API](https://affiliate.itunes.apple.com/resources/documentation/itunes-store-web-service-search-api/)（**无需鉴权**）。它用 `std` + `serde_json`（wasm32 目标下 `std` 自带默认全局分配器并导出 `memory`，无需 `#[no_std]` 或自定义分配器），编译产物即 `plugin.wasm`，与 `plugin.json` 的 `entry` 对齐。

关键片段：

```rust
// 从模块 "bilusic" 导入宿主函数（必须用 link 属性指定 import module 名）
#[link(wasm_import_module = "bilusic")]
extern "C" {
    fn host_http_get(url_ptr: i32, url_len: i32, out_ptr_slot: i32, out_len_slot: i32) -> i32;
    fn host_log(msg_ptr: i32, msg_len: i32) -> i32;
}

// 导出：bump 分配器（host 经它写入输入、我们也经它分配输出缓冲）
#[no_mangle]
pub extern "C" fn alloc(size: i32) -> i32 { /* std::alloc::alloc */ }

// 导出：搜索 → 调 host_http_get(itunes search) → 映射为 track 数组 JSON
#[no_mangle]
pub extern "C" fn metadata_search(qptr: i32, qlen: i32, _pageptr: i32, _pagelen: i32,
                                  out_ptr_slot: i32, out_len_slot: i32) -> i32 {
    let query = String::from_utf8_lossy(&load_bytes(qptr, qlen)).to_string();
    let url = format!("https://itunes.apple.com/search?term={}&media=music&limit=20",
                      urlencode(&query));
    let body = match http_get(&url) { Some(b) => b, None => return -1 };
    let v: Value = serde_json::from_slice(&body).map_err(|_| return -1).unwrap();
    let arr: Vec<Value> = v.get("results").and_then(|x| x.as_array())
        .map(|r| r.iter().map(track_to_json).collect()).unwrap_or_default();
    let out = serde_json::to_vec(&arr).unwrap_or_default();
    emit(out_ptr_slot, out_len_slot, &out)   // 写回 (ptr,len) 到 scratch 槽
}
// metadata_get_track / metadata_get_lyrics / audio_find_stream_by_id 同理
```

> `metadata_get_lyrics` 在 guest 内**直接返回空 LRC 字符串、不发任何 HTTP**，因此可作为完全离线的握手测试。

编译（产物名由 `[lib] name = "plugin"` 控制，恰好就是 `plugin.wasm`）：

```bash
rustup target add wasm32-unknown-unknown
cd guest && ./build.sh        # = cargo build --target wasm32-unknown-unknown
# 产物：guest/target/wasm32-unknown-unknown/debug/plugin.wasm
```

## 4. 宿主侧适配器（src/wasm_runtime.rs）

以 `wasmtime/async` feature 编译（整引擎进入 async 模式）：

- `WasmInstance::load`（同步）：读 `.wasm` → 建 `Config` 并 `with_host_stack` → `Engine::new` → `Module::new` → `Linker` 注册 host functions（模块名 `bilusic`）→ `Store::new`。**不在此实例化**，实例化推迟到每次 `invoke`。
- host functions：
  - `host_http_get` 经 `func_wrap_async` 注册——闭包内 `reqwest::Client` 发起 **真正非阻塞** 的 HTTP GET（`await` 把 tokio worker 让出，不阻塞线程），响应体写入 guest 分配缓冲并填 `out_ptr/out_len`。
  - `host_log` 经 `func_wrap` 注册（仅打印，同步即可）。
- `WasmInstance::invoke`（**`async fn`**）：`store` 用 `tokio::sync::Mutex` 保护；每次调用 `instantiate_async` → 把输入 `alloc`+写入内存 → 追加 8 字节 scratch → `call_async` 转发导出 → 读回 `out_ptr/out_len` 解析 JSON。
- `WasmMetadataSource` / `WasmAudioEngine`：分别实现 `MetadataSource` / `AudioEngine`（trait 方法本就是 `async fn`），内部 `.await` 调 `invoke`。**不波及任何原生 Rust 插件**（qq/bilibili/netease 等）。
- `build_wasm_plugin(manifest)`：被 `PluginHost::load_runtime_plugin` 在 `runtime == "wasm"` 且开启 `wasm` feature 时调用（同步；`reqwest::Client::new()`）。
- 集成测试（`#[cfg(test)] mod tests`，`#[tokio::test]`）：`load_guest_and_offline_lyrics`（离线，必跑）+ `online_search_smoke`（在线，需 `-- --ignored`）。

## 5. 调试

- guest 内用 `host_log` 打印；宿主侧以 `[bilusic-wasm] …` 输出到 stderr。
- 任何 host function 失败返回 `-1`，导出函数非 0 返回 → 宿主转为 `AppError::msg(…)` 透出。

## 6. 验证状态

- ✅ 离线 ABI 往返：`cargo test --features wasm load_guest_and_offline_lyrics` 通过——host 加载真实 `plugin.wasm`、实例化、经完整内存交换契约调 `metadata_get_lyrics` 并解析 JSON。
- ✅ 真实 guest 可编译：iTunes guest 在 `wasm32-unknown-unknown` 下 `cargo build` 通过，产物 `plugin.wasm`。
- ✅ 在线链路：`cargo test --features wasm -- --ignored online_search_smoke` 走真实 iTunes HTTP 通过——返回 20 条 track（如 `One More Time — Daft Punk`），证明 host HTTP（非阻塞）→ guest 映射 → host 解析整条链路打通。
- ✅ host HTTP 已改为**异步非阻塞**（`func_wrap_async` + `reqwest::Client`，引擎开启 `with_host_stack`），HTTP 等待期间让出 tokio worker 而非阻塞线程。

