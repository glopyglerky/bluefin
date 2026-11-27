//! Album covers resolved only from stored MusicBrainz identifiers.
//!
//! Contract: <https://musicbrainz.org/doc/Cover_Art_Archive/API>.
//! An exact release never falls back to a different edition. Only an album
//! with no release identifier may use its release group's chosen cover.
//! Approved front images are offered through CAA's 1200px endpoint; arbitrary
//! image URLs in the response are ignored. No title or artist search occurs.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use ferrofin_model::entities::ImageType;
use reqwest::Url;
use serde::Deserialize;
use uuid::Uuid;

use crate::destination::is_private_address;
use crate::guarded_http::{FetchKind, Fetched, GuardedHttp, SendError};
use crate::rate_limit::CappedBody;
use crate::tmdb::TmdbImage;

/// Display name used by both the scanner and the image chooser.
pub const PROVIDER_NAME: &str = "Cover Art Archive";
const ORIGIN: &str = "https://coverartarchive.org";
const JSON_LIMIT: u64 = 1024 * 1024;
/// The provider uses thumbnails, never an unbounded original scan.
pub(crate) const IMAGE_LIMIT: u64 = 8 * 1024 * 1024;
const CACHE_ENTRIES: usize = 256;
const HIT_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const MISS_TTL: Duration = Duration::from_secs(15 * 60);
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(25);

static FETCH: FetchKind = FetchKind::new(
    "cover_art_archive",
    "album cover",
    "album cover lookups",
    "the destination is a public Cover Art Archive or Internet Archive host",
    Some(concat!(
        "Bluefin/",
        env!("CARGO_PKG_VERSION"),
        " (https://github.com/mangoleaf/ferrofin)"
    )),
);

/// HTTPS only, with no credentials or nonstandard port, at the two official
/// services used by CAA redirects. The resolver separately refuses private IPs.
fn allowed_hop(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.host_str().is_some_and(|host| {
            host == "coverartarchive.org" || host == "archive.org" || host.ends_with(".archive.org")
        })
}

/// Whether the normal image downloader must apply CAA's stricter hop and size
/// policy. Matching the host regardless of scheme also refuses HTTP downgrades.
pub(crate) fn is_archive_url(url: &str) -> bool {
    Url::parse(url)
        .ok()
        .is_some_and(|url| url.host_str() == Some("coverartarchive.org"))
}

fn public_http() -> GuardedHttp {
    GuardedHttp::with_environment_proxy(Arc::new(is_private_address as fn(IpAddr) -> bool), &FETCH)
}

/// The image downloader shares the same HTTPS/public-destination policy as
/// metadata lookups. No account token is sent to the archive.
pub(crate) async fn send_image(url: &str) -> Result<Fetched, SendError> {
    static HTTP: OnceLock<GuardedHttp> = OnceLock::new();
    tokio::time::timeout(
        LOOKUP_TIMEOUT,
        HTTP.get_or_init(public_http).send_checked(url, allowed_hop),
    )
    .await
    .map_err(|_| SendError::Failed("album cover redirect deadline"))?
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Lookup {
    kind: &'static str,
    id: Uuid,
}

impl Lookup {
    fn from_ids(release: Option<&str>, group: Option<&str>) -> Option<Self> {
        // A malformed but present release ID must not pick another edition.
        let (kind, value) = match release.filter(|id| !id.trim().is_empty()) {
            Some(id) => ("release", id),
            None => ("release-group", group?),
        };
        let id = Uuid::parse_str(value.trim())
            .ok()
            .filter(|id| !id.is_nil())?;
        Some(Self { kind, id })
    }
}

struct Cached {
    until: Instant,
    images: Vec<TmdbImage>,
}

/// Shared scanner/image-chooser client. The bounded memory cache contains only
/// public cover metadata; downloaded image bytes use the existing image store.
pub struct CoverArtArchiveClient {
    http: GuardedHttp,
    cache: Mutex<HashMap<Lookup, Cached>>,
    pacing: tokio::sync::Mutex<Instant>,
    admission: tokio::sync::Semaphore,
    #[cfg(any(test, feature = "test-util"))]
    test_origin: Option<Url>,
}

impl std::fmt::Debug for CoverArtArchiveClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoverArtArchiveClient")
            .finish_non_exhaustive()
    }
}

impl Default for CoverArtArchiveClient {
    fn default() -> Self {
        Self::new()
    }
}

impl CoverArtArchiveClient {
    /// Uses the official public archive, without credentials.
    #[must_use]
    pub fn new() -> Self {
        Self {
            http: public_http(),
            cache: Mutex::new(HashMap::new()),
            pacing: tokio::sync::Mutex::new(Instant::now()),
            admission: tokio::sync::Semaphore::new(8),
            #[cfg(any(test, feature = "test-util"))]
            test_origin: None,
        }
    }

    /// A loopback-only test transport. This method is absent from production.
    #[cfg(any(test, feature = "test-util"))]
    #[must_use]
    pub fn for_test(origin: &str) -> Self {
        let url = Url::parse(origin).expect("test origin");
        assert!(url.scheme() == "http");
        assert!(
            url.host_str()
                .and_then(|host| host.parse::<IpAddr>().ok())
                .is_some_and(|ip| ip.is_loopback())
        );
        Self {
            http: GuardedHttp::new(
                Arc::new((|_: IpAddr| false) as fn(IpAddr) -> bool),
                false,
                &FETCH,
            ),
            test_origin: Some(url),
            ..Self::new()
        }
    }

    fn origin(&self) -> &str {
        #[cfg(any(test, feature = "test-util"))]
        if let Some(origin) = &self.test_origin {
            return origin.as_str().trim_end_matches('/');
        }
        ORIGIN
    }

    fn allows(&self, url: &Url) -> bool {
        #[cfg(any(test, feature = "test-util"))]
        if let Some(origin) = &self.test_origin {
            return url.origin() == origin.origin();
        }
        allowed_hop(url)
    }

    fn cached(&self, key: &Lookup) -> Option<Vec<TmdbImage>> {
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .filter(|cached| cached.until > Instant::now())
            .map(|cached| cached.images.clone())
    }

    /// Approved Primary candidates for these stored IDs. A miss, bad document
    /// or unavailable provider supplies no image and never guesses a match.
    pub async fn album_images(&self, release: Option<&str>, group: Option<&str>) -> Vec<TmdbImage> {
        let Some(key) = Lookup::from_ids(release, group) else {
            return Vec::new();
        };
        if let Some(cached) = self.cached(&key) {
            return cached;
        }
        // Bound active and waiting requests. Excess work is retried by the next
        // normal refresh instead of building an unbounded provider queue.
        let Ok(_permit) = self.admission.try_acquire() else {
            return Vec::new();
        };
        tokio::time::timeout(LOOKUP_TIMEOUT, self.lookup_and_cache(key))
            .await
            .unwrap_or_default()
    }

    async fn lookup_and_cache(&self, key: Lookup) -> Vec<TmdbImage> {
        // The total deadline includes waiting for this gate and request pacing.
        // Keep the gate until publication so identical requests share one fetch.
        let mut next = self.pacing.lock().await;
        if let Some(cached) = self.cached(&key) {
            return cached;
        }
        if *next > Instant::now() {
            tokio::time::sleep_until((*next).into()).await;
        }
        *next = Instant::now() + Duration::from_secs(1);
        let Some(images) = self.fetch(&key).await else {
            return Vec::new();
        };
        let ttl = if images.is_empty() { MISS_TTL } else { HIT_TTL };
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.retain(|_, cached| cached.until > Instant::now());
        if cache.len() >= CACHE_ENTRIES
            && let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, cached)| cached.until)
                .map(|(key, _)| key.clone())
        {
            cache.remove(&oldest);
        }
        cache.insert(
            key,
            Cached {
                until: Instant::now() + ttl,
                images: images.clone(),
            },
        );
        images
    }

    async fn fetch(&self, key: &Lookup) -> Option<Vec<TmdbImage>> {
        let url = format!("{}/{}/{}", self.origin(), key.kind, key.id);
        let fetched = self
            .http
            .send_checked(&url, |url| self.allows(url))
            .await
            .ok()?;
        if fetched.response.status() == reqwest::StatusCode::NOT_FOUND {
            return Some(Vec::new());
        }
        if !fetched.response.status().is_success() {
            return None;
        }
        let bytes = CappedBody::new(fetched.response, JSON_LIMIT)
            .ok()?
            .read_to_end(Vec::new())
            .await
            .ok()?;
        let index: Index = serde_json::from_slice(&bytes).ok()?;
        let images = candidates(key, index)?;
        // The fixture owns its local image route too. Production has no override.
        #[cfg(any(test, feature = "test-util"))]
        let images = if self.test_origin.is_some() {
            images
                .into_iter()
                .map(|mut image| {
                    image.url = image.url.replacen(ORIGIN, self.origin(), 1);
                    image
                })
                .collect()
        } else {
            images
        };
        Some(images)
    }
}

#[derive(Deserialize)]
struct Index {
    release: String,
    images: Vec<Picture>,
}

#[derive(Deserialize)]
struct Picture {
    id: serde_json::Value,
    #[serde(default)]
    front: bool,
    #[serde(default)]
    approved: bool,
}

fn source_release(raw: &str) -> Option<Uuid> {
    let url = Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str() != Some("musicbrainz.org")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let path = url.path().strip_prefix("/release/")?;
    Uuid::parse_str(path).ok().filter(|id| !id.is_nil())
}

fn candidates(key: &Lookup, index: Index) -> Option<Vec<TmdbImage>> {
    let release = source_release(&index.release)?;
    if key.kind == "release" && release != key.id {
        return None;
    }
    let image = index.images.into_iter().find_map(|picture| {
        if !picture.front || !picture.approved {
            return None;
        }
        let id = match picture.id {
            serde_json::Value::String(id)
                if id.len() <= 20 && id.bytes().all(|c| c.is_ascii_digit()) =>
            {
                id.parse::<u64>().ok()?
            }
            serde_json::Value::Number(id) => id.as_u64()?,
            _ => return None,
        };
        (id > 0).then(|| TmdbImage {
            image_type: ImageType::Primary,
            url: format!("{ORIGIN}/release/{release}/{id}-1200.jpg"),
            width: None,
            height: None,
            community_rating: None,
            vote_count: None,
            language: None,
        })
    });
    Some(image.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock_http::MockServer;

    const RELEASE: &str = "11111111-1111-4111-8111-111111111111";
    const OTHER: &str = "22222222-2222-4222-8222-222222222222";
    const GROUP: &str = "33333333-3333-4333-8333-333333333333";

    fn document(release: &str, images: serde_json::Value) -> String {
        serde_json::json!({"release":format!("https://musicbrainz.org/release/{release}"),"images":images}).to_string()
    }

    #[tokio::test]
    async fn exact_release_is_used_and_success_is_cached() {
        let body = document(
            RELEASE,
            serde_json::json!([{"id":"829521842","front":true,"approved":true,"image":"https://unrelated.invalid/forged.jpg"}]),
        );
        let server = MockServer::start_raw(vec![MockServer::raw_ok(
            "Content-Type: application/json\r\n",
            body.as_bytes(),
        )])
        .await;
        let client = CoverArtArchiveClient::for_test(&server.base_url);
        let images = client.album_images(Some(RELEASE), Some(GROUP)).await;
        assert_eq!(images.len(), 1);
        assert_eq!(
            images[0].url,
            format!("{}/release/{RELEASE}/829521842-1200.jpg", server.base_url)
        );
        assert_eq!(
            client.album_images(Some(RELEASE), Some(GROUP)).await.len(),
            1
        );
        assert_eq!(server.requests(), 1);
        assert_eq!(client.cache.lock().unwrap().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn queued_lookup_has_a_total_deadline_and_releases_admission() {
        let client = CoverArtArchiveClient::for_test("http://127.0.0.1:9");
        let _gate = client.pacing.lock().await;
        let started = tokio::time::Instant::now();
        assert!(client.album_images(Some(RELEASE), None).await.is_empty());
        assert_eq!(started.elapsed(), LOOKUP_TIMEOUT);
        assert_eq!(client.admission.available_permits(), 8);
        assert!(client.cache.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn excess_uncached_work_is_refused_without_waiting() {
        let client = CoverArtArchiveClient::for_test("http://127.0.0.1:9");
        let _capacity = client.admission.acquire_many(8).await.unwrap();
        let result = tokio::time::timeout(
            Duration::from_millis(100),
            client.album_images(Some(RELEASE), None),
        )
        .await
        .expect("no queued waiter beyond the admission limit");
        assert!(result.is_empty());
        assert!(client.cache.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn identical_concurrent_lookups_share_the_cached_response() {
        let body = document(
            RELEASE,
            serde_json::json!([{"id":"1","front":true,"approved":true}]),
        );
        let server = MockServer::start_raw(vec![MockServer::raw_ok(
            "Content-Type: application/json\r\n",
            body.as_bytes(),
        )])
        .await;
        let client = Arc::new(CoverArtArchiveClient::for_test(&server.base_url));
        let mut requests = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let client = Arc::clone(&client);
            requests.spawn(async move { client.album_images(Some(RELEASE), None).await });
        }
        let mut completed = 0;
        while let Some(images) = requests.join_next().await {
            assert_eq!(images.unwrap().len(), 1);
            completed += 1;
        }
        assert_eq!(completed, 8);
        assert_eq!(server.requests(), 1);
        assert_eq!(client.admission.available_permits(), 8);
    }

    #[tokio::test]
    async fn a_different_release_is_rejected_and_not_cached() {
        let body = document(
            OTHER,
            serde_json::json!([{"id":"1","front":true,"approved":true}]),
        );
        let server = MockServer::always(&body).await;
        let client = CoverArtArchiveClient::for_test(&server.base_url);
        assert!(
            client
                .album_images(Some(RELEASE), Some(GROUP))
                .await
                .is_empty()
        );
        assert!(client.cache.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn group_cover_is_allowed_only_without_a_release_id() {
        let body = document(
            RELEASE,
            serde_json::json!([{"id":1,"front":true,"approved":true}]),
        );
        let server = MockServer::start(vec![("/release-group/", body)]).await;
        let client = CoverArtArchiveClient::for_test(&server.base_url);
        assert_eq!(client.album_images(None, Some(GROUP)).await.len(), 1);
        assert!(
            client
                .album_images(Some("broken"), Some(GROUP))
                .await
                .is_empty()
        );
        assert!(
            client
                .album_images(Some(OTHER), Some(GROUP))
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn no_art_is_cached_and_pending_or_back_art_is_not_a_cover() {
        let body = document(
            RELEASE,
            serde_json::json!([
                {"id":"1","front":false,"approved":true},
                {"id":"2","front":true,"approved":false},
                {"id":"../bad","front":true,"approved":true}
            ]),
        );
        let server = MockServer::always(&body).await;
        let client = CoverArtArchiveClient::for_test(&server.base_url);
        assert!(client.album_images(Some(RELEASE), None).await.is_empty());
        assert_eq!(client.cache.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_missing_exact_release_does_not_try_the_group_and_is_cached() {
        let server = MockServer::start_raw(vec![MockServer::raw_status("404 Not Found")]).await;
        let client = CoverArtArchiveClient::for_test(&server.base_url);
        assert!(
            client
                .album_images(Some(RELEASE), Some(GROUP))
                .await
                .is_empty()
        );
        assert!(
            client
                .album_images(Some(RELEASE), Some(GROUP))
                .await
                .is_empty()
        );
        assert_eq!(server.requests(), 1);
    }

    #[tokio::test]
    async fn oversized_index_is_rejected_before_reading_its_body() {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            JSON_LIMIT + 1
        )
        .into_bytes();
        let server = MockServer::start_raw(vec![response]).await;
        let client = CoverArtArchiveClient::for_test(&server.base_url);
        assert!(client.album_images(Some(RELEASE), None).await.is_empty());
        assert!(client.cache.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_redirect_is_checked_before_contacting_another_origin() {
        let destination = MockServer::start_raw(vec![MockServer::raw_status("200 OK")]).await;
        let response = format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {}/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", destination.base_url).into_bytes();
        let source = MockServer::start_raw(vec![response]).await;
        let client = CoverArtArchiveClient::for_test(&source.base_url);
        assert!(client.album_images(Some(RELEASE), None).await.is_empty());
        assert_eq!(source.requests(), 1);
        assert_eq!(destination.requests(), 0);
    }

    #[test]
    fn hops_require_official_https_hosts_and_public_addresses() {
        for raw in [
            "http://coverartarchive.org/release/x",
            "https://archive.org.evil.invalid/x",
            "https://coverartarchive.org.evil.invalid/x",
            "https://user@archive.org/x",
            "https://archive.org:8443/x",
            "https://127.0.0.1/x",
        ] {
            assert!(!allowed_hop(&Url::parse(raw).unwrap()), "{raw}");
        }
        for raw in [
            "https://coverartarchive.org/release/x",
            "https://archive.org/download/x",
            "https://ia800100.us.archive.org/0/items/x",
        ] {
            assert!(allowed_hop(&Url::parse(raw).unwrap()), "{raw}");
        }
        assert!(is_private_address("127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn source_release_must_be_an_exact_musicbrainz_release_url() {
        for raw in [
            format!("https://musicbrainz.org.evil.invalid/release/{RELEASE}"),
            format!("https://musicbrainz.org/release/{RELEASE}?other=1"),
            format!("https://musicbrainz.org/release-group/{RELEASE}"),
            "https://musicbrainz.org/release/not-an-id".into(),
        ] {
            assert!(source_release(&raw).is_none());
        }
        assert_eq!(
            source_release(&format!("http://musicbrainz.org/release/{RELEASE}")),
            Uuid::parse_str(RELEASE).ok()
        );
    }
}
