//! B · WASM runtime escape hatch (feature = `wasm`).
//!
//! For audio sources that a *declarative* config can't express (non-REST
//! protocols, bespoke signing, binary payloads), a third-party author compiles
//! a `wasm32-unknown-unknown` module against the Bilusic guest contract and
//! ships it as `plugins/<id>/<entry>.wasm`. The host instantiates it with
//! wasmtime, exposes a tiny set of **host functions** (HTTP GET + logging), and
//! provides [`WasmMetadataSource`] / [`WasmAudioEngine`] adapters that forward
//! the `MetadataSource` / `AudioEngine` trait calls into the module's exports.
//! The result is symmetric with the built-in Rust sources in the registries.
//!
//! ## Async mode
//!
//! This module is compiled with the `wasmtime/async` feature: the wasm engine
//! runs in async mode, host functions are registered via `func_wrap_async`, and
//! the HTTP host function performs a **truly non-blocking** `reqwest` request
//! (`await` yields the tokio worker instead of blocking a thread). The
//! [`WasmInstance::invoke`] method is `async` and is `.await`-ed from inside the
//! already-`async` `MetadataSource` / `AudioEngine` trait methods, so the whole
//! call chain cooperatively yields on the app's tokio runtime. The synchronous
//! [`WasmInstance::load`] (called once at plugin-discovery time from
//! `load_runtime_plugin`) builds the module/linker/store without issuing any
//! async call.
//!
//! ## Guest contract
//!
//! Required exports:
//! - `memory` — exported linear memory.
//! - `alloc(size: i32) -> i32` — bump/heap allocator returning a pointer.
//! - `metadata_search(qptr, qlen, outp, outl) -> i32`
//! - `metadata_get_track(idptr, idlen, outp, outl) -> i32`
//! - `metadata_get_lyrics(idptr, idlen, titleptr, titlelen, artistptr, artistlen, outp, outl) -> i32`
//! - `audio_find_stream_by_id(idptr, idlen, outp, outl) -> i32`
//!
//! Each trait call packs its string inputs into guest memory (via `alloc`),
//! passes `(in_ptr, in_len, … , out_ptr, out_len)` where `out_ptr`/`out_len`
//! are pointers to two `i32` scratch slots the guest fills with the result
//! buffer address + length. The guest returns `0` on success, non-zero on
//! error. The host reads the result and parses it as JSON:
//! - `metadata_search` → JSON array of track objects (or `{"results":[…]}`).
//! - `metadata_get_track` → single track object.
//! - `metadata_get_lyrics` → JSON string (the LRC text).
//! - `audio_find_stream_by_id` → single stream object.
//!
//! Track object: `{source_id,title,artist,album,duration_ms,cover,lyrics_id}`.
//! Stream object: `{url,mime,bitrate,duration_ms,cover,title,artist}`.
//!
//! Required imports (host functions), module name `bilusic`:
//! - `host_http_get(urlptr: i32, urllen: i32, outp: i32, outl: i32) -> i32` —
//!   performs an **async** HTTP GET and writes the response body into a
//!   guest-allocated buffer, filling `outp`/`outl`. Returns `0` on success,
//!   non-zero on error.
//! - `host_log(msgptr: i32, msglen: i32) -> i32` — prints a debug line to
//!   stderr. (The return value is ignored; `i32` is kept for uniformity.)
//!
//! A reference guest SDK (`bilusic-wasm-sdk`) and an example plugin live under
//! `apps/desktop/plugins-examples/wasm/`.

use crate::engine::{AudioEngine, StreamInfo};
use crate::error::AppError;
use crate::metadata::{Lyrics, MetadataSource, MetadataTrack};
use crate::plugin::{Plugin, PluginCapabilities, PluginInfo};
use crate::plugin_host::{capability_bits, PluginManifest};
use async_trait::async_trait;
use serde_json::{Value, from_slice};
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;
use wasmtime::{
    AsContext, AsContextMut, Caller, Config, Engine, Extern, Func, Instance, Linker, Memory, Module,
    Store, Val,
};

/// Host state threaded through every wasm instance (shared by all host fns).
struct WasmState {
    client: reqwest::Client,
}

/// A loaded wasm plugin, shared by its metadata/audio adapters.
///
/// The module + linker are built once in [`WasmInstance::load`]; each
/// [`WasmInstance::invoke`] re-instantiates (cheap for our stateless
/// request/response model) and dispatches an export. The store is behind an
/// async mutex so concurrent trait calls can each drive their own async call.
pub struct WasmInstance {
    module: Module,
    linker: Linker<WasmState>,
    store: AsyncMutex<Store<WasmState>>,
    info: PluginInfo,
}

impl WasmInstance {
    /// Load a `.wasm` module from `path`, building the engine/linker/store and
    /// wiring the host functions. Synchronous so it can be called from
    /// [`load_runtime_plugin`] at plugin-discovery time.
    pub fn load(
        path: &std::path::Path,
        info: PluginInfo,
        client: reqwest::Client,
    ) -> Result<Arc<Self>, AppError> {
        // Async mode uses wasmtime's default fiber stack allocator for guest
        // calls into async host functions (host_http_get) — no custom stack
        // creator needed.
        let config = Config::default();
        let engine = Engine::new(&config)
            .map_err(|e| AppError::msg(format!("wasmtime engine：{e}")))?;

        let bytes = std::fs::read(path)
            .map_err(|e| AppError::msg(format!("读取 wasm 失败：{e}")))?;
        let module = Module::new(&engine, &bytes)
            .map_err(|e| AppError::msg(format!("wasmtime module：{e}")))?;

        let mut linker: Linker<WasmState> = Linker::new(&engine);

        // host_http_get — async: yields the tokio worker during the network
        // round-trip instead of blocking a thread. wasmtime 47's
        // `func_wrap_async` takes (Caller, Params) with Params as a single
        // tuple and expects the future's output to be the return type (i32
        // status code) directly — no `Result`/`Trap` involved.
        linker
            .func_wrap_async(
                "bilusic",
                "host_http_get",
                |mut caller: Caller<'_, WasmState>,
                 (url_ptr, url_len, out_ptr, out_len): (i32, i32, i32, i32)| {
                    Box::new(async move {
                        let url = match read_string(&mut caller, "memory", url_ptr, url_len) {
                            Some(u) => u,
                            None => return -1i32,
                        };
                        let body = match caller.data().client.get(url.as_str()).send().await {
                            Ok(r) => match r.bytes().await {
                                Ok(b) => b,
                                Err(_) => return -1i32,
                            },
                            Err(_) => return -1i32,
                        };

                        let alloc = match caller.get_export("alloc").and_then(|e| e.into_func()) {
                            Some(f) => f,
                            None => return -1i32,
                        };
                        let mut res = [Val::I32(0)];
                        if alloc
                            .call_async(
                                caller.as_context_mut(),
                                &[Val::I32(body.len() as i32)],
                                &mut res,
                            )
                            .await
                            .is_err()
                        {
                            return -1i32;
                        }
                        let out_ptr_val = match res[0].i32() {
                            Some(v) => v,
                            None => return -1i32,
                        };
                        if out_ptr_val < 0 {
                            return -1i32;
                        }
                        let mem =
                            match caller.get_export("memory").and_then(|e| e.into_memory()) {
                                Some(m) => m,
                                None => return -1i32,
                            };
                        if mem
                            .write(caller.as_context_mut(), out_ptr_val as usize, &body)
                            .is_err()
                        {
                            return -1i32;
                        }
                        if mem
                            .write(
                                caller.as_context_mut(),
                                out_ptr as usize,
                                &(out_ptr_val as i32).to_le_bytes(),
                            )
                            .is_err()
                        {
                            return -1i32;
                        }
                        if mem
                            .write(
                                caller.as_context_mut(),
                                out_len as usize,
                                &(body.len() as i32).to_le_bytes(),
                            )
                            .is_err()
                        {
                            return -1i32;
                        }
                        0i32
                    })
                },
            )
            .map_err(|e| AppError::msg(format!("注册 host_http_get：{e}")))?;

        // host_log — sync (just prints), kept as func_wrap.
        linker
            .func_wrap("bilusic", "host_log", host_log)
            .map_err(|e| AppError::msg(format!("注册 host_log：{e}")))?;

        let state = WasmState { client };
        let store = Store::new(&engine, state);

        Ok(Arc::new(Self {
            module,
            linker,
            store: AsyncMutex::new(store),
            info,
        }))
    }

    /// Call a guest export, passing `inputs` as guest-allocated byte strings.
    /// Returns the JSON value the guest wrote to its result buffer.
    pub async fn invoke(&self, export: &str, inputs: &[Vec<u8>]) -> Result<Value, AppError> {
        let mut store = self.store.lock().await;
        let instance = self
            .linker
            .instantiate_async(&mut *store, &self.module)
            .await
            .map_err(|e| AppError::msg(format!("wasmtime 实例化：{e}")))?;

        let memory = get_memory(&instance, &mut *store)?;
        let alloc = get_func(&instance, &mut *store, "alloc")?;

        // Allocate + write each input string, build the param list of
        // (ptr, len) pairs.
        let mut params: Vec<Val> = Vec::with_capacity(inputs.len() * 2);
        for inp in inputs {
            let ptr = call_i32(&alloc, &mut *store, &[Val::I32(inp.len() as i32)]).await?;
            if ptr < 0 {
                return Err(AppError::msg("wasm alloc 返回非法指针"));
            }
            memory
                .write(&mut *store, ptr as usize, inp)
                .map_err(|e| AppError::msg(format!("wasm 写入输入：{e}")))?;
            params.push(Val::I32(ptr));
            params.push(Val::I32(inp.len() as i32));
        }

        // Scratch: two i32 for (out_ptr, out_len).
        let scratch = call_i32(&alloc, &mut *store, &[Val::I32(8)]).await?;
        if scratch < 0 {
            return Err(AppError::msg("wasm alloc scratch 失败"));
        }
        memory
            .write(&mut *store, scratch as usize, &[0u8; 8])
            .map_err(|e| AppError::msg(format!("wasm 清 scratch：{e}")))?;
        params.push(Val::I32(scratch));
        params.push(Val::I32(scratch + 4));

        let func = get_func(&instance, &mut *store, export)?;
        let mut results = [Val::I32(0)];
        func.call_async(&mut *store, &params, &mut results)
            .await
            .map_err(|e| AppError::msg(format!("wasm 调用 {export}：{e}")))?;
        let status = results[0].i32().unwrap_or(-1);
        if status != 0 {
            return Err(AppError::msg(format!("wasm {export} 返回错误码 {status}")));
        }

        let out_ptr = read_i32(&memory, &mut *store, scratch)?;
        let out_len = read_i32(&memory, &mut *store, scratch + 4)?;
        if out_len < 0 {
            return Err(AppError::msg("wasm 输出长度非法"));
        }
        let mut buf = vec![0u8; out_len as usize];
        memory
            .read(&mut *store, out_ptr as usize, &mut buf)
            .map_err(|e| AppError::msg(format!("wasm 读结果：{e}")))?;
        from_slice(&buf).map_err(|e| AppError::msg(format!("wasm 解析 JSON：{e}")))
    }
}

// --------------------------------------------------------------------------
// Host functions (guest imports) — return status codes, never construct Trap.
// wasmtime 47 removed `Trap::new(&str)`; status codes map cleanly to our
// guest contract (0 = ok, non-zero = error). `host_http_get` is async (see
// the closure registered in `load`); `host_log` is a plain sync function.
// --------------------------------------------------------------------------

fn host_log(mut caller: Caller<'_, WasmState>, ptr: i32, len: i32) -> i32 {
    if let Some(msg) = read_string(&mut caller, "memory", ptr, len) {
        eprintln!("[bilusic-wasm] {msg}");
    }
    0
}

// --------------------------------------------------------------------------
// JSON <-> domain mapping
// --------------------------------------------------------------------------

fn track_from_value(v: &Value, source: &str) -> MetadataTrack {
    MetadataTrack {
        source_id: str_of(v, "source_id").to_string(),
        source: source.to_string(),
        engine_hint: None,
        title: str_of(v, "title").to_string(),
        artist: str_of(v, "artist").to_string(),
        album: str_of(v, "album").to_string(),
        duration_ms: v.get("duration_ms").and_then(|x| x.as_i64()).unwrap_or(0),
        cover: str_of(v, "cover").to_string(),
        lyrics_id: opt_str(v, "lyrics_id"),
    }
}

fn parse_tracks(v: &Value, source: &str) -> Result<Vec<MetadataTrack>, AppError> {
    let arr = match v {
        Value::Array(a) => a,
        Value::Object(o) => match o.get("results").and_then(|x| x.as_array()) {
            Some(a) => a,
            None => return Err(AppError::msg("wasm search 返回非数组")),
        },
        _ => return Err(AppError::msg("wasm search 返回非数组")),
    };
    Ok(arr.iter().map(|it| track_from_value(it, source)).collect())
}

fn parse_stream(v: &Value) -> Result<StreamInfo, AppError> {
    let mime = match opt_str(v, "mime") {
        Some(s) if !s.is_empty() => s,
        _ => "audio/mpeg".into(),
    };
    Ok(StreamInfo {
        url: str_of(v, "url").to_string(),
        mime,
        bitrate: v.get("bitrate").and_then(|x| x.as_i64()).unwrap_or(0) as u32,
        duration_ms: v.get("duration_ms").and_then(|x| x.as_i64()).unwrap_or(0),
        cover: str_of(v, "cover").to_string(),
        title: str_of(v, "title").to_string(),
        artist: str_of(v, "artist").to_string(),
    })
}

fn parse_lrc(text: &str) -> Vec<crate::metadata::LyricLine> {
    let mut out = vec![];
    for raw in text.lines() {
        let mut s = raw.trim();
        if s.is_empty() {
            continue;
        }
        let mut time_ms = 0i64;
        while s.starts_with('[') {
            if let Some(end) = s.find(']') {
                let tag = &s[1..end];
                if let Some((mm, rest)) = tag.split_once(':') {
                    if let (Ok(m), Ok(sec)) = (mm.trim().parse::<i64>(), rest.trim().parse::<f64>()) {
                        time_ms = m * 60_000 + (sec * 1000.0) as i64;
                    }
                }
                s = s[end + 1..].trim_start();
            } else {
                break;
            }
        }
        if !s.is_empty() {
            out.push(crate::metadata::LyricLine {
                time_ms,
                text: s.to_string(),
            });
        }
    }
    out
}

// --------------------------------------------------------------------------
// Adapters
// --------------------------------------------------------------------------

pub struct WasmMetadataSource {
    inner: Arc<WasmInstance>,
}

pub struct WasmAudioEngine {
    inner: Arc<WasmInstance>,
}

#[async_trait]
impl Plugin for WasmMetadataSource {
    fn info(&self) -> PluginInfo {
        self.inner.info.clone()
    }
}

#[async_trait]
impl MetadataSource for WasmMetadataSource {
    async fn search(&self, query: &str, page: u32) -> Result<Vec<MetadataTrack>, AppError> {
        let v = self
            .inner
            .invoke(
                "metadata_search",
                &[query.as_bytes().to_vec(), page.to_string().into_bytes()],
            )
            .await?;
        parse_tracks(&v, &self.inner.info.id)
    }

    async fn get_track(&self, id: &str) -> Result<MetadataTrack, AppError> {
        let v = self
            .inner
            .invoke("metadata_get_track", &[id.as_bytes().to_vec()])
            .await?;
        Ok(track_from_value(&v, &self.inner.info.id))
    }

    async fn get_lyrics(&self, track: &MetadataTrack) -> Result<Lyrics, AppError> {
        let v = self
            .inner
            .invoke("metadata_get_lyrics", &[
                track.source_id.as_bytes().to_vec(),
                track.title.as_bytes().to_vec(),
                track.artist.as_bytes().to_vec(),
            ])
            .await?;
        let text = v.as_str().map(|s| s.to_string()).unwrap_or_default();
        Ok(Lyrics {
            lines: parse_lrc(&text),
        })
    }
}

#[async_trait]
impl Plugin for WasmAudioEngine {
    fn info(&self) -> PluginInfo {
        self.inner.info.clone()
    }
}

#[async_trait]
impl AudioEngine for WasmAudioEngine {
    async fn find_stream_by_id(&self, id: &str) -> Result<StreamInfo, AppError> {
        let v = self
            .inner
            .invoke("audio_find_stream_by_id", &[id.as_bytes().to_vec()])
            .await?;
        parse_stream(&v)
    }
}

/// Build the wasm adapters for a discovered manifest. Returns the metadata
/// source plus an optional audio engine (when the manifest advertises audio
/// capability or is typed as an engine).
pub fn build_wasm_plugin(
    m: &PluginManifest,
) -> Result<(Arc<dyn MetadataSource>, Option<Arc<dyn AudioEngine>>), AppError> {
    let wasm_path = m.folder.join(&m.entry);
    let caps = PluginCapabilities::from_bits(capability_bits(&m.capabilities))
        .unwrap_or(PluginCapabilities::empty());
    let info = PluginInfo {
        id: m.id.clone(),
        name: m.name.clone(),
        version: m.version.clone(),
        description: m.description.clone(),
        capabilities: caps.bits(),
    };
    let client = reqwest::Client::new();
    let inner = WasmInstance::load(&wasm_path, info, client)?;

    let meta = Arc::new(WasmMetadataSource {
        inner: inner.clone(),
    }) as Arc<dyn MetadataSource>;
    let engine = if m.r#type == "engine" || caps.contains(PluginCapabilities::AUDIO_BY_ID) {
        Some(Arc::new(WasmAudioEngine { inner }) as Arc<dyn AudioEngine>)
    } else {
        None
    };
    Ok((meta, engine))
}

// --------------------------------------------------------------------------
// wasmtime helpers
// --------------------------------------------------------------------------

fn get_export<'a>(
    instance: &'a Instance,
    store: &mut Store<WasmState>,
    name: &str,
) -> Result<Extern, AppError> {
    instance
        .get_export(store, name)
        .ok_or_else(|| AppError::msg(format!("wasm：缺少导出 {name}")))
}

fn get_memory<'a>(instance: &'a Instance, store: &mut Store<WasmState>) -> Result<Memory, AppError> {
    get_export(instance, store, "memory")?
        .into_memory()
        .ok_or_else(|| AppError::msg("wasm：memory 导出非 Memory 类型"))
}

fn get_func<'a>(
    instance: &'a Instance,
    store: &mut Store<WasmState>,
    name: &str,
) -> Result<Func, AppError> {
    get_export(instance, store, name)?
        .into_func()
        .ok_or_else(|| AppError::msg(format!("wasm：{name} 导出非函数")))
}

async fn call_i32(func: &Func, store: &mut Store<WasmState>, params: &[Val]) -> Result<i32, AppError> {
    let mut res = [Val::I32(0)];
    func.call_async(store, params, &mut res)
        .await
        .map_err(|e| AppError::msg(format!("wasm 调用：{e}")))?;
    Ok(res[0].i32().unwrap_or(-1))
}

fn read_i32(memory: &Memory, store: &mut Store<WasmState>, offset: i32) -> Result<i32, AppError> {
    let mut buf = [0u8; 4];
    memory
        .read(store, offset as usize, &mut buf)
        .map_err(|e| AppError::msg(format!("wasm 读 i32：{e}")))?;
    Ok(i32::from_le_bytes(buf))
}

fn read_string(
    caller: &mut Caller<'_, WasmState>,
    mem_name: &str,
    ptr: i32,
    len: i32,
) -> Option<String> {
    let mem = caller.get_export(mem_name).and_then(|e| e.into_memory())?;
    let mut buf = vec![0u8; len.max(0) as usize];
    mem.read(caller.as_context(), ptr as usize, &mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("")
}

fn opt_str(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(|s| s.to_string())
}

// --------------------------------------------------------------------------
// Integration tests — load the real iTunes guest and exercise the contract.
// The guest `.wasm` is built on demand from `apps/desktop/plugins-examples/wasm/guest`
// (requires the `wasm32-unknown-unknown` target + network for serde_json).
// --------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::PluginCapabilities;
    use std::path::PathBuf;

    fn guest_wasm() -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("..");
        p.push("plugins-examples");
        p.push("wasm");
        p.push("guest");
        p.push("target");
        p.push("wasm32-unknown-unknown");
        p.push("debug");
        p.push("plugin.wasm");
        p
    }

    fn ensure_guest_built() {
        let wasm = guest_wasm();
        if wasm.exists() {
            return;
        }
        let guest_dir = wasm
            .parent() // debug
            .and_then(|p| p.parent()) // wasm32-unknown-unknown
            .and_then(|p| p.parent()) // target
            .and_then(|p| p.parent()) // guest
            .expect("resolve guest dir");
        let status = std::process::Command::new("cargo")
            .args(["build", "--target", "wasm32-unknown-unknown"])
            .current_dir(guest_dir)
            .status();
        assert!(
            status.map(|s| s.success()).unwrap_or(false),
            "guest wasm build failed"
        );
    }

    fn test_info() -> PluginInfo {
        PluginInfo {
            id: "itunes".into(),
            name: "iTunes".into(),
            version: "0.1.0".into(),
            description: "".into(),
            capabilities: PluginCapabilities::AUDIO_BY_ID.bits(),
        }
    }

    /// Fully offline: `get_lyrics` returns an empty LRC string without any
    /// host HTTP call, so it validates the whole ABI round-trip deterministically.
    #[tokio::test]
    async fn load_guest_and_offline_lyrics() {
        ensure_guest_built();
        let wasm = guest_wasm();
        assert!(wasm.exists(), "guest.wasm missing at {wasm:?}");
        let client = reqwest::Client::new();
        let inst = WasmInstance::load(&wasm, test_info(), client).expect("load guest");
        let v = inst
            .invoke(
                "metadata_get_lyrics",
                &[b"itunes:12345".to_vec(), b"".to_vec(), b"".to_vec()],
            )
            .await
            .expect("lyrics invoke");
        assert_eq!(v.as_str(), Some(""), "expected empty lyrics string");
    }

    /// Online smoke test — only runs with `-- --ignored` (needs network to
    /// itunes.apple.com). Best-effort: asserts the contract shape, not content.
    #[tokio::test]
    #[ignore]
    async fn online_search_smoke() {
        ensure_guest_built();
        let wasm = guest_wasm();
        let client = reqwest::Client::new();
        let inst = WasmInstance::load(&wasm, test_info(), client).expect("load guest");
        let v = inst
            .invoke("metadata_search", &[b"daft punk".to_vec(), b"0".to_vec()])
            .await
            .expect("search invoke");
        let arr = match &v {
            Value::Array(a) => a,
            _ => panic!("search did not return an array: {v}"),
        };
        assert!(!arr.is_empty(), "search returned empty results");
        eprintln!("[test] got {} tracks; first = {:?}", arr.len(), arr.first());
    }
}
