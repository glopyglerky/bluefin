//! Public discovery records carry catalog identity, never playback authority.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CatalogKind {
    Movie,
    Series,
    Person,
    Artist,
    Album,
    Recording,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum CatalogId {
    TmdbMovie(u64),
    TmdbSeries(u64),
    TmdbPerson(u64),
    MusicBrainzArtist(Uuid),
    MusicBrainzReleaseGroup(Uuid),
    MusicBrainzRecording(Uuid),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogArtwork {
    pub url: String,
    pub credit: String,
    pub source_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogCredit {
    pub id: CatalogId,
    pub name: String,
    pub role: String,
}

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogPage {
    pub results: Vec<CatalogRecord>,
    pub next_page: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CatalogAccess {
    Free,
    Subscription,
    Buy,
    Rent,
    Addon,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogOffer {
    pub provider: String,
    pub access: CatalogAccess,
    pub title_url: String,
    pub player_url: Option<String>,
    pub price: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogOffers {
    pub country: String,
    pub offers: Vec<CatalogOffer>,
}
