//! A device's explicitly imported audio. This is provenance, not a sharing grant.
//! Peer caches and service streams must never be admitted through this catalog.
//! Platform adapters own byte hashing and durable storage; this module owns the
//! bounded catalog and queue transitions, with no account or network dependency.
#![forbid(unsafe_code)]

use crate::RepeatMode;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt};

pub const VERSION: u32 = 1;
pub const MAX_AUDIO_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_ITEMS: usize = 10_000;
pub const MAX_CATALOG_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_QUEUE_ITEMS: usize = 512;
pub const MAX_TITLE_BYTES: usize = 512;
pub const MAX_JSON_INTEGER: u64 = (1_u64 << 53) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportSource {
    /// The user selected a local file. This does not establish copyright,
    /// redistribution permission, possession of a peer grant, or account trust.
    #[serde(rename = "user_file_import")]
    UserFileImport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat { Mp3, Aac, M4a, Flac, Wav, Aiff }

impl AudioFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3", Self::Aac => "aac", Self::M4a => "m4a",
            Self::Flac => "flac", Self::Wav => "wav", Self::Aiff => "aiff",
        }
    }

    /// A bounded container check, not a codec/DRM/rights validator. The platform
    /// must separately establish that the selected file is playable audio.
    pub fn detect(prefix: &[u8], extension: &str) -> Result<Self, LibraryError> {
        let extension = extension.to_ascii_lowercase();
        let id3 = prefix.starts_with(b"ID3");
        let mpeg = prefix.len() >= 2 && prefix[0] == 0xff
            && prefix[1] & 0xe0 == 0xe0 && prefix[1] & 0x06 != 0
            && prefix[1] & 0x18 != 0x08;
        let adts = prefix.len() >= 2 && prefix[0] == 0xff && prefix[1] & 0xf6 == 0xf0;
        match extension.as_str() {
            "mp3" if id3 || mpeg => Ok(Self::Mp3),
            "aac" if id3 || adts => Ok(Self::Aac),
            "m4a" if prefix.len() >= 12 && &prefix[4..8] == b"ftyp" => Ok(Self::M4a),
            "flac" if prefix.starts_with(b"fLaC") => Ok(Self::Flac),
            "wav" if prefix.len() >= 12 && &prefix[..4] == b"RIFF"
                && &prefix[8..12] == b"WAVE" => Ok(Self::Wav),
            "aif" | "aiff" if prefix.len() >= 12 && &prefix[..4] == b"FORM"
                && matches!(&prefix[8..12], b"AIFF" | b"AIFC") => Ok(Self::Aiff),
            _ => Err(LibraryError("Choose a supported local audio file.")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnedTrack {
    /// SHA-256 of all imported file bytes, lowercase hexadecimal. Adapters must
    /// calculate this themselves; a caller-supplied hash is not evidence.
    pub id: String,
    pub title: String,
    pub format: AudioFormat,
    pub byte_length: u64,
    pub imported_at_unix_ms: u64,
    pub source: ImportSource,
}
impl OwnedTrack {
    pub fn validate(&self) -> Result<(), LibraryError> {
        if !valid_id(&self.id) || self.title.trim().is_empty()
            || self.title.len() > MAX_TITLE_BYTES || self.title.chars().any(char::is_control)
            || self.byte_length == 0 || self.byte_length > MAX_AUDIO_BYTES
            || self.imported_at_unix_ms > MAX_JSON_INTEGER {
            return Err(LibraryError("The imported audio record is invalid."));
        }
        Ok(())
    }
    pub fn file_name(&self) -> String { format!("{}.{}", self.id, self.format.extension()) }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnedQueue {
    pub items: Vec<String>,
    pub current_index: Option<u32>,
    pub repeat_mode: RepeatMode,
    /// The order in items is already resolved; this flag does not authorize
    /// unbounded random work or alter the sequence during a retry.
    pub shuffled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnedLibrary {
    pub version: u32,
    /// Random public namespace for this catalog, never a device key or grant.
    pub library_id: String,
    pub revision: u64,
    pub items: Vec<OwnedTrack>,
    pub queue: OwnedQueue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibraryError(pub &'static str);
impl fmt::Display for LibraryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.0) }
}
impl std::error::Error for LibraryError {}

pub fn valid_id(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl OwnedLibrary {
    pub fn new(library_id: String) -> Result<Self, LibraryError> {
        let value = Self { version: VERSION, library_id, revision: 0,
            items: Vec::new(), queue: OwnedQueue::default() };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), LibraryError> {
        if self.version != VERSION || !valid_id(&self.library_id)
            || self.revision > MAX_JSON_INTEGER || self.items.len() > MAX_ITEMS {
            return Err(LibraryError("The local music catalog is invalid."));
        }
        let mut ids = BTreeSet::new();
        let mut previous: Option<&str> = None;
        for item in &self.items {
            item.validate()?;
            if previous.is_some_and(|id| id >= item.id.as_str()) || !ids.insert(item.id.as_str()) {
                return Err(LibraryError("The local music catalog has conflicting items."));
            }
            previous = Some(&item.id);
        }
        Self::validate_queue(&self.queue, &ids)
    }

    fn validate_queue(queue: &OwnedQueue, ids: &BTreeSet<&str>) -> Result<(), LibraryError> {
        if queue.items.len() > MAX_QUEUE_ITEMS
            || queue.items.iter().any(|id| !ids.contains(id.as_str()))
            || queue.current_index.is_some_and(|i| i as usize >= queue.items.len()) {
            return Err(LibraryError("The local music queue contains unavailable items."));
        }
        Ok(())
    }

    pub fn check_revision(&self, library_id: &str, expected: u64) -> Result<(), LibraryError> {
        self.validate()?;
        if self.library_id != library_id || self.revision != expected {
            return Err(LibraryError("The local library changed. Refresh and try again."));
        }
        Ok(())
    }

    fn next_revision(&self) -> Result<u64, LibraryError> {
        self.revision.checked_add(1).filter(|v| *v <= MAX_JSON_INTEGER)
            .ok_or(LibraryError("The local music revision is exhausted."))
    }

    pub fn track(&self, id: &str) -> Option<&OwnedTrack> {
        self.items.binary_search_by(|item| item.id.as_str().cmp(id)).ok().map(|i| &self.items[i])
    }

    /// An identical retry retains the first import's metadata and revision.
    pub fn import_track(&mut self, track: OwnedTrack) -> Result<bool, LibraryError> {
        self.validate()?;
        track.validate()?;
        if let existing = self.track(&track.id) {
            if existing.byte_length != track.byte_length || existing.format != track.format {
                return Err(LibraryError("The imported audio conflicts with its saved content identity."));
            }
            return Ok(false);
        }
        if self.items.len() >= MAX_ITEMS { return Err(LibraryError("The local music library is full.")); }
        let revision = self.next_revision()?;
        let index = self.items.binary_search_by(|item| item.id.cmp(&track.id)).unwrap_err();
        self.items.insert(index, track);
        self.revision = revision;
        Ok(true)
    }

    pub fn remove_track(&mut self, id: &str) -> Result<bool, LibraryError> {
        self.validate()?;
        if !valid_id(id) { return Err(LibraryError("The local music item is invalid.")); }
        let Ok(index) = self.items.binary_search_by(|item| item.id.as_str().cmp(id)) else { return Ok(false) };
        let revision = self.next_revision()?;
        let old_cursor = self.queue.current_index.map(|v| v as usize);
        let before = old_cursor.map(|cursor| self.queue.items[..cursor].iter().filter(|value| value.as_str() == id).count()).unwrap_or(0);
        self.items.remove(index);
        self.queue.items.retain(|value| value != id);
        self.queue.current_index = old_cursor.and_then(|cursor| {
            let next = cursor - before;
            (next < self.queue.items.len()).then_some(next as u32)
        });
        self.revision = revision;
        Ok(true)
    }

    pub fn set_queue(&mut self, queue: OwnedQueue) -> Result<bool, LibraryError> {
        self.validate()?;
        let ids = self.items.iter().map(|i| i.id.as_str()).collect();
        Self::validate_queue(&queue, &ids)?;
        if self.queue == queue { return Ok(false) }
        let revision = self.next_revision()?;
        self.queue = queue;
        self.revision = revision;
        Ok(true)
    }

    pub fn select(&mut self, id: &str) -> Result<(), LibraryError> {
        self.validate()?;
        if self.track(id).is_none() { return Err(LibraryError("This local music item is unavailable.")); }
        let mut queue = self.queue.clone();
        if let Some(index) = queue.items.iter().position(|value| value == id) {
            queue.current_index = Some(index as u32);
        } else {
            queue.items = vec![id.to_owned()];
            queue.current_index = Some(0);
            queue.shuffled = false;
        }
        self.set_queue(queue)?;
        Ok(())
    }

    pub fn current(&self) -> Option<&OwnedTrack> {
        self.queue.current_index.and_then(|i| self.queue.items.get(i as usize)).and_then(|id| self.track(id))
    }

    /// Automatic completion respects repeat-one; explicit Next/Previous does not.
    pub fn advance(&mut self, forward: bool, automatic: bool) -> Result<Option<String>, LibraryError> {
        self.validate()?;
        let count = self.queue.items.len();
        if count == 0 { return Ok(None) }
        let cursor = self.queue.current_index.map(|i| i as usize);
        let next = if automatic && self.queue.repeat_mode == RepeatMode::One && cursor.is_some() {
            cursor
        } else {
            match (cursor, forward) {
                (None, true) => Some(0),
                (None, false) => Some(count - 1),
                (Some(i), true) if i + 1 < count => Some(i + 1),
                (Some(i), false) if i > 0 => Some(i - 1),
                (_, true) if self.queue.repeat_mode == RepeatMode::All => Some(0),
                (_, false) if self.queue.repeat_mode == RepeatMode::All => Some(count - 1),
                _ => None,
            }
        };
        let mut queue = self.queue.clone();
        queue.current_index = next.map(|i| i as u32);
        self.set_queue(queue)?;
        Ok(self.current().map(|item| item.id.clone()))
    }

    pub fn search(&self, query: &str) -> Result<Vec<&OwnedTrack>, LibraryError> {
        self.validate()?;
        if query.len() > 256 || query.chars().any(char::is_control) {
            return Err(LibraryError("The local music search is too long."));
        }
        let query = query.trim().to_lowercase();
        Ok(self.items.iter().filter(|item| item.title.to_lowercase().contains(&query)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(c: char) -> OwnedTrack {
        OwnedTrack { id: c.to_string().repeat(64), title: format!("Song {c}"),
            format: AudioFormat::Wav, byte_length: 128, imported_at_unix_ms: 1,
            source: ImportSource::UserFileImport }
    }
    fn library() -> OwnedLibrary { OwnedLibrary::new("f".repeat(64)).unwrap() }

    #[test]
    fn imports_are_stable_and_conflicts_do_not_mutate() {
        let mut library = library();
        let a = item('a');
        assert!(library.import_track(a.clone()).unwrap());
        let snapshot = library.clone();
        let mut duplicate = a.clone(); duplicate.title = "Other name".into();
        assert!(!library.import_track(duplicate).unwrap());
        assert_eq!(library, snapshot);
        let mut conflict = a; conflict.byte_length += 1;
        assert!(library.import_track(conflict).is_err());
        assert_eq!(library, snapshot);
        let mut invalid = item('b'); invalid.title = "bad\nname".into();
        assert!(library.import_track(invalid).is_err());
        assert_eq!(library, snapshot);
    }

    #[test]
    fn deletion_preserves_queue_positions_without_dangling_references() {
        let mut library = library();
        for c in ['a', 'b', 'c'] { library.import_track(item(c)).unwrap(); }
        let ids = ['a', 'b', 'a', 'c'].map(|c| c.to_string().repeat(64));
        library.set_queue(OwnedQueue { items: ids.to_vec(), current_index: Some(2), ..Default::default() }).unwrap();
        library.remove_track(&ids[0]).unwrap();
        assert_eq!(library.queue.items, [ids[1].clone(), ids[3].clone()]);
        assert_eq!(library.current().unwrap().id, ids[3]);
        library.validate().unwrap();
        library.remove_track(&ids[3]).unwrap();
        assert!(library.current().is_none());
        library.validate().unwrap();
    }

    #[test]
    fn repeat_and_stale_revision_are_deterministic() {
        let mut library = library();
        library.import_track(item('a')).unwrap(); library.import_track(item('b')).unwrap();
        let ids = ["a".repeat(64), "b".repeat(64)];
        library.set_queue(OwnedQueue { items: ids.to_vec(), current_index: Some(0),
            repeat_mode: RepeatMode::One, shuffled: false }).unwrap();
        assert_eq!(library.advance(true, true).unwrap(), Some(ids[0].clone()));
        assert_eq!(library.advance(true, false).unwrap(), Some(ids[1].clone()));
        assert!(library.check_revision(&library.library_id, library.revision - 1).is_err());
        assert_eq!(library.advance(true, false).unwrap(), None);
        let snapshot = library.clone();
        let invalid = OwnedQueue { items: vec!["e".repeat(64)], ..Default::default() };
        assert!(library.set_queue(invalid).is_err());
        assert_eq!(library, snapshot);
    }

    #[test]
    fn wire_cannot_claim_peer_or_service_ownership() {
        let track = item('a');
        let value = serde_json::to_value(&track).unwrap();
        assert_eq!(value["source"], "user_file_import");
        let mut peer = value.clone(); peer["source"] = serde_json::json!("peer_cache");
        assert!(serde_json::from_value::<OwnedTrack>(peer).is_err());
        let mut grant = value; grant["reshareAllowed"] = serde_json::json!(true);
        assert!(serde_json::from_value::<OwnedTrack>(grant).is_err());
        let mut library = library(); library.items = vec![item('b'), item('a')];
        assert!(library.validate().is_err());
        library.items = vec![item('a')]; library.revision = MAX_JSON_INTEGER;
        let snapshot = library.clone();
        assert!(library.import_track(item('b')).is_err());
        assert_eq!(library, snapshot);
    }

    #[test]
    fn container_sniff_rejects_playlists_and_mismatched_extensions() {
        assert_eq!(AudioFormat::detect(b"RIFF0000WAVEdata", "wav").unwrap(), AudioFormat::Wav);
        assert_eq!(AudioFormat::detect(b"FORM0000AIFF", "aif").unwrap(), AudioFormat::Aiff);
        assert!(AudioFormat::detect(b"#EXTM3U", "mp3").is_err());
        assert!(AudioFormat::detect(b"RIFF0000WAVEdata", "m4a").is_err());
        assert!(AudioFormat::detect(b"ID3", "m4p").is_err());
        assert!(AudioFormat::detect(b"fLaC", "https://example.test/audio.flac").is_err());
    }
}
