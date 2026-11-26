use crate::protocol::{Item, Result, Session};
use bluefin_contract::{Capabilities, MusicQueue, PROTOCOL_VERSION};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Home,
    Sports,
    Shows,
    Movies,
    Guide,
    Music,
    Search,
    Collections,
    Account,
    Queue,
    Discovery,
}
impl Section {
    pub const PRIMARY: [Self; 6] = [
        Self::Home,
        Self::Sports,
        Self::Shows,
        Self::Movies,
        Self::Guide,
        Self::Music,
    ];
    pub const UTILITIES: [Self; 3] = [Self::Search, Self::Collections, Self::Account];
    pub fn title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Sports => "Sports",
            Self::Shows => "TV Shows",
            Self::Movies => "Movies",
            Self::Guide => "Live TV",
            Self::Music => "Music",
            Self::Search => "Search",
            Self::Collections => "Collections",
            Self::Account => "Account",
            Self::Queue => "Music queue",
            Self::Discovery => "Discover",
        }
    }
    pub fn kind(self) -> &'static str {
        match self {
            Self::Movies => "Movie",
            Self::Shows => "Series",
            Self::Music => "MusicAlbum,Audio,MusicArtist,MusicVideo,Playlist",
            _ => "",
        }
    }
}

pub struct Model {
    pub session: Option<Session>,
    pub epoch: u64,
    pub section: Section,
    pub rows: Vec<Item>,
    pub detail: Option<Item>,
    pub capabilities: Option<Capabilities>,
    pub queue: MusicQueue,
    pub queue_dirty: bool,
    pub fixture: bool,
}
impl Default for Model {
    fn default() -> Self {
        Self {
            session: None,
            epoch: 0,
            section: Section::Home,
            rows: vec![],
            detail: None,
            capabilities: None,
            queue: MusicQueue::default(),
            queue_dirty: false,
            fixture: false,
        }
    }
}
impl Model {
    /// Account switch and cancellation invalidate every outstanding completion.
    pub fn clear(&mut self) {
        let epoch = self.epoch.wrapping_add(1);
        *self = Self::default();
        self.epoch = epoch;
    }
    pub fn accepts(&self, epoch: u64) -> bool {
        epoch == self.epoch
    }
    pub fn admit(&mut self, capabilities: Capabilities) -> Result<()> {
        if capabilities.protocol_version != PROTOCOL_VERSION
            || self
                .session
                .as_ref()
                .is_none_or(|s| s.user != capabilities.viewer.user_id)
        {
            return Err("Server capabilities do not match this account/protocol".into());
        }
        self.capabilities = Some(capabilities);
        Ok(())
    }
    pub fn enqueue(&mut self, item: &Item) -> Result<()> {
        if item.kind != "Audio" {
            return Err("Only audio belongs in the music queue".into());
        }
        let mut next = self.queue.queue.clone();
        next.item_ids.push(item.id);
        if !next.is_valid() {
            return Err("Queue is full".into());
        }
        self.queue.queue = next;
        self.queue_dirty = true;
        Ok(())
    }
    pub fn replace_queue(&mut self, queue: MusicQueue) -> Result<()> {
        if queue.protocol_version != PROTOCOL_VERSION || !queue.queue.is_valid() {
            return Err("Invalid server queue".into());
        }
        self.queue = queue;
        self.queue_dirty = false;
        Ok(())
    }
    pub fn fixture_rows(&self) -> Vec<Item> {
        let entries = match self.section {
            Section::Home | Section::Movies => vec![(
                1,
                "Paper moon harbor",
                "Movie",
                "Original moving test pattern with a synthesized tone. No licensed film is included.",
            )],
            Section::Shows => vec![(
                2,
                "The observatory",
                "Series",
                "An original series fixture. Open to browse episodes.",
            )],
            Section::Music => vec![
                (
                    3,
                    "Signal at dawn",
                    "Audio",
                    "An original synthesized tone. Turn down your volume.",
                ),
                (
                    6,
                    "Signal at dusk",
                    "Audio",
                    "Second original tone fixture for queue sequencing.",
                ),
            ],
            Section::Guide => vec![(
                5,
                "Night sky workshop",
                "TvChannel",
                "Original generated live test pattern and tone.",
            )],
            _ => vec![],
        };
        entries
            .into_iter()
            .map(|(id, name, kind, overview)| Item {
                id: Uuid::from_u128(id),
                name: name.into(),
                kind: kind.into(),
                overview: overview.into(),
                start_date: Some("2030-01-01T20:00:00Z".into()),
                end_date: Some("2030-01-01T21:00:00Z".into()),
                channel_name: Some("Synthetic science".into()),
                channel_id: None,
                ..Default::default()
            })
            .collect()
    }
}
/// Original PCM fixture, one second at 8 kHz, quiet 440 Hz.
pub fn tone() -> Vec<u8> {
    let mut out = Vec::new();
    let data = 16000u32;
    out.extend(b"RIFF");
    out.extend((36 + data).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(8000u32.to_le_bytes());
    out.extend(16000u32.to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(data.to_le_bytes());
    for i in 0..8000 {
        let sample = ((i as f32 * 440.0 * std::f32::consts::TAU / 8000.0).sin() * 1800.0) as i16;
        out.extend(sample.to_le_bytes());
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    use bluefin_contract::QueueState;
    #[test]
    fn original_fixture_journey_and_account_boundary() {
        let mut m = Model {
            fixture: true,
            ..Default::default()
        };
        let movie = m.fixture_rows().remove(0);
        assert!(movie.playable());
        assert!(m.enqueue(&movie).is_err());
        m.section = Section::Music;
        let song = m.fixture_rows().remove(0);
        m.enqueue(&song).unwrap();
        m.enqueue(&song).unwrap();
        assert_eq!(m.queue.queue.item_ids.len(), 2);
        let epoch = m.epoch;
        m.clear();
        assert!(!m.accepts(epoch));
        assert!(m.queue.queue.item_ids.is_empty());
        assert!(!m.fixture);
    }
    #[test]
    fn rejects_invalid_queue_and_origin() {
        let mut m = Model::default();
        let q = MusicQueue {
            queue: QueueState {
                item_ids: vec![Uuid::nil()],
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(m.replace_queue(q).is_err());
        for url in [
            "http://example.test",
            "https://u:p@example.test",
            "https://example.test/?token=x",
        ] {
            assert!(crate::protocol::server(url).is_err());
        }
        assert_eq!(
            crate::protocol::server("https://example.test/base")
                .unwrap()
                .as_str(),
            "https://example.test/base/"
        );
    }
    #[test]
    fn fixture_is_real_pcm() {
        let bytes = tone();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 16044);
        assert!(bytes[44..].iter().any(|b| *b != 0));
    }
}
