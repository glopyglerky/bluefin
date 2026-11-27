//! Credential-owning adapters for the documented upstream HTTP APIs.
//! Callers configure their own credentials. The portable/default build excludes this module.
use crate::{video, *};
use alloc::{format, string::String, vec, vec::Vec};
use core::fmt;
use reqwest::{
    Client, RequestBuilder, StatusCode,
    header::{AUTHORIZATION, HeaderValue},
    redirect::Policy,
};
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;
const TMDB_API: &str = "https://api.themoviedb.org/3/";
const MOTN_DIRECT_API: &str = "https://api.movieofthenight.com/v4/";
const MOTN_RAPID_API: &str = "https://streaming-availability.p.rapidapi.com/";

/// Credentials are intentionally neither serializable nor printable.
#[derive(Clone)]
pub enum TmdbCredential {
    ApiKey(String),
    Bearer(String),
}
impl fmt::Debug for TmdbCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ApiKey(_) => "ApiKey([redacted])",
            Self::Bearer(_) => "Bearer([redacted])",
        })
    }
}
/// Direct and RapidAPI subscriptions have different canonical endpoints and authentication headers.
#[derive(Clone)]
pub enum MovieOfTheNightCredential {
    DirectApiKey(String),
    RapidApiKey(String),
}
impl fmt::Debug for MovieOfTheNightCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::DirectApiKey(_) => "DirectApiKey([redacted])",
            Self::RapidApiKey(_) => "RapidApiKey([redacted])",
        })
    }
}
/// Optional configured sources. Missing credentials become explicit source observations.
#[derive(Clone, Debug, Default)]
pub struct Credentials {
    pub tmdb: Option<TmdbCredential>,
    pub movie_of_the_night: Option<MovieOfTheNightCredential>,
}
/// Safe initialization failures do not include request URLs, credential values or upstream bodies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientError {
    Initialization,
    InvalidCredential(&'static str),
}
impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Initialization => f.write_str("could not initialize catalog HTTP client"),
            Self::InvalidCredential(source) => {
                write!(f, "invalid {source} credential configuration")
            }
        }
    }
}
impl std::error::Error for ClientError {}

#[derive(Clone)]
enum TmdbAuth {
    ApiKey(String),
    Bearer(HeaderValue),
}
#[derive(Clone)]
struct MotnAuth {
    rapid: bool,
    key: HeaderValue,
}

/// Reusable native HTTP client. Provider credentials never enter the shared response model.
#[derive(Clone)]
pub struct CatalogClient {
    client: Client,
    tmdb: Option<TmdbAuth>,
    motn: Option<MotnAuth>,
}
impl fmt::Debug for CatalogClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CatalogClient")
            .field("tmdb_configured", &self.tmdb.is_some())
            .field("movie_of_the_night_configured", &self.motn.is_some())
            .finish_non_exhaustive()
    }
}
fn secret_header(value: &str, source: &'static str) -> Result<HeaderValue, ClientError> {
    if value.is_empty() || value.len() > 8192 || value.chars().any(char::is_whitespace) {
        return Err(ClientError::InvalidCredential(source));
    }
    let mut header =
        HeaderValue::from_str(value).map_err(|_| ClientError::InvalidCredential(source))?;
    header.set_sensitive(true);
    Ok(header)
}
impl CatalogClient {
    /// Uses fixed API origins, no ambient proxy, no redirects and bounded request deadlines.
    pub fn new(credentials: Credentials) -> Result<Self, ClientError> {
        let tmdb = match credentials.tmdb {
            Some(TmdbCredential::ApiKey(value)) => {
                secret_header(&value, video::TMDB_SOURCE)?;
                Some(TmdbAuth::ApiKey(value))
            }
            Some(TmdbCredential::Bearer(value)) => {
                secret_header(&value, video::TMDB_SOURCE)?;
                let mut header = HeaderValue::from_str(&format!("Bearer {value}"))
                    .map_err(|_| ClientError::InvalidCredential(video::TMDB_SOURCE))?;
                header.set_sensitive(true);
                Some(TmdbAuth::Bearer(header))
            }
            None => None,
        };
        let motn = match credentials.movie_of_the_night {
            Some(MovieOfTheNightCredential::DirectApiKey(value)) => Some(MotnAuth {
                rapid: false,
                key: secret_header(&value, video::MOVIE_OF_THE_NIGHT_SOURCE)?,
            }),
            Some(MovieOfTheNightCredential::RapidApiKey(value)) => Some(MotnAuth {
                rapid: true,
                key: secret_header(&value, video::MOVIE_OF_THE_NIGHT_SOURCE)?,
            }),
            None => None,
        };
        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(8))
            .user_agent(concat!(
                "Bluefin-Catalog/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/glopyglerky/bluefin)"
            ))
            .build()
            .map_err(|_| ClientError::Initialization)?;
        Ok(Self { client, tmdb, motn })
    }

    /// Aggregates independent video source observations. It never turns failures into absence.
    pub async fn lookup(&self, request: &LookupRequest) -> OfferResponse {
        let (tmdb, motn) = tokio::join!(self.lookup_tmdb(request), self.lookup_motn(request));
        let mut response = OfferResponse::empty(request);
        response.merge(tmdb);
        response.merge(motn);
        response
    }

    /// Discovers upstream countries and service IDs without a compiled provider allowlist.
    pub async fn registry(&self, country: Option<&CountryCode>) -> RegistryResponse {
        let (mut tmdb, motn) =
            tokio::join!(self.tmdb_registry(country), self.motn_registry(country));
        tmdb.merge(motn);
        tmdb
    }

    fn tmdb_request(&self, path: &str) -> Result<RequestBuilder, SourceStatus> {
        let auth = self.tmdb.as_ref().ok_or(SourceStatus::MissingCredentials)?;
        let url = Url::parse(TMDB_API)
            .and_then(|base| base.join(path))
            .map_err(|_| SourceStatus::InvalidResponse)?;
        let request = self.client.get(url);
        Ok(match auth {
            TmdbAuth::ApiKey(key) => request.query(&[("api_key", key)]),
            TmdbAuth::Bearer(value) => request.header(AUTHORIZATION, value.clone()),
        })
    }
    fn motn_request(&self, path: &[&str]) -> Result<RequestBuilder, SourceStatus> {
        let auth = self.motn.as_ref().ok_or(SourceStatus::MissingCredentials)?;
        let mut url = Url::parse(if auth.rapid {
            MOTN_RAPID_API
        } else {
            MOTN_DIRECT_API
        })
        .map_err(|_| SourceStatus::InvalidResponse)?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| SourceStatus::InvalidResponse)?;
            segments.pop_if_empty();
            for part in path {
                segments.push(part);
            }
        }
        let mut request = self.client.get(url);
        if auth.rapid {
            request = request
                .header("X-RapidAPI-Key", auth.key.clone())
                .header("X-RapidAPI-Host", "streaming-availability.p.rapidapi.com");
        } else {
            request = request.header("X-API-Key", auth.key.clone());
        }
        Ok(request)
    }
    async fn resolve_tmdb_request(
        &self,
        request: &LookupRequest,
    ) -> Result<(LookupRequest, String), SourceStatus> {
        if request.identity.id(video::TMDB_SOURCE).is_some() {
            let key = video::tmdb_key(&request.identity)?;
            if request.identity.id("imdb").is_some() {
                let imdb = video::imdb_key(&request.identity)?;
                let ids = fetch_json(self.tmdb_request(&format!("{key}/external_ids"))?).await?;
                match ids.get("imdb_id").and_then(Value::as_str) {
                    Some(actual) if actual == imdb => {}
                    Some(_) => return Err(SourceStatus::IdentityMismatch),
                    None => return Err(SourceStatus::InvalidResponse),
                }
            }
            return Ok((request.clone(), key));
        }
        let imdb = video::imdb_key(&request.identity)?;
        let (array, kind) = match request.identity.kind {
            MediaKind::Movie => ("movie_results", "movie"),
            MediaKind::Series => ("tv_results", "tv"),
            _ => return Err(SourceStatus::UnsupportedIdentity),
        };
        let found = fetch_json(
            self.tmdb_request(&format!("find/{imdb}"))?
                .query(&[("external_source", "imdb_id")]),
        )
        .await?;
        let items = found
            .get(array)
            .and_then(Value::as_array)
            .ok_or(SourceStatus::InvalidResponse)?;
        if items.is_empty() {
            return Err(SourceStatus::NotFound);
        }
        if items.len() != 1 {
            return Err(SourceStatus::IdentityMismatch);
        }
        let id = items[0]
            .get("id")
            .and_then(Value::as_u64)
            .filter(|id| *id > 0)
            .ok_or(SourceStatus::InvalidResponse)?;
        let key = format!("{kind}/{id}");
        let mut resolved = request.clone();
        resolved.identity.ids.push(CatalogId {
            source: video::TMDB_SOURCE.into(),
            id: key.clone(),
        });
        Ok((resolved, key))
    }
    async fn lookup_tmdb(&self, request: &LookupRequest) -> OfferResponse {
        let now = observed_at();
        if !matches!(request.identity.kind, MediaKind::Movie | MediaKind::Series) {
            return failure(
                request,
                video::TMDB_SOURCE,
                SourceStatus::UnsupportedIdentity,
                now,
            );
        }
        let valid_key = if request.identity.id(video::TMDB_SOURCE).is_some() {
            video::tmdb_key(&request.identity)
        } else {
            video::imdb_key(&request.identity)
        };
        if let Err(status) = valid_key {
            return failure(request, video::TMDB_SOURCE, status, now);
        }
        if self.tmdb.is_none() {
            return failure(
                request,
                video::TMDB_SOURCE,
                SourceStatus::MissingCredentials,
                now,
            );
        }
        let (resolved, key) = match self.resolve_tmdb_request(request).await {
            Ok(found) => found,
            Err(status) => return failure(request, video::TMDB_SOURCE, status, now),
        };
        let result = match self.tmdb_request(&format!("{key}/watch/providers")) {
            Ok(http) => fetch_json(http).await,
            Err(status) => Err(status),
        };
        match result {
            Ok(body) => {
                let mut projected =
                    video::project_tmdb_watch_providers(&resolved, &body, observed_at());
                // Offers may carry the source-verified cross-reference, while aggregation remains keyed to the request.
                projected.identity = request.identity.clone();
                projected
            }
            Err(status) => failure(request, video::TMDB_SOURCE, status, now),
        }
    }
    async fn lookup_motn(&self, request: &LookupRequest) -> OfferResponse {
        let now = observed_at();
        let key = match video::movie_of_the_night_key(&request.identity) {
            Ok(key) => key,
            Err(status) => return failure(request, video::MOVIE_OF_THE_NIGHT_SOURCE, status, now),
        };
        let mut http = match self.motn_request(&["shows", &key]) {
            Ok(http) => http,
            Err(status) => return failure(request, video::MOVIE_OF_THE_NIGHT_SOURCE, status, now),
        };
        http = http.query(&[("series_granularity", "show")]);
        if let Some(country) = &request.country {
            http = http.query(&[("country", country.lower())]);
        }
        match fetch_json(http).await {
            Ok(body) => video::project_movie_of_the_night(request, &body, observed_at()),
            Err(status) => failure(request, video::MOVIE_OF_THE_NIGHT_SOURCE, status, now),
        }
    }
    async fn tmdb_registry(&self, country: Option<&CountryCode>) -> RegistryResponse {
        let now = observed_at();
        let regions = self.tmdb_request("watch/providers/regions");
        let movie = self.tmdb_request("watch/providers/movie");
        let tv = self.tmdb_request("watch/providers/tv");
        let (regions, mut movie, mut tv) = match (regions, movie, tv) {
            (Ok(regions), Ok(movie), Ok(tv)) => (regions, movie, tv),
            _ => {
                return registry_failure(video::TMDB_SOURCE, SourceStatus::MissingCredentials, now);
            }
        };
        if let Some(country) = country {
            movie = movie.query(&[("watch_region", country.as_str())]);
            tv = tv.query(&[("watch_region", country.as_str())]);
        }
        match tokio::try_join!(fetch_json(regions), fetch_json(movie), fetch_json(tv)) {
            Ok((regions, movie, tv)) => {
                video::project_tmdb_registry(country, &regions, &movie, &tv, observed_at())
            }
            Err(status) => registry_failure(video::TMDB_SOURCE, status, now),
        }
    }
    async fn motn_registry(&self, country: Option<&CountryCode>) -> RegistryResponse {
        let now = observed_at();
        let http = match self.motn_request(&["countries"]) {
            Ok(http) => http,
            Err(status) => return registry_failure(video::MOVIE_OF_THE_NIGHT_SOURCE, status, now),
        };
        match fetch_json(http).await {
            Ok(body) => video::project_movie_of_the_night_registry(country, &body, observed_at()),
            Err(status) => registry_failure(video::MOVIE_OF_THE_NIGHT_SOURCE, status, now),
        }
    }
}
fn source_observation(source: &str, status: SourceStatus, observed_at: u64) -> SourceObservation {
    let attributions = if source == video::TMDB_SOURCE {
        vec![
            Attribution {
                name: "TMDB".into(),
                url: "https://www.themoviedb.org/".into(),
            },
            Attribution {
                name: "JustWatch".into(),
                url: "https://www.justwatch.com/".into(),
            },
        ]
    } else {
        vec![Attribution {
            name: "Movie of the Night".into(),
            url: "https://www.movieofthenight.com/".into(),
        }]
    };
    SourceObservation {
        source: source.into(),
        status,
        observed_at,
        scope: CoverageScope::RegionalAvailability,
        coverage: Coverage::Unknown,
        catalog_url: None,
        attributions,
    }
}
fn failure(
    request: &LookupRequest,
    source: &str,
    status: SourceStatus,
    observed_at: u64,
) -> OfferResponse {
    let mut response = OfferResponse::empty(request);
    response
        .sources
        .push(source_observation(source, status, observed_at));
    response
}
fn registry_failure(source: &str, status: SourceStatus, observed_at: u64) -> RegistryResponse {
    RegistryResponse {
        schema_version: 1,
        countries: Vec::new(),
        sources: vec![source_observation(source, status, observed_at)],
    }
}
pub(crate) fn observed_at() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_secs())
        .unwrap_or(0)
}
fn status_error(status: StatusCode) -> Option<SourceStatus> {
    match status.as_u16() {
        200..=299 => None,
        401 | 403 => Some(SourceStatus::Denied),
        404 => Some(SourceStatus::NotFound),
        429 => Some(SourceStatus::RateLimited),
        500..=599 => Some(SourceStatus::Unavailable),
        _ => Some(SourceStatus::InvalidResponse),
    }
}
/// Never returns raw transport errors (which can include an API-key query) or response bodies.
pub(crate) async fn fetch_json(request: RequestBuilder) -> Result<Value, SourceStatus> {
    let mut response = request
        .send()
        .await
        .map_err(|_| SourceStatus::Unavailable)?;
    if let Some(status) = status_error(response.status()) {
        return Err(status);
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_JSON_BYTES as u64)
    {
        return Err(SourceStatus::InvalidResponse);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| SourceStatus::Unavailable)?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_JSON_BYTES {
            return Err(SourceStatus::InvalidResponse);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| SourceStatus::InvalidResponse)
}

/// A public TMDB metadata document with explicit acquisition status.
#[derive(Clone, Debug)]
pub struct TmdbDocument {
    pub body: Option<Value>,
    pub observed_at: u64,
    pub status: SourceStatus,
}
impl TmdbDocument {
    fn failure(status: SourceStatus) -> Self {
        Self {
            body: None,
            observed_at: observed_at(),
            status,
        }
    }
}
impl CatalogClient {
    /// Searches the configured TMDB catalog using the same owned credentials and bounded transport.
    pub async fn metadata_search(&self, kind: MediaKind, term: &str, page: u32) -> TmdbDocument {
        let Some(resource) = metadata_resource(kind) else {
            return TmdbDocument::failure(SourceStatus::UnsupportedIdentity);
        };
        let term = term.trim();
        if !(2..=160).contains(&term.chars().count())
            || term.chars().any(char::is_control)
            || !(1..=100).contains(&page)
        {
            return TmdbDocument::failure(SourceStatus::UnsupportedIdentity);
        }
        let request = match self.tmdb_request(&format!("search/{resource}")) {
            Ok(request) => request
                .query(&[("query", term), ("include_adult", "false")])
                .query(&[("page", page)]),
            Err(status) => return TmdbDocument::failure(status),
        };
        match fetch_json(request).await {
            Ok(body) => {
                if body.get("page").and_then(Value::as_u64) != Some(u64::from(page))
                    || body.get("results").and_then(Value::as_array).is_none()
                    || body.get("total_pages").and_then(Value::as_u64).is_none()
                {
                    return TmdbDocument::failure(SourceStatus::InvalidResponse);
                }
                TmdbDocument {
                    body: Some(body),
                    observed_at: observed_at(),
                    status: SourceStatus::Complete,
                }
            }
            Err(status) => TmdbDocument::failure(status),
        }
    }
    /// Retrieves exact movie, TV or person metadata; no free-text match chooses a detail identity.
    pub async fn metadata(&self, identity: &CatalogIdentity) -> TmdbDocument {
        let Some(resource) = metadata_resource(identity.kind) else {
            return TmdbDocument::failure(SourceStatus::UnsupportedIdentity);
        };
        let Some(raw) = identity.id("tmdb") else {
            return TmdbDocument::failure(SourceStatus::UnsupportedIdentity);
        };
        if identity
            .ids
            .iter()
            .filter(|id| id.source.eq_ignore_ascii_case("tmdb"))
            .any(|id| id.id != raw)
        {
            return TmdbDocument::failure(SourceStatus::IdentityMismatch);
        }
        let id = if let Some((prefix, id)) = raw.split_once('/') {
            if prefix != resource {
                return TmdbDocument::failure(SourceStatus::IdentityMismatch);
            }
            id
        } else {
            raw
        };
        if id.is_empty() || id.len() > 20 || !id.bytes().all(|byte| byte.is_ascii_digit()) {
            return TmdbDocument::failure(SourceStatus::UnsupportedIdentity);
        }
        let Some(id) = id.parse::<u64>().ok().filter(|id| *id > 0) else {
            return TmdbDocument::failure(SourceStatus::UnsupportedIdentity);
        };
        let includes = if kind_is_person(identity.kind) {
            "combined_credits,external_ids"
        } else {
            "credits,external_ids"
        };
        let request = match self.tmdb_request(&format!("{resource}/{id}")) {
            Ok(request) => request.query(&[("append_to_response", includes)]),
            Err(status) => return TmdbDocument::failure(status),
        };
        match fetch_json(request).await {
            Ok(body) => {
                match body.get("id").and_then(Value::as_u64) {
                    Some(actual) if actual == id => {}
                    Some(_) => return TmdbDocument::failure(SourceStatus::IdentityMismatch),
                    None => return TmdbDocument::failure(SourceStatus::InvalidResponse),
                }
                TmdbDocument {
                    body: Some(body),
                    observed_at: observed_at(),
                    status: SourceStatus::Complete,
                }
            }
            Err(status) => TmdbDocument::failure(status),
        }
    }
}
fn metadata_resource(kind: MediaKind) -> Option<&'static str> {
    match kind {
        MediaKind::Movie => Some("movie"),
        MediaKind::Series => Some("tv"),
        MediaKind::Person => Some("person"),
        _ => None,
    }
}
fn kind_is_person(kind: MediaKind) -> bool {
    kind == MediaKind::Person
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_failures_remain_distinct_from_no_offers() {
        for (code, expected) in [
            (401, SourceStatus::Denied),
            (403, SourceStatus::Denied),
            (404, SourceStatus::NotFound),
            (429, SourceStatus::RateLimited),
            (503, SourceStatus::Unavailable),
            (302, SourceStatus::InvalidResponse),
        ] {
            assert_eq!(
                status_error(StatusCode::from_u16(code).unwrap()),
                Some(expected)
            );
        }
        assert_eq!(status_error(StatusCode::OK), None);
    }
    #[test]
    fn credentials_are_redacted_and_invalid_headers_rejected() {
        let secret = "example-secret-value";
        let credentials = Credentials {
            tmdb: Some(TmdbCredential::ApiKey(secret.into())),
            movie_of_the_night: Some(MovieOfTheNightCredential::RapidApiKey(secret.into())),
        };
        assert!(!format!("{credentials:?}").contains(secret));
        assert!(secret_header("unsafe\nheader", "source").is_err());
        assert!(secret_header("", "source").is_err());
        assert!(secret_header(secret, "source").unwrap().is_sensitive());
    }
    #[tokio::test]
    async fn unconfigured_sources_are_not_reported_as_no_offers() {
        let client = CatalogClient::new(Credentials::default()).unwrap();
        let request = LookupRequest {
            identity: CatalogIdentity {
                kind: MediaKind::Movie,
                ids: vec![CatalogId {
                    source: "tmdb".into(),
                    id: "movie/238".into(),
                }],
            },
            country: None,
        };
        let response = client.lookup(&request).await;
        assert!(response.offers.is_empty());
        assert_eq!(response.sources.len(), 2);
        assert!(
            response
                .sources
                .iter()
                .all(|source| source.status == SourceStatus::MissingCredentials)
        );
    }
}
