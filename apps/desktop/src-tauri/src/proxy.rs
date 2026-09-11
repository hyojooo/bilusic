//! Local streaming proxy.
//!
//! Bilibili stream URLs carry per-request防盗链 / Referer / Cookie checks that
//! a browser `<audio>` element cannot satisfy (it cannot set `Referer` or auth
//! cookies). So we run a tiny localhost HTTP server that fetches the upstream
//! stream with the correct headers and streams the bytes back to the webview.
//! Range requests are forwarded so seeking works natively.

use axum::{
    body::Body,
    extract::Query,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use std::collections::HashMap;
use std::sync::OnceLock;

use reqwest::Client;

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
/// Fixed localhost port for the proxy (must match `PROXY_BASE` on the frontend).
pub const PROXY_PORT: u16 = 9527;
pub const PROXY_BASE: &str = "http://127.0.0.1:9527";

/// Proxy HTTP client (no cookie store — cookies arrive per-request via `ck`).
fn client() -> &'static Client {
    static C: OnceLock<Client> = OnceLock::new();
    C.get_or_init(|| Client::builder().user_agent(UA).build().expect("proxy client"))
}

/// Headers we copy through from the upstream response.
const PASSTHROUGH: &[header::HeaderName] = &[
    header::CONTENT_TYPE,
    header::CONTENT_LENGTH,
    header::CONTENT_RANGE,
    header::ACCEPT_RANGES,
    header::CACHE_CONTROL,
    header::ETAG,
];

async fn stream(Query(params): Query<HashMap<String, String>>, headers: HeaderMap) -> impl IntoResponse {
    let url = match params.get("u") {
        Some(u) if !u.is_empty() => u,
        _ => return (StatusCode::BAD_REQUEST, "missing stream url `u`").into_response(),
    };
    let cookie = params.get("ck").cloned();
    let mime_override = params.get("mime").cloned();

    let mut req = client()
        .get(url)
        .header("Referer", "https://www.bilibili.com/")
        .header("Origin", "https://www.bilibili.com")
        .header("User-Agent", UA)
        .header("Accept", "*/*");
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    if let Some(range) = headers.get(header::RANGE).cloned() {
        req = req.header(header::RANGE, range);
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("upstream error: {e}")).into_response(),
    };

    let status = resp.status();
    if !status.is_success() {
        // Read the body + a couple of headers for the diagnostic payload.
        // `text()` consumes `resp`, so collect content-type up-front.
        let upstream_ct = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = resp.text().await.unwrap_or_default();
        let payload = serde_json::json!({
            "upstream_status": status.as_u16(),
            "upstream_content_type": upstream_ct,
            "body_excerpt": body.chars().take(400).collect::<String>(),
        });
        return (status, axum::Json(payload)).into_response();
    }

    let mut builder = Response::builder().status(status);
    for name in PASSTHROUGH {
        if let Some(v) = resp.headers().get(name) {
            builder = builder.header(name, v);
        }
    }
    // B 站 DASH .m4s segments often return `application/octet-stream` even
    // when the audio element would happily play them as `audio/mp4`. Let
    // the caller pin the correct content type via `?mime=...`.
    if let Some(mime) = mime_override {
        builder = builder.header(header::CONTENT_TYPE, mime);
    }

    match builder.body(Body::from_stream(resp.bytes_stream())) {
        Ok(r) => r.into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "failed to build response").into_response(),
    }
}

/// Spawn the proxy server on the Tauri runtime. Safe to call once at startup;
/// the bind error is logged and the task exits silently on port conflict.
pub fn start_proxy() {
    tauri::async_runtime::spawn(run_proxy());
}

/// Bind the proxy server and serve forever. Spawned from Tauri's setup hook.
pub async fn run_proxy() {
    let app = Router::new().route("/stream", get(stream));
    let addr: std::net::SocketAddr = match format!("127.0.0.1:{PROXY_PORT}").parse() {
        Ok(a) => a,
        Err(_) => return,
    };
    match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => {
            let _ = axum::serve(listener, app).await;
        }
        Err(e) => {
            eprintln!("Bilusic proxy failed to bind 127.0.0.1:{PROXY_PORT}: {e}");
        }
    }
}
