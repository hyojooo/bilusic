//! Built-in plugins — compiled-in Rust trait implementations (P5-II,
//! native-first; no V8 / deno_core). Each lives in its own file. `lib.rs`
//! constructs these instances directly so the registry and the cookie service
//! can share the same `Arc` — equivalent to a manual "registration" step.
//!
//! First-party sources:
//! - `qq_music_metadata` — search / home (legacy `c.y.qq.com` endpoints).
//! - `bilibili_metadata` — B 站原生元数据（与 bilibili_engine 同源 id）。
//! - `bilibili_engine` — B 站音频引擎（跨源播放的实际字节来源）。
//! - `lrclib` — **lyrics-only**，无鉴权公开 REST，完整实现（参考实现）。
//! - `netease_metadata` — 网易云音乐元数据（weapi 加密 · 搜索 / 单曲 / 歌词 / 歌单 / 推荐）。
//!
//! N14 (2026-08-27): `kugou` / `spotify` / `ytmusic` 三个 阶段 2 skeleton 已被
//! 彻底删除——所有 op 直接 `Err(NOT_IMPL)`、永不启用，纯死代码。如未来真要
//! 接入对应平台，重新建 .rs + 注册即可。
//! - `kuwo` — 已移至 JS 插件（`apps/desktop/plugins/kuwo/`，真实可用）。

pub mod bilibili_engine;
pub mod bilibili_metadata;
pub mod lrclib;
pub mod netease_metadata;
pub mod qq_music_metadata;
