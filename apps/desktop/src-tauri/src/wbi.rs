//! Bilibili WBI (web interface) request signing.
//!
//! Bilibili's `nav`, `view`, `playurl` and `search/type` endpoints require a
//! `w_rid` signature derived from the account's `img_key` / `sub_key` (fetched
//! from the `nav` API) plus a timestamp. This module implements the well-known
//! mixin-key algorithm and query signing.

use url::form_urlencoded;

/// 64-entry permutation table used to derive the 32-char mixin key.
const MIXIN_TABLE: [usize; 64] = [
    39, 37, 33, 15, 53, 43, 9, 2, 52, 51, 1, 35, 48, 34, 8, 10, 47, 6, 17, 0, 28, 49, 31, 13, 12,
    45, 27, 24, 46, 22, 19, 29, 38, 21, 14, 11, 32, 41, 18, 25, 16, 40, 3, 42, 54, 5, 4, 23, 50,
    44, 36, 20, 30, 7, 56, 26, 55, 57, 58, 59, 60, 61, 62, 63,
];

/// Build the 32-char mixin key from the img + sub key filenames.
pub fn mixin_key(img_key: &str, sub_key: &str) -> String {
    let combined = format!("{img_key}{sub_key}");
    let mut key = String::with_capacity(32);
    for &idx in &MIXIN_TABLE {
        if let Some(c) = combined.chars().nth(idx) {
            key.push(c);
        }
    }
    key.truncate(32);
    key
}

/// Percent-encode a single query value (application/x-www-form-urlencoded).
pub fn enc(value: &str) -> String {
    form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn md5_hex(input: &str) -> String {
    let digest = md5::compute(input.as_bytes());
    let bytes: &[u8] = digest.as_ref();
    let mut out = String::with_capacity(32);
    for b in bytes {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

/// Append `wts` + `w_rid` to the given parameter list and return the signed
/// (sorted) parameter vector ready to be turned into a query string.
pub fn signed_params(mut params: Vec<(String, String)>, img_key: &str, sub_key: &str) -> Vec<(String, String)> {
    let wts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    params.push(("wts".to_string(), wts.to_string()));
    params.sort_by(|a, b| a.0.cmp(&b.0));

    let mut query = String::new();
    for (k, v) in &params {
        query.push_str(&format!("{}={}&", enc(k), enc(v)));
    }
    query.pop(); // drop trailing '&'

    let mk = mixin_key(img_key, sub_key);
    let w_rid = md5_hex(&format!("{query}{mk}"));
    params.push(("w_rid".to_string(), w_rid));
    params
}

/// Build a full signed URL from an endpoint and a parameter list.
pub fn signed_url(endpoint: &str, params: Vec<(String, String)>, img_key: &str, sub_key: &str) -> String {
    let signed = signed_params(params, img_key, sub_key);
    let query: String = signed
        .iter()
        .map(|(k, v)| format!("{}={}&", enc(k), enc(v)))
        .collect();
    format!("{}?{}", endpoint, query.trim_end_matches('&'))
}
