//! Common trait for every drop-in plugin (metadata source or audio engine).
//!
//! Both halves of Bilusic's decoupled architecture implement [`Plugin`], so the
//! settings UI can list, enable, and remove them uniformly. Each plugin also
//! reports its `PluginCapabilities` so the coordinator can pick an
//! implementation path (search-only, stream-by-id, stream-by-query, etc.).

use bitflags::bitflags;
use serde::Serialize;

bitflags! {
    /// Capability bits advertised by a plugin.
    ///
    /// A metadata plugin typically only sets the `METADATA_*` bits; an audio
    /// engine sets the `AUDIO_*` bits. Some plugins (e.g. Bilibili metadata)
    /// could carry both — kept separate here for clarity.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PluginCapabilities: u32 {
        const METADATA_SEARCH   = 1 << 0;
        const METADATA_GET      = 1 << 1;
        const METADATA_LYRICS   = 1 << 2;
        const METADATA_HOME     = 1 << 3;
        const AUDIO_BY_ID       = 1 << 4;
        const AUDIO_BY_QUERY    = 1 << 5;
        const AUDIO_NEEDS_AUTH  = 1 << 6;
    }
}

/// A plugin descriptor serialized to the frontend. Capabilities are exposed as
/// a bitmask so the UI can interpret them without needing Rust-side reflection.
///
/// Fields are owned `String`s (not `&'static str`) so third-party plugins
/// discovered from `plugin.json` at runtime can populate them too.
#[derive(Debug, Clone, Serialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub capabilities: u32,
}

impl PluginInfo {
    pub fn from(
        id: &'static str,
        name: &'static str,
        version: &'static str,
        description: &'static str,
        caps: PluginCapabilities,
    ) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            version: version.to_string(),
            description: description.to_string(),
            capabilities: caps.bits(),
        }
    }
}

pub trait Plugin: Send + Sync {
    fn info(&self) -> PluginInfo;
    fn capabilities(&self) -> PluginCapabilities {
        PluginCapabilities::from_bits(self.info().capabilities).unwrap_or(PluginCapabilities::empty())
    }
}
