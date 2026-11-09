//! Version 1 Bluefin server/client wire contract. Contains no server authority.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_QUEUE_ITEMS: usize = 512;
pub const MAX_QUEUE_BODY_BYTES: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub protocol_version: u32,
    pub music_queue_sync: bool,
    pub max_queue_items: usize,
    pub viewer: ViewerCapabilities,
}

/// Read-only policy projection, never accepted as preferences or authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerCapabilities {
    pub user_id: Uuid,
    pub media_playback: bool,
    pub audio_transcoding: bool,
    pub content_downloading: bool,
    pub live_tv_access: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

/// Cursor is an index in item_ids, or null. Duplicate tracks are allowed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueueState {
    pub item_ids: Vec<Uuid>,
    pub cursor: Option<usize>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
}

impl QueueState {
    pub fn is_valid(&self) -> bool {
        self.item_ids.len() <= MAX_QUEUE_ITEMS
            && self.item_ids.iter().all(|id| !id.is_nil())
            && self.cursor.is_none_or(|index| index < self.item_ids.len())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MusicQueue {
    pub protocol_version: u32,
    pub revision: u64,
    pub queue: QueueState,
}

impl Default for MusicQueue {
    fn default() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            revision: 0,
            queue: QueueState::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateMusicQueue {
    pub protocol_version: u32,
    pub expected_revision: u64,
    pub queue: QueueState,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wire_and_bounds() {
        let request: UpdateMusicQueue = serde_json::from_str(r#"{"protocolVersion":1,"expectedRevision":0,"queue":{"itemIds":[],"cursor":null,"shuffle":false,"repeat":"off"}}"#).unwrap();
        assert!(request.queue.is_valid());
        let mut queue = request.queue;
        queue.cursor = Some(0);
        assert!(!queue.is_valid());
        queue.cursor = None;
        queue.item_ids = vec![Uuid::from_u128(1); MAX_QUEUE_ITEMS + 1];
        assert!(!queue.is_valid());
        assert!(
            serde_json::from_str::<QueueState>(
                r#"{"itemIds":[],"cursor":null,"shuffle":false,"repeat":"off","userId":"other"}"#
            )
            .is_err()
        );
    }
}
