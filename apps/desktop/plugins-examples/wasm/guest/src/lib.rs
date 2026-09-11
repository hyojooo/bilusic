//! Bilusic WASM guest example — wraps the public iTunes Search / Lookup API.
//!
//! This is a reference implementation of the host <-> guest contract described
//! in `apps/desktop/src-tauri/src/wasm_runtime.rs`. It deliberately uses only
//! `std` + `serde_json` so it builds with a single command:
//!
//! ```sh
//! cargo build --target wasm32-unknown-unknown
//! ```
//!
//! No allocator shims are needed: `std` on `wasm32-unknown-unknown` already
//! provides a default global allocator and exports linear `memory`.

use std::alloc::{alloc as sys_alloc, Layout};
use std::ptr;

// --- host imports (wasmtime linker registers these under module "bilusic") --

#[link(wasm_import_module = "bilusic")]
extern "C" {
    /// Performs an HTTP GET, writes the response body into a guest-allocated
    /// buffer, and stores (ptr, len) into the two scratch slots. Returns 0 on
    /// success, non-zero on error.
    fn host_http_get(url_ptr: i32, url_len: i32, out_ptr_slot: i32, out_len_slot: i32) -> i32;
    /// Prints a debug line to the host's stderr. Return value is ignored.
    fn host_log(msg_ptr: i32, msg_len: i32) -> i32;
}

// --- low-level memory helpers ------------------------------------------------

/// Bump allocator over the wasm linear memory. Leaked chunks are fine for a
/// short-lived guest that only allocates a few buffers per call.
#[no_mangle]
pub extern "C" fn alloc(size: i32) -> i32 {
    if size <= 0 {
        return 0;
    }
    let layout = match Layout::from_size_align(size as usize, 1) {
        Ok(l) => l,
        Err(_) => return 0,
    };
    let ptr = unsafe { sys_alloc(layout) };
    ptr as i32
}

fn load_bytes(ptr: i32, len: i32) -> Vec<u8> {
    let len = len.max(0) as usize;
    let mut v = vec![0u8; len];
    unsafe {
        ptr::copy_nonoverlapping(ptr as *const u8, v.as_mut_ptr(), len);
    }
    v
}

fn store_bytes(ptr: i32, data: &[u8]) {
    unsafe {
        ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, data.len());
    }
}

fn read_i32(ptr: i32) -> i32 {
    let mut b = [0u8; 4];
    unsafe {
        ptr::copy_nonoverlapping(ptr as *const u8, b.as_mut_ptr(), 4);
    }
    i32::from_le_bytes(b)
}

fn write_i32(ptr: i32, v: i32) {
    let b = v.to_le_bytes();
    unsafe {
        ptr::copy_nonoverlapping(b.as_ptr(), ptr as *mut u8, 4);
    }
}

/// Allocate `json` in guest memory and publish (ptr, len) into the two output
/// scratch slots the host passed. Mirrors the host `invoke` contract.
fn emit(out_ptr_slot: i32, out_len_slot: i32, json: &[u8]) -> i32 {
    let ptr = alloc(json.len() as i32);
    if ptr == 0 {
        return -1;
    }
    store_bytes(ptr, json);
    write_i32(out_ptr_slot, ptr);
    write_i32(out_len_slot, json.len() as i32);
    0
}

fn log(msg: &str) {
    let bytes = msg.as_bytes();
    unsafe {
        host_log(bytes.as_ptr() as i32, bytes.len() as i32);
    }
}

/// Minimal percent-encoder for the query string (spaces -> '+').
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Call `host_http_get` and return the raw response body. `None` on any error.
fn http_get(url: &str) -> Option<Vec<u8>> {
    let url_bytes = url.as_bytes();
    let out_ptr_slot = alloc(4);
    let out_len_slot = alloc(4);
    if out_ptr_slot == 0 || out_len_slot == 0 {
        return None;
    }
    let status = unsafe {
        host_http_get(
            url_bytes.as_ptr() as i32,
            url_bytes.len() as i32,
            out_ptr_slot,
            out_len_slot,
        )
    };
    if status != 0 {
        return None;
    }
    let body_ptr = read_i32(out_ptr_slot);
    let body_len = read_i32(out_len_slot);
    if body_ptr == 0 || body_len < 0 {
        return None;
    }
    Some(load_bytes(body_ptr, body_len))
}

// --- iTunes field mapping ----------------------------------------------------

const SOURCE_PREFIX: &str = "itunes:";

fn strip_prefix(id: &str) -> &str {
    id.strip_prefix(SOURCE_PREFIX).unwrap_or(id)
}

/// Map one iTunes result object to a Bilusic track object.
fn track_to_json(t: &serde_json::Value) -> serde_json::Value {
    let id = t.get("trackId").and_then(|x| x.as_i64()).unwrap_or(0);
    let artwork = t
        .get("artworkUrl100")
        .and_then(|x| x.as_str())
        .map(|s| s.replace("100x100", "512x512"))
        .unwrap_or_default();
    let dur = t
        .get("trackTimeMillis")
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    let mut o = serde_json::Map::new();
    o.insert("source_id".into(), serde_json::json!(format!("{SOURCE_PREFIX}{id}")));
    o.insert(
        "title".into(),
        serde_json::json!(t.get("trackName").and_then(|x| x.as_str()).unwrap_or("")),
    );
    o.insert(
        "artist".into(),
        serde_json::json!(t.get("artistName").and_then(|x| x.as_str()).unwrap_or("")),
    );
    o.insert(
        "album".into(),
        serde_json::json!(t.get("collectionName").and_then(|x| x.as_str()).unwrap_or("")),
    );
    o.insert("duration_ms".into(), serde_json::json!(dur));
    o.insert("cover".into(), serde_json::json!(artwork));
    o.insert("lyrics_id".into(), serde_json::json!(format!("{SOURCE_PREFIX}{id}")));
    serde_json::Value::Object(o)
}

/// Look up a single track by its numeric iTunes id and map it.
fn lookup_track(numeric_id: &str) -> Option<serde_json::Value> {
    let url = format!("https://itunes.apple.com/lookup?id={numeric_id}&entity=song");
    let body = http_get(&url)?;
    let v: serde_json::Value = serde_json::from_slice(&body).ok()?;
    let results = v.get("results").and_then(|x| x.as_array())?;
    results.first().map(track_to_json)
}

// --- exported guest functions ------------------------------------------------

#[no_mangle]
pub extern "C" fn metadata_search(
    qptr: i32,
    qlen: i32,
    _pageptr: i32,
    _pagelen: i32,
    out_ptr_slot: i32,
    out_len_slot: i32,
) -> i32 {
    let query = String::from_utf8_lossy(&load_bytes(qptr, qlen)).to_string();
    log(&format!("itunes search: {query}"));
    let url = format!(
        "https://itunes.apple.com/search?term={}&media=music&limit=20",
        urlencode(&query)
    );
    let body = match http_get(&url) {
        Some(b) => b,
        None => return -1,
    };
    let v: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(x) => x,
        Err(e) => {
            log(&format!("itunes parse error: {e}"));
            return -1;
        }
    };
    let results = v.get("results").and_then(|x| x.as_array());
    let arr: Vec<serde_json::Value> = match results {
        Some(r) => r.iter().map(track_to_json).collect(),
        None => Vec::new(),
    };
    let out = serde_json::to_vec(&arr).unwrap_or_default();
    emit(out_ptr_slot, out_len_slot, &out)
}

#[no_mangle]
pub extern "C" fn metadata_get_track(
    idptr: i32,
    idlen: i32,
    out_ptr_slot: i32,
    out_len_slot: i32,
) -> i32 {
    let id = String::from_utf8_lossy(&load_bytes(idptr, idlen)).to_string();
    let numeric = strip_prefix(&id).to_string();
    let track = match lookup_track(&numeric) {
        Some(t) => t,
        None => return -1,
    };
    let out = serde_json::to_vec(&track).unwrap_or_default();
    emit(out_ptr_slot, out_len_slot, &out)
}

#[no_mangle]
pub extern "C" fn metadata_get_lyrics(
    sidptr: i32,
    sidlen: i32,
    _titleptr: i32,
    _titlelen: i32,
    _artistptr: i32,
    _artistlen: i32,
    out_ptr_slot: i32,
    out_len_slot: i32,
) -> i32 {
    // iTunes does not expose lyrics; return an empty LRC string. This makes the
    // function fully offline (no host HTTP call), handy for a handshake test.
    let _sid = String::from_utf8_lossy(&load_bytes(sidptr, sidlen)).to_string();
    let out = serde_json::to_vec("").unwrap_or_default();
    emit(out_ptr_slot, out_len_slot, &out)
}

#[no_mangle]
pub extern "C" fn audio_find_stream_by_id(
    idptr: i32,
    idlen: i32,
    out_ptr_slot: i32,
    out_len_slot: i32,
) -> i32 {
    let id = String::from_utf8_lossy(&load_bytes(idptr, idlen)).to_string();
    let numeric = strip_prefix(&id).to_string();
    let track = match lookup_track(&numeric) {
        Some(t) => t,
        None => return -1,
    };
    // iTunes only provides a 30s AAC preview; map it as the stream.
    let preview = track
        .get("previewUrl")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if preview.is_empty() {
        return -1;
    }
    let dur = track
        .get("trackTimeMillis")
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    let title = track
        .get("trackName")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let artist = track
        .get("artistName")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let mut o = serde_json::Map::new();
    o.insert("url".into(), serde_json::json!(preview));
    o.insert("mime".into(), serde_json::json!("audio/x-m4a"));
    o.insert("bitrate".into(), serde_json::json!(0));
    o.insert("duration_ms".into(), serde_json::json!(dur));
    o.insert(
        "cover".into(),
        serde_json::json!(track
            .get("artworkUrl100")
            .and_then(|x| x.as_str())
            .unwrap_or("")),
    );
    o.insert("title".into(), serde_json::json!(title));
    o.insert("artist".into(), serde_json::json!(artist));
    let out = serde_json::to_vec(&serde_json::Value::Object(o)).unwrap_or_default();
    emit(out_ptr_slot, out_len_slot, &out)
}
