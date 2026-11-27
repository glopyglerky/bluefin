//! Public metadata records carry catalog identity, never library or playback authority.
use bluefin_catalog::{SourceObservation, SourceStatus};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Metadata entity type. Album identifies a MusicBrainz release group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CatalogKind {
    Movie,
    Series,
    Person,
    Artist,
    Album,
    Release,
    Recording,
    MusicVideo,
    Work,
}

/// Stable provider identity, preserving the specialist catalog wire contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum CatalogId {
    TmdbMovie(u64),
    TmdbSeries(u64),
    TmdbPerson(u64),
    MusicBrainzArtist(Uuid),
    MusicBrainzReleaseGroup(Uuid),
    MusicBrainzRelease(Uuid),
    MusicBrainzRecording(Uuid),
    MusicBrainzWork(Uuid),
}

/// Source-owned artwork, with the attribution needed by each presentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogArtwork {
    pub url: String,
    pub credit: String,
    pub source_url: String,
}

/// A person credit or related work that can open another catalog detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogCredit {
    pub id: CatalogId,
    pub name: String,
    pub role: String,
    #[serde(default)]
    pub artwork: Option<CatalogArtwork>,
    #[serde(default)]
    pub year: Option<i32>,
}

/// Whitelisted public metadata. Private request/watchlist/media state is never represented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogRecord {
    pub id: CatalogId,
    pub title: String,
    pub year: Option<i32>,
    pub summary: Option<String>,
    pub artwork: Option<CatalogArtwork>,
    pub imdb_id: Option<String>,
    pub credits: Vec<CatalogCredit>,
    #[serde(default)]
    pub backdrop: Option<CatalogArtwork>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub credits_truncated: bool,
    #[serde(default)]
    pub sources: Vec<SourceObservation>,
}

/// One source page. Missing totals remain unknown rather than counting filtered rows as the whole catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogPage {
    pub results: Vec<CatalogRecord>,
    pub next_page: Option<u32>,
    #[serde(default)]
    pub total_results: Option<u64>,
    #[serde(default)]
    pub sources: Vec<SourceObservation>,
}

/// Structured source failure, separate from a successful empty search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFailure {
    pub error: String,
    pub sources: Vec<SourceObservation>,
}
impl CatalogFailure {
    /// The last attempted source determines the failure's transport status.
    pub fn status(&self) -> SourceStatus {
        self.sources
            .last()
            .map(|source| source.status)
            .unwrap_or(SourceStatus::Unavailable)
    }
}
