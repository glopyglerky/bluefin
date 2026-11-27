//! Adapts existing configured metadata clients to the shared public catalog.
use std::sync::Arc;

use bluefin_contract::public_catalog::CatalogKind;
use bluefin_discovery::{
    catalog::metadata::{MetadataDocument, MetadataFuture, MetadataReader},
    catalog_types::{CatalogIdentity, MediaKind, SourceStatus},
};
use ferrofin_providers::{
    MusicBrainzClient, TmdbClient, musicbrainz::CatalogMusicEntity,
    public_catalog::CatalogFetchError, tmdb::CatalogVideoEntity,
};
use uuid::Uuid;

pub(crate) struct SpecialistCatalog {
    musicbrainz: Arc<MusicBrainzClient>,
    tmdb: Arc<TmdbClient>,
}

impl SpecialistCatalog {
    pub(crate) fn new(musicbrainz: Arc<MusicBrainzClient>, tmdb: Arc<TmdbClient>) -> Self {
        Self { musicbrainz, tmdb }
    }
}

impl MetadataReader for SpecialistCatalog {
    fn search<'a>(&'a self, kind: CatalogKind, query: &'a str, page: u32) -> MetadataFuture<'a> {
        Box::pin(async move {
            if !(1..=100).contains(&page) {
                return MetadataDocument::failure(SourceStatus::UnsupportedIdentity);
            }
            let result = match kind {
                CatalogKind::Movie => self.tmdb.catalog_search_result(CatalogVideoEntity::Movie, query, page).await,
                CatalogKind::Series => self.tmdb.catalog_search_result(CatalogVideoEntity::Series, query, page).await,
                CatalogKind::Person => self.tmdb.catalog_search_result(CatalogVideoEntity::Person, query, page).await,
                CatalogKind::Artist => self.musicbrainz.catalog_search_result(CatalogMusicEntity::Artist, query, (page - 1) * 20).await,
                CatalogKind::Album => self.musicbrainz.catalog_search_result(CatalogMusicEntity::ReleaseGroup, query, (page - 1) * 20).await,
                CatalogKind::Recording => self.musicbrainz.catalog_search_result(CatalogMusicEntity::Recording, query, (page - 1) * 20).await,
                _ => Err(CatalogFetchError::UnsupportedIdentity),
            };
            document(result)
        })
    }

    fn details<'a>(&'a self, identity: &'a CatalogIdentity) -> MetadataFuture<'a> {
        Box::pin(async move {
            let video = match identity.kind {
                MediaKind::Movie => Some((CatalogVideoEntity::Movie, "movie")),
                MediaKind::Series => Some((CatalogVideoEntity::Series, "tv")),
                MediaKind::Person => Some((CatalogVideoEntity::Person, "person")),
                _ => None,
            };
            if let Some((entity, resource)) = video {
                let Some(id) = identity.id("tmdb")
                    .and_then(|value| value.strip_prefix(&format!("{resource}/")))
                    .and_then(|value| value.parse::<u64>().ok())
                    .filter(|id| *id > 0)
                else {
                    return MetadataDocument::failure(SourceStatus::UnsupportedIdentity);
                };
                return document(self.tmdb.catalog_details_result(entity, id).await);
            }
            let entity = match identity.kind {
                MediaKind::Artist => CatalogMusicEntity::Artist,
                MediaKind::ReleaseGroup => CatalogMusicEntity::ReleaseGroup,
                MediaKind::Release => CatalogMusicEntity::Release,
                MediaKind::Recording | MediaKind::MusicVideo => CatalogMusicEntity::Recording,
                MediaKind::Work => CatalogMusicEntity::Work,
                _ => return MetadataDocument::failure(SourceStatus::UnsupportedIdentity),
            };
            let Some(id) = identity.id("musicbrainz")
                .and_then(|value| Uuid::parse_str(value).ok())
                .filter(|id| !id.is_nil())
            else {
                return MetadataDocument::failure(SourceStatus::UnsupportedIdentity);
            };
            document(self.musicbrainz.catalog_details_result(entity, id).await)
        })
    }
}

fn document(result: Result<serde_json::Value, CatalogFetchError>) -> MetadataDocument {
    match result {
        Ok(body) => MetadataDocument {
            body: Some(body),
            observed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            status: SourceStatus::Complete,
        },
        Err(error) => MetadataDocument::failure(match error {
            CatalogFetchError::Unavailable => SourceStatus::Unavailable,
            CatalogFetchError::RateLimited => SourceStatus::RateLimited,
            CatalogFetchError::Denied => SourceStatus::Denied,
            CatalogFetchError::NotFound => SourceStatus::NotFound,
            CatalogFetchError::InvalidResponse => SourceStatus::InvalidResponse,
            CatalogFetchError::UnsupportedIdentity => SourceStatus::UnsupportedIdentity,
        }),
    }
}
