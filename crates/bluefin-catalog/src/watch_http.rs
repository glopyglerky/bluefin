//! Public watch-page acquisition for standalone clients and hosted catalog services.
//! No private server address, service credential or library pairing participates.
use alloc::{format, string::{String, ToString}, sync::Arc, vec, vec::Vec};
use std::{collections::BTreeMap, sync::OnceLock, time::{Duration, Instant}};
use reqwest::{Client, StatusCode, header::RETRY_AFTER};
use tokio::sync::{Mutex, OnceCell, Semaphore};
use crate::{*, http::observed_at, tmdb_links, video};

const SOURCE: &str = "tmdb-watch-page";
const MAX_HTML_BYTES: usize = 2_500_000;
const MAX_CACHED_PAGES: usize = 8;
const PAGE_TTL: Duration = Duration::from_secs(300);
const REQUEST_SPACING: Duration = Duration::from_millis(750);

#[derive(Clone)]
struct Page { url: String, html: Arc<str>, observed_at: u64 }
#[derive(Clone, Copy)]
struct Failure { status: SourceStatus, observed_at: u64 }
type PageCell = Arc<OnceCell<Result<Page, Failure>>>;
struct Gate {
    next: Instant,
    blocked: Option<(Instant, Failure)>,
}
struct Shared {
    cache: Mutex<BTreeMap<String, (Instant, PageCell)>>,
    gate: Mutex<Gate>,
    network: Semaphore,
}
fn shared() -> &'static Shared {
    static SHARED: OnceLock<Shared> = OnceLock::new();
    SHARED.get_or_init(|| Shared {
        cache: Mutex::new(BTreeMap::new()),
        gate: Mutex::new(Gate { next: Instant::now(), blocked: None }),
        network: Semaphore::new(4),
    })
}

/// Credential-free exact-title acquisition from the public TMDB watch page.
/// Instances share an in-process response cache, request spacing and rate-limit cooldown.
#[derive(Clone, Debug)]
pub struct WatchPageClient { client: Client }
impl WatchPageClient {
    /// Builds an honest fixed-origin client; redirects can remain only on TMDB HTTPS.
    pub fn new() -> Result<Self, reqwest::Error> {
        let client = Client::builder().no_proxy()
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() < 3 && trusted_source_url(attempt.url()) {
                    attempt.follow()
                } else { attempt.stop() }
            }))
            .timeout(Duration::from_secs(15)).connect_timeout(Duration::from_secs(4))
            .user_agent(concat!("Bluefin-Catalog/", env!("CARGO_PKG_VERSION"), " (+https://github.com/glopyglerky/bluefin)"))
            .build()?;
        Ok(Self { client })
    }

    /// Fetches an exact movie/series in the explicitly selected country.
    /// None remains unknown/partial; it does not select a country from IP or a default storefront.
    pub async fn lookup(&self, request: &LookupRequest) -> OfferResponse {
        let now = observed_at();
        if let Err(status) = validate(request) { return offer_failure(request, Failure { status, observed_at: now }); }
        let Some(country) = request.country.as_ref() else {
            return offer_failure(request, Failure { status: SourceStatus::Partial, observed_at: now });
        };
        let url = match request_url(request) {
            Ok(url) => url, Err(status) => return offer_failure(request, Failure { status, observed_at: now }),
        };
        let page = match self.page(&url).await {
            Ok(page) => page, Err(failure) => return offer_failure(request, failure),
        };
        // The public picker establishes supported choices without a hardcoded country list.
        let countries = tmdb_links::project_tmdb_watch_page_countries(request, &page.url, &page.html, page.observed_at);
        if !countries.countries.is_empty() && !countries.countries.iter().any(|region| &region.code == country) {
            return offer_failure(request, Failure { status: SourceStatus::UnsupportedCountry, observed_at: page.observed_at });
        }
        let result = tmdb_links::project_tmdb_watch_page(request, &page.url, &page.html, page.observed_at);
        if !offers_cacheable(&result) { discard(&url).await; }
        result
    }

    /// Returns the source's current country picker for this exact title page.
    /// This lists supported choices only: empty service lists do not imply no providers.
    /// With no country, only the registry is read; the site's geographic default never becomes an offer.
    pub async fn countries(&self, request: &LookupRequest) -> RegistryResponse {
        let now = observed_at();
        if let Err(status) = validate(request) { return registry_failure(Failure { status, observed_at: now }); }
        let url = match request_url(request) {
            Ok(url) => url, Err(status) => return registry_failure(Failure { status, observed_at: now }),
        };
        let page = match self.page(&url).await {
            Ok(page) => page, Err(failure) => return registry_failure(failure),
        };
        let result = tmdb_links::project_tmdb_watch_page_countries(request, &page.url, &page.html, page.observed_at);
        if result.countries.is_empty() { discard(&url).await; }
        result
    }

    async fn page(&self, url: &str) -> Result<Page, Failure> {
        let cell = {
            let mut cache = shared().cache.lock().await;
            cache.retain(|_, (created, _)| created.elapsed() < PAGE_TTL);
            if let Some((_, cell)) = cache.get(url) { Arc::clone(cell) }
            else {
                if cache.len() >= MAX_CACHED_PAGES {
                    let oldest = cache.iter().min_by_key(|(_, (created, _))| *created)
                        .map(|(key, _)| key.clone());
                    if let Some(oldest) = oldest { cache.remove(&oldest); }
                }
                let cell = Arc::new(OnceCell::new());
                cache.insert(url.into(), (Instant::now(), Arc::clone(&cell)));
                cell
            }
        };
        let result = cell.get_or_init(|| self.download(url)).await.clone();
        // Failures remain failures; Retry-After is handled separately from positive evidence caching.
        if result.is_err() { discard(url).await; }
        result
    }

    async fn download(&self, url: &str) -> Result<Page, Failure> {
        let fail = |status| Failure { status, observed_at: observed_at() };
        let _permit = match tokio::time::timeout(Duration::from_secs(3), shared().network.acquire()).await {
            Ok(Ok(permit)) => permit, _ => return Err(fail(SourceStatus::RateLimited)),
        };
        {
            let mut gate = shared().gate.lock().await;
            if let Some((until, failure)) = gate.blocked {
                if until > Instant::now() { return Err(failure); }
                gate.blocked = None;
            }
            if gate.next > Instant::now() { tokio::time::sleep_until(gate.next.into()).await; }
            gate.next = Instant::now() + REQUEST_SPACING;
        }
        let mut response = self.client.get(url).header("Accept", "text/html")
            .send().await.map_err(|_| fail(SourceStatus::Unavailable))?;
        if let Some(status) = status_error(response.status()) {
            let failure = fail(status);
            if matches!(status, SourceStatus::RateLimited | SourceStatus::Unavailable) {
                let seconds = response.headers().get(RETRY_AFTER).and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok()).map(|seconds| seconds.clamp(1, 900))
                    .unwrap_or(if status == SourceStatus::RateLimited { 60 } else { 15 });
                shared().gate.lock().await.blocked = Some((Instant::now() + Duration::from_secs(seconds), failure));
            }
            return Err(failure);
        }
        if !trusted_source_url(response.url()) { return Err(fail(SourceStatus::InvalidResponse)); }
        if response.content_length().is_some_and(|length| length > MAX_HTML_BYTES as u64) {
            return Err(fail(SourceStatus::InvalidResponse));
        }
        let final_url = response.url().to_string();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| fail(SourceStatus::Unavailable))? {
            if bytes.len().saturating_add(chunk.len()) > MAX_HTML_BYTES { return Err(fail(SourceStatus::InvalidResponse)); }
            bytes.extend_from_slice(&chunk);
        }
        let html = String::from_utf8(bytes).map_err(|_| fail(SourceStatus::InvalidResponse))?;
        Ok(Page { url: final_url, html: Arc::from(html), observed_at: observed_at() })
    }
}

fn validate(request: &LookupRequest) -> Result<(), SourceStatus> {
    if request.identity.ids.is_empty() || request.identity.ids.len() > 12 ||
        request.identity.ids.iter().any(|id| id.source.is_empty() || id.source.len() > 64 ||
            id.id.is_empty() || id.id.len() > 256 || id.source.chars().chain(id.id.chars()).any(char::is_control)) {
        return Err(SourceStatus::UnsupportedIdentity);
    }
    video::tmdb_key(&request.identity).map(|_| ())
}
fn request_url(request: &LookupRequest) -> Result<String, SourceStatus> {
    let key = video::tmdb_key(&request.identity)?;
    Ok(match &request.country {
        Some(country) => format!("https://www.themoviedb.org/{key}/watch?locale={country}"),
        None => format!("https://www.themoviedb.org/{key}/watch"),
    })
}
fn trusted_source_url(url: &url::Url) -> bool {
    url.scheme() == "https" && matches!(url.host_str(), Some("www.themoviedb.org" | "themoviedb.org")) &&
        url.username().is_empty() && url.password().is_none() && url.port().is_none_or(|port| port == 443)
}
fn status_error(status: StatusCode) -> Option<SourceStatus> {
    match status.as_u16() {
        200..=299 => None,
        401 | 403 => Some(SourceStatus::Denied),
        404 | 410 => Some(SourceStatus::NotFound),
        429 => Some(SourceStatus::RateLimited),
        500..=599 => Some(SourceStatus::Unavailable),
        _ => Some(SourceStatus::InvalidResponse),
    }
}
fn offers_cacheable(response: &OfferResponse) -> bool {
    response.sources.iter().any(|source| source.status == SourceStatus::NoOffers ||
        (!response.offers.is_empty() && matches!(source.status, SourceStatus::Complete | SourceStatus::Partial)))
}
async fn discard(url: &str) { shared().cache.lock().await.remove(url); }
fn observation(failure: Failure) -> SourceObservation {
    SourceObservation { source: SOURCE.into(), status: failure.status, observed_at: failure.observed_at,
        scope: CoverageScope::RegionalAvailability, coverage: Coverage::Unknown, catalog_url: None,
        attributions: vec![
            Attribution { name: "TMDB".into(), url: "https://www.themoviedb.org/".into() },
            Attribution { name: "JustWatch".into(), url: "https://www.justwatch.com/".into() },
        ] }
}
fn offer_failure(request: &LookupRequest, failure: Failure) -> OfferResponse {
    let mut result = OfferResponse::empty(request);
    result.sources.push(observation(failure));
    result
}
fn registry_failure(failure: Failure) -> RegistryResponse {
    RegistryResponse { schema_version: 1, countries: Vec::new(), sources: vec![observation(failure)] }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(country: Option<&str>) -> LookupRequest {
        LookupRequest { identity: CatalogIdentity { kind: MediaKind::Movie,
            ids: vec![CatalogId { source: "tmdb".into(), id: "movie/10331".into() }] },
            country: country.map(|country| CountryCode::parse(country).unwrap()) }
    }
    #[tokio::test]
    async fn no_country_never_requests_an_ip_selected_storefront() {
        let client = WatchPageClient::new().unwrap();
        let response = client.lookup(&request(None)).await;
        assert!(response.offers.is_empty());
        assert!(response.country.is_none());
        assert_eq!(response.sources[0].status, SourceStatus::Partial);
        assert_eq!(response.sources[0].coverage, Coverage::Unknown);
    }
    #[test]
    fn source_urls_are_exact_and_redirects_cannot_leave_tmdb() {
        assert_eq!(request_url(&request(Some("GB"))).unwrap(),
            "https://www.themoviedb.org/movie/10331/watch?locale=GB");
        for bad in ["http://www.themoviedb.org/movie/10331/watch", "https://example.com/",
            "https://user:secret@www.themoviedb.org/", "https://www.themoviedb.org:8443/"] {
            assert!(!trusted_source_url(&url::Url::parse(bad).unwrap()));
        }
        assert!(trusted_source_url(&url::Url::parse("https://www.themoviedb.org/movie/10331-title/watch?locale=GB").unwrap()));
    }
    #[test]
    fn transient_or_unparsed_empty_results_are_not_positive_cache_entries() {
        let request = request(Some("US"));
        for status in [SourceStatus::Unavailable, SourceStatus::RateLimited, SourceStatus::Denied,
            SourceStatus::InvalidResponse, SourceStatus::Partial] {
            let response = offer_failure(&request, Failure { status, observed_at: 10 });
            assert!(!offers_cacheable(&response));
        }
        assert!(offers_cacheable(&offer_failure(&request,
            Failure { status: SourceStatus::NoOffers, observed_at: 10 })));
    }
}
