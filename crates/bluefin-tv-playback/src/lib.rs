//! TV wire contract, portable client policy, and bounded playback capabilities.
//! This crate makes no content-access decisions. The server backend must apply
//! its existing current-session, account, network and item policy each time.
use serde::{Deserialize, Serialize};

pub mod client;
#[cfg(feature = "server")]
pub mod server;
#[cfg(target_arch = "wasm32")]
mod wasm;

pub const VERSION: u8 = 1;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaseRequest {
    pub version: u8,
    pub item_id: String,
    pub playback: PlaybackOptions,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlaybackOptions {
    pub kind: MediaKind,
    pub max_bitrate: u32,
    #[serde(default)]
    pub start_ticks: u64,
    #[serde(default)]
    pub audio_stream_index: Option<u16>,
    #[serde(default)]
    pub subtitle_stream_index: Option<u16>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Video,
    Audio,
}
impl LeaseRequest {
    pub fn valid(&self) -> bool {
        self.version == VERSION
            && self.item_id.len() == 32
            && self.item_id.bytes().all(|c| c.is_ascii_hexdigit())
            && (128_000..=20_000_000).contains(&self.playback.max_bitrate)
            && self.playback.start_ticks <= 7 * 24 * 3600 * 10_000_000
    }
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaseResponse {
    pub version: u8,
    /// Relative to the configured server origin/base path. Contains no account token.
    pub media_url: String,
    pub release_url: String,
    pub play_session_id: String,
    pub expires_in_seconds: u64,
    pub idle_seconds: u64,
}
