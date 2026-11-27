//! Public, read-only catalog access. Library permissions and request actions stay
//! in DiscoveryService; public metadata never supplies a playback entitlement.
pub mod metadata;
mod seerr;
mod settings;
use bluefin_contract::public_catalog::{CatalogFailure, CatalogPage, CatalogRecord};

use crate::{Error, Result};
use bluefin_catalog::{
    CatalogClient, CatalogIdentity, CountryCode, Coverage, CoverageScope, LookupRequest, MediaKind,
    OfferResponse, RegistryResponse, SourceObservation, SourceStatus, WatchPageClient,
    music_http::MusicBrainzClient,
};
use reqwest::{Response, StatusCode};
use std::{
    collections::HashMap,
    path::Path,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, OnceCell, Semaphore};

const OFFER_CACHE_TTL: Duration = Duration::from_secs(900);
const REGISTRY_CACHE_TTL: Duration = Duration::from_secs(3600);
const MAX_OFFER_CACHE: usize = 128;
type Cached<T> = (Instant, Arc<OnceCell<T>>);
type Cache<T> = Mutex<HashMap<String, Cached<T>>>;

/// Process-owned catalog clients, bounded caches and request concurrency.
pub struct CatalogService {
    direct: CatalogClient,
    music: MusicBrainzClient,
    seerr: Option<seerr::Seerr>,
    watch: WatchPageClient,
    public_watch_pages: bool,
    offers: Cache<OfferResponse>,
    registry: Cache<RegistryResponse>,
    network: Semaphore,
    metadata_pages: Cache<std::result::Result<CatalogPage, CatalogFailure>>,
    metadata_records: Cache<std::result::Result<CatalogRecord, CatalogFailure>>,
}

impl CatalogService {
    /// Loads optional protected upstream configuration beside discovery settings.
    pub fn open(path: &Path) -> Result<Self> {
        let settings = settings::Settings::load(path)?;
        let direct = CatalogClient::new(settings.credentials()).map_err(|_| Error::Upstream)?;
        let music = MusicBrainzClient::new().map_err(|_| Error::Upstream)?;
        let seerr = seerr::Seerr::from_settings(&settings)?;
        let watch = WatchPageClient::new().map_err(|_| Error::Upstream)?;
        Ok(Self {
            direct,
            music,
            seerr,
            watch,
            public_watch_pages: !settings.disable_public_watch_pages,
            offers: Mutex::new(HashMap::new()),
            registry: Mutex::new(HashMap::new()),
            network: Semaphore::new(4),
            metadata_pages: Mutex::new(HashMap::new()),
            metadata_records: Mutex::new(HashMap::new()),
        })
    }

    /// Resolves source-backed offers for a typed identity and explicit region.
    /// A missing country retains all regions reported by upstream APIs.
    pub async fn lookup(&self, mut request: LookupRequest) -> Result<OfferResponse> {
        validate(&request)?;
        request.identity.ids.sort();
        request.identity.ids.dedup();
        let key = serde_json::to_string(&request).map_err(|_| Error::Invalid)?;
        let cell = cache_entry(&self.offers, &key, OFFER_CACHE_TTL, MAX_OFFER_CACHE).await;
        let result = cell
            .get_or_init(|| async {
                match tokio::time::timeout(Duration::from_secs(3), self.network.acquire()).await {
                    Ok(Ok(_permit)) => self.uncached_lookup(&request).await,
                    _ => OfferResponse::with_status(
                        &request,
                        "catalog",
                        SourceStatus::RateLimited,
                        CoverageScope::RegionalAvailability,
                        timestamp(),
                    ),
                }
            })
            .await
            .clone();
        if result.sources.iter().any(|source| {
            matches!(
                source.status,
                SourceStatus::Unavailable
                    | SourceStatus::RateLimited
                    | SourceStatus::InvalidResponse
            )
        }) || !result.sources.iter().any(|source| {
            matches!(
                source.status,
                SourceStatus::Complete | SourceStatus::Partial | SourceStatus::NoOffers
            )
        }) {
            self.offers.lock().await.remove(&key);
        }
        Ok(result)
    }

    async fn uncached_lookup(&self, request: &LookupRequest) -> OfferResponse {
        match request.identity.kind {
            MediaKind::Artist
            | MediaKind::Release
            | MediaKind::ReleaseGroup
            | MediaKind::Recording
            | MediaKind::Work
            | MediaKind::MusicVideo => self.music.lookup(request).await,
            MediaKind::Movie | MediaKind::Series | MediaKind::Season | MediaKind::Episode => {
                let (mut result, summary, page) = tokio::join!(
                    self.direct.lookup(request),
                    self.seerr_offers(request),
                    self.watch_page(request)
                );
                if let Some(summary) = summary {
                    result.merge(summary);
                }
                if let Some(page) = page {
                    result.merge(page);
                }
                if result
                    .sources
                    .iter()
                    .any(|source| source.status == SourceStatus::IdentityMismatch)
                {
                    result.offers.clear();
                }
                remove_replaced_placeholders(&mut result);
                result
            }
            _ => OfferResponse::with_status(
                request,
                "catalog",
                SourceStatus::UnsupportedIdentity,
                CoverageScope::LinkedAccess,
                timestamp(),
            ),
        }
    }

    async fn seerr_offers(&self, request: &LookupRequest) -> Option<OfferResponse> {
        match &self.seerr {
            Some(seerr) => Some(seerr.lookup(request).await),
            None => None,
        }
    }

    async fn watch_page(&self, request: &LookupRequest) -> Option<OfferResponse> {
        if self.public_watch_pages {
            Some(self.watch.lookup(request).await)
        } else {
            None
        }
    }

    /// Supplements an unconfigured hosted registry with the requested title's
    /// public source country dataset. No private service is required.
    pub async fn providers_for(
        &self,
        country: Option<CountryCode>,
        identity: Option<CatalogIdentity>,
    ) -> Result<RegistryResponse> {
        let mut result = self.providers(country.clone()).await?;
        if result.countries.is_empty() && self.public_watch_pages {
            if let Some(identity) = identity {
                let request = LookupRequest {
                    identity,
                    country: country.clone(),
                };
                validate(&request)?;
                let mut observed = self.watch.countries(&request).await;
                if let Some(country) = country {
                    observed.countries.retain(|entry| entry.code == country);
                }
                result.merge(observed);
            }
        }
        Ok(result)
    }

    /// Returns current source-reported countries and services, refreshed hourly.
    /// With no country, sources may return countries alone; a selected country
    /// supplies its dynamic service list without a compile-time provider list.
    pub async fn providers(&self, country: Option<CountryCode>) -> Result<RegistryResponse> {
        let key = country
            .as_ref()
            .map_or_else(|| "*".into(), ToString::to_string);
        let cell = cache_entry(&self.registry, &key, REGISTRY_CACHE_TTL, 300).await;
        let result = cell
            .get_or_init(|| async {
                let Ok(Ok(_permit)) =
                    tokio::time::timeout(Duration::from_secs(3), self.network.acquire()).await
                else {
                    return RegistryResponse {
                        sources: vec![observation(
                            "catalog",
                            SourceStatus::RateLimited,
                            Coverage::Unknown,
                            timestamp(),
                        )],
                        ..RegistryResponse::default()
                    };
                };
                let (mut result, local) =
                    tokio::join!(self.direct.registry(country.as_ref()), async {
                        match &self.seerr {
                            Some(seerr) => Some(seerr.registry(country.as_ref()).await),
                            None => None,
                        }
                    });
                if let Some(local) = local {
                    result.merge(local);
                }
                result
            })
            .await
            .clone();
        if result.countries.is_empty() {
            self.registry.lock().await.remove(&key);
        }
        Ok(result)
    }
}

async fn cache_entry<T>(
    cache: &Cache<T>,
    key: &str,
    ttl: Duration,
    maximum: usize,
) -> Arc<OnceCell<T>> {
    let mut cache = cache.lock().await;
    cache.retain(|_, (created, _)| created.elapsed() < ttl);
    if let Some((_, cell)) = cache.get(key) {
        return Arc::clone(cell);
    }
    if cache.len() >= maximum {
        let oldest = cache
            .iter()
            .min_by_key(|(_, (created, _))| *created)
            .map(|(key, _)| key.clone());
        if let Some(oldest) = oldest {
            cache.remove(&oldest);
        }
    }
    let cell = Arc::new(OnceCell::new());
    cache.insert(key.to_owned(), (Instant::now(), Arc::clone(&cell)));
    cell
}
fn validate(request: &LookupRequest) -> Result<()> {
    if request.identity.ids.is_empty()
        || request.identity.ids.len() > 12
        || request.identity.ids.iter().any(|id| {
            id.source.is_empty()
                || id.source.len() > 64
                || !id
                    .source
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                || id.id.is_empty()
                || id.id.len() > 256
                || id.id.chars().any(|c| c.is_control() || c.is_whitespace())
        })
    {
        return Err(Error::Invalid);
    }
    let mut seen = HashMap::new();
    for id in &request.identity.ids {
        if seen
            .insert(id.source.to_ascii_lowercase(), &id.id)
            .is_some_and(|previous| previous != &id.id)
        {
            return Err(Error::Invalid);
        }
    }
    Ok(())
}
fn remove_replaced_placeholders(result: &mut OfferResponse) {
    let actionable: Vec<_> = result
        .offers
        .iter()
        .filter(|offer| !offer.destinations.is_empty())
        .map(|offer| {
            (
                offer.country.clone(),
                offer.service.source.clone(),
                offer.service.id.clone(),
                offer.access_type,
                offer.raw_access_type.clone(),
                offer
                    .addon
                    .as_ref()
                    .map(|addon| (addon.source.clone(), addon.id.clone())),
            )
        })
        .collect();
    result.offers.retain(|offer| {
        !offer.destinations.is_empty()
            || !actionable
                .iter()
                .any(|(country, source, id, access, raw, addon)| {
                    country == &offer.country
                        && source == &offer.service.source
                        && id == &offer.service.id
                        && access == &offer.access_type
                        && raw == &offer.raw_access_type
                        && addon
                            .as_ref()
                            .map(|(source, id)| (source.as_str(), id.as_str()))
                            == offer
                                .addon
                                .as_ref()
                                .map(|addon| (addon.source.as_str(), addon.id.as_str()))
                })
    });
}
fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn observation(
    source: &str,
    status: SourceStatus,
    coverage: Coverage,
    now: u64,
) -> SourceObservation {
    SourceObservation {
        source: source.into(),
        status,
        observed_at: now,
        scope: CoverageScope::RegionalAvailability,
        coverage,
        catalog_url: None,
        attributions: Vec::new(),
    }
}
fn http_status(status: StatusCode) -> SourceStatus {
    match status.as_u16() {
        401 | 403 => SourceStatus::Denied,
        404 => SourceStatus::NotFound,
        429 => SourceStatus::RateLimited,
        300..=399 | 400 | 422 => SourceStatus::InvalidResponse,
        _ => SourceStatus::Unavailable,
    }
}
async fn bounded_bytes(
    mut response: Response,
    limit: usize,
) -> std::result::Result<Vec<u8>, SourceStatus> {
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(SourceStatus::InvalidResponse);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| SourceStatus::Unavailable)?
    {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err(SourceStatus::InvalidResponse);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    use bluefin_catalog::{CatalogId, CatalogIdentity};
    #[test]
    fn ambiguous_ids_and_unbounded_requests_are_rejected_before_network() {
        let mut request = LookupRequest {
            identity: CatalogIdentity {
                kind: MediaKind::Movie,
                ids: vec![CatalogId {
                    source: "tmdb".into(),
                    id: "movie/278".into(),
                }],
            },
            country: None,
        };
        assert!(validate(&request).is_ok());
        request.identity.ids.push(CatalogId {
            source: "TMDB".into(),
            id: "movie/603".into(),
        });
        assert!(validate(&request).is_err());
        request.identity.ids.clear();
        assert!(validate(&request).is_err());
    }
    #[tokio::test]
    async fn identical_concurrent_requests_share_one_cell_and_expire() {
        let cache = Mutex::new(HashMap::<String, Cached<u32>>::new());
        let a = cache_entry(&cache, "same", Duration::from_secs(60), 2).await;
        let b = cache_entry(&cache, "same", Duration::from_secs(60), 2).await;
        assert!(Arc::ptr_eq(&a, &b));
        let c = cache_entry(&cache, "same", Duration::ZERO, 2).await;
        assert!(!Arc::ptr_eq(&a, &c));
    }
}
