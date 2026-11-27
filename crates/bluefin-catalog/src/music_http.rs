//! Hosted acquisition for MusicBrainz access relationships and exact iTunes music listings.
//!
//! Only the two fixed upstream API origins are queried. Returned destination URLs are data;
//! this client never fetches or launches them. The in-memory cache contains public metadata
//! and is disposable. Durable catalog ownership remains with the caller.
//!
//! MusicBrainz's public service permits an average of one request per second per source IP.
//! The process-wide gate below shares that budget across all instances of this adapter.
//! Hosts sharing an egress IP must route their reads through the same catalog owner.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use alloc::{format, string::{String, ToString}, vec::Vec};
use reqwest::{Client, StatusCode};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::{LookupRequest, MediaKind, OfferResponse, SourceStatus};
use crate::musicbrainz::{
    itunes_music_ids, itunes_release_group_ids, itunes_source, musicbrainz_entity, musicbrainz_source,
    project_itunes_music_lookup, project_musicbrainz_relations,
    project_musicbrainz_release_group_editions,
};

const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_CACHE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 64;
const USER_AGENT: &str = "BluefinCatalog/0.1 (https://github.com/glopyglerky/bluefin)";

/// Public music access acquisition with no paid credentials or implicit storefront.
#[derive(Clone)]
pub struct MusicBrainzClient {
    client: Client,
}

impl MusicBrainzClient {
    /// Constructs a fixed-origin, redirect-disabled, bounded-time public metadata client.
    pub fn new() -> Result<Self, reqwest::Error> {
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .pool_max_idle_per_host(1)
            .build()?;
        Ok(Self { client })
    }


    /// Searches one typed public music catalog page using MusicBrainz's shared IP budget.
    /// The returned count and offset belong to the source; a page is not a complete catalog.
    pub async fn search(
        &self, kind: MediaKind, term: &str, offset: u64,
    ) -> MusicBrainzDocument {
        let (entity, field) = match kind {
            MediaKind::Artist => ("artist", "artists"),
            MediaKind::ReleaseGroup => ("release-group", "release-groups"),
            MediaKind::Recording => ("recording", "recordings"),
            _ => return MusicBrainzDocument::failure(SourceStatus::UnsupportedIdentity),
        };
        let term = term.trim();
        if !(2..=160).contains(&term.chars().count()) || term.chars().any(char::is_control)
            || offset > 100_000 {
            return MusicBrainzDocument::failure(SourceStatus::InvalidResponse);
        }
        let Ok(mut url) = reqwest::Url::parse(&format!("https://musicbrainz.org/ws/2/{entity}")) else {
            return MusicBrainzDocument::failure(SourceStatus::InvalidResponse);
        };
        url.query_pairs_mut().append_pair("query", &search_query(term)).append_pair("fmt","json")
            .append_pair("limit","20").append_pair("offset",&offset.to_string());
        let acquired = acquire(&self.client, provider_state(Provider::MusicBrainz),
            url.as_str(), Provider::MusicBrainz, |body| {
                if body[field].as_array().is_none() || body["count"].as_u64().is_none()
                    || body["offset"].as_u64() != Some(offset) {
                    Some(SourceStatus::InvalidResponse)
                } else { None }
            }).await;
        MusicBrainzDocument::from(acquired)
    }

    /// Loads descriptive metadata and related entities for one exact MusicBrainz identity.
    pub async fn metadata(&self, identity: &crate::CatalogIdentity) -> MusicBrainzDocument {
        let Some((entity,id)) = musicbrainz_entity(identity) else {
            return MusicBrainzDocument::failure(SourceStatus::UnsupportedIdentity);
        };
        let include = match identity.kind {
            MediaKind::Artist => "url-rels+release-groups+genres",
            MediaKind::ReleaseGroup => "artists+releases+url-rels+genres",
            MediaKind::Release => "artists+recordings+release-groups+url-rels+genres",
            MediaKind::Recording | MediaKind::MusicVideo => "artists+releases+isrcs+url-rels+genres",
            _ => "url-rels+genres",
        };
        let Ok(mut url) = reqwest::Url::parse(&format!("https://musicbrainz.org/ws/2/{entity}/{id}")) else {
            return MusicBrainzDocument::failure(SourceStatus::InvalidResponse);
        };
        url.query_pairs_mut().append_pair("inc",include).append_pair("fmt","json");
        let acquired = acquire(&self.client, provider_state(Provider::MusicBrainz),
            url.as_str(), Provider::MusicBrainz, |body| {
                if !body["id"].as_str().is_some_and(|actual| actual.eq_ignore_ascii_case(&id)) {
                    Some(SourceStatus::IdentityMismatch)
                } else if identity.kind == MediaKind::MusicVideo && body["video"].as_bool()!=Some(true) {
                    Some(SourceStatus::IdentityMismatch)
                } else if body["name"].as_str().or_else(||body["title"].as_str()).is_none() {
                    Some(SourceStatus::InvalidResponse)
                } else { None }
            }).await;
        MusicBrainzDocument::from(acquired)
    }

    /// Looks up real music access links and, when requested, exact regional purchase listings.
    ///
    /// MusicBrainz relationships stay region-unknown. The iTunes supplement requires an
    /// explicitly selected country and an exact upstream-linked or supplied iTunes identity.
    /// An API failure remains a source failure, including when older cached links are retained.
    pub async fn lookup(&self, request: &LookupRequest) -> OfferResponse {
        let mut response = OfferResponse::empty(request);
        let mut body = None;
        let mut current_musicbrainz = false;
        if let Some((entity, id)) = musicbrainz_entity(&request.identity) {
            let url = format!("https://musicbrainz.org/ws/2/{entity}/{id}?inc=url-rels&fmt=json");
            let acquired = acquire(
                &self.client, provider_state(Provider::MusicBrainz), &url, Provider::MusicBrainz,
                |value| {
                    let projected = project_musicbrainz_relations(request, value, 0);
                    match projected.sources[0].status {
                        SourceStatus::Complete | SourceStatus::NoOffers | SourceStatus::Partial => None,
                        status => Some(status),
                    }
                },
            ).await;
            current_musicbrainz = acquired.status == SourceStatus::Complete;
            let mut projected = if let Some(value) = &acquired.body {
                project_musicbrainz_relations(request, value, acquired.observed_at)
            } else {
                musicbrainz_source(request, acquired.status, acquired.observed_at)
            };
            if acquired.status != SourceStatus::Complete {
                projected.sources[0].status = acquired.status;
            }
            response.merge(projected);
            body = acquired.body;
        }

        let mut ids: BTreeSet<String> = itunes_music_ids(
            &request.identity, if current_musicbrainz { body.as_ref() } else { None },
        ).into_iter().collect();
        if request.identity.kind == MediaKind::ReleaseGroup && body.is_some() {
            let (editions, apple_ids) = self.release_group_editions(request).await;
            response.merge(editions);
            ids.extend(apple_ids);
        }
        let ids: Vec<String> = ids.into_iter().take(20).collect();
        if !ids.is_empty() {
            if let Some(country) = &request.country {
                let url = format!(
                    "https://itunes.apple.com/lookup?id={}&country={}",
                    ids.join(","), country.as_str(),
                );
                let acquired = acquire(
                    &self.client, provider_state(Provider::Itunes), &url, Provider::Itunes,
                    |value| {
                        let projected = project_itunes_music_lookup(request, &ids, value, 0);
                        match projected.sources[0].status {
                            SourceStatus::Complete | SourceStatus::NoOffers | SourceStatus::Partial =>
                                None,
                            status => Some(status),
                        }
                    },
                ).await;
                let mut projected = if let Some(value) = &acquired.body {
                    project_itunes_music_lookup(request, &ids, value, acquired.observed_at)
                } else {
                    itunes_source(request, acquired.status, acquired.observed_at)
                };
                if acquired.status != SourceStatus::Complete {
                    projected.sources[0].status = acquired.status;
                }
                response.merge(projected);
            } else {
                // No upstream response was observed: do not manufacture a lookup time or US result.
                response.merge(itunes_source(request, SourceStatus::UnsupportedCountry, 0));
            }
        }
        if response.sources.is_empty() {
            response = musicbrainz_source(request, SourceStatus::UnsupportedIdentity, 0);
        }
        response
    }

    async fn release_group_editions(&self, request: &LookupRequest) -> (OfferResponse, Vec<String>) {
        let Some(("release-group", group_id)) = musicbrainz_entity(&request.identity) else {
            return (musicbrainz_source(request, SourceStatus::UnsupportedIdentity, 0), Vec::new());
        };
        // Browse actual editions with URLs in the same response, never one lookup per edition.
        // Keep an interactive deadline. Unfinished pagination remains explicitly partial.
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut offset = 0u64;
        let mut response = OfferResponse::empty(request);
        let mut apple_ids = BTreeSet::new();
        let mut incomplete = false;
        let mut completed = false;
        for _ in 0..32 {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                incomplete = true;
                break;
            };
            let url = format!(
                "https://musicbrainz.org/ws/2/release?release-group={group_id}&inc=url-rels%2Brelease-groups&limit=100&offset={offset}&fmt=json",
            );
            let acquired = tokio::time::timeout(remaining, acquire(
                &self.client, provider_state(Provider::MusicBrainz), &url, Provider::MusicBrainz,
                |body| {
                    if body["release-offset"].as_u64() != Some(offset) {
                        return Some(SourceStatus::InvalidResponse);
                    }
                    match project_musicbrainz_release_group_editions(request, body, 0).sources[0].status {
                        SourceStatus::Complete | SourceStatus::NoOffers | SourceStatus::Partial => None,
                        status => Some(status),
                    }
                },
            )).await;
            let acquired = match acquired {
                Ok(value) => value,
                Err(_) => {
                    response.sources.extend(
                        musicbrainz_source(request, SourceStatus::Unavailable, observed_at()).sources,
                    );
                    incomplete = true;
                    break;
                }
            };
            let Some(body) = acquired.body.as_ref() else {
                response.sources.extend(
                    musicbrainz_source(request, acquired.status, acquired.observed_at).sources,
                );
                incomplete = true;
                break;
            };
            let mut page = project_musicbrainz_release_group_editions(
                request, body, acquired.observed_at,
            );
            if acquired.status != SourceStatus::Complete {
                page.sources[0].status = acquired.status;
            } else {
                apple_ids.extend(itunes_release_group_ids(&request.identity, body));
            }
            let returned = body["releases"].as_array().map_or(0, Vec::len) as u64;
            let total = body["release-count"].as_u64().unwrap_or(0);
            let next = offset.saturating_add(returned);
            // A cached page is still projected with its original observation time.
            response.merge(page);
            if acquired.status != SourceStatus::Complete {
                incomplete = true;
                break;
            }
            if next >= total {
                completed = true;
                break;
            }
            if returned == 0 {
                incomplete = true;
                break;
            }
            offset = next;
        }
        if !completed { incomplete = true; }
        if incomplete && response.sources.is_empty() {
            response.sources.extend(musicbrainz_source(request, SourceStatus::Partial, 0).sources);
        }
        // Each page keeps its own observation time/status; no pagination bound means "no offers".
        for source in &mut response.sources {
            if incomplete && matches!(source.status, SourceStatus::Complete | SourceStatus::NoOffers) {
                source.status = SourceStatus::Partial;
            }
            source.catalog_url = Some(format!("https://musicbrainz.org/release-group/{group_id}"));
        }
        (response, apple_ids.into_iter().take(20).collect())
    }
}


/// A public upstream document with its real observation time and acquisition state.
/// Stale cached metadata can be retained during an outage without relabelling it as current.
#[derive(Clone, Debug)]
pub struct MusicBrainzDocument {
    pub body: Option<Value>,
    pub observed_at: u64,
    pub status: SourceStatus,
}
impl MusicBrainzDocument {
    fn failure(status: SourceStatus) -> Self {
        Self { body: None, observed_at: 0, status }
    }
}
impl From<Acquisition> for MusicBrainzDocument {
    fn from(value: Acquisition) -> Self {
        Self { body: value.body, observed_at: value.observed_at, status: value.status }
    }
}


fn search_query(term: &str) -> String {
    // Treat a person's title/artist phrase as text, not MusicBrainz's Lucene operators.
    let mut escaped = String::new();
    for character in term.chars() {
        if matches!(character, '+' | '-' | '&' | '|' | '!' | '(' | ')' | '{' | '}' |
            '[' | ']' | '^' | '"' | '~' | '*' | '?' | ':' | '\\' | '/') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

#[derive(Clone, Copy)]
enum Provider { MusicBrainz, Itunes }

impl Provider {
    fn interval(self) -> Duration {
        match self {
            Self::MusicBrainz => Duration::from_secs(1),
            // Apple's public Search API documents an approximate twenty-call/minute limit.
            Self::Itunes => Duration::from_secs(3),
        }
    }
    fn fresh_for(self) -> Duration {
        match self {
            Self::MusicBrainz => Duration::from_secs(24 * 60 * 60),
            Self::Itunes => Duration::from_secs(60 * 60),
        }
    }
    fn stale_for(self) -> Duration {
        match self {
            Self::MusicBrainz => Duration::from_secs(7 * 24 * 60 * 60),
            Self::Itunes => Duration::from_secs(24 * 60 * 60),
        }
    }
}

#[derive(Default)]
struct HttpState {
    next_request: Option<Instant>,
    cooldown: Option<(Instant, SourceStatus, u64)>,
    cache: BTreeMap<String, Cached>,
}

#[derive(Clone)]
struct Cached {
    body: Option<Value>,
    observed_at: u64,
    status: SourceStatus,
    fresh_until: Instant,
    stale_until: Instant,
    last_used: Instant,
    bytes: usize,
}

struct Acquisition {
    body: Option<Value>,
    observed_at: u64,
    status: SourceStatus,
}

fn provider_state(provider: Provider) -> &'static Mutex<HttpState> {
    static MUSICBRAINZ: OnceLock<Mutex<HttpState>> = OnceLock::new();
    static ITUNES: OnceLock<Mutex<HttpState>> = OnceLock::new();
    match provider {
        Provider::MusicBrainz => MUSICBRAINZ.get_or_init(|| Mutex::new(HttpState::default())),
        Provider::Itunes => ITUNES.get_or_init(|| Mutex::new(HttpState::default())),
    }
}

fn observed_at() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

async fn acquire(
    client: &Client,
    state: &Mutex<HttpState>,
    url: &str,
    provider: Provider,
    validate: impl Fn(&Value) -> Option<SourceStatus>,
) -> Acquisition {
    // Keeping the gate during a request also coalesces concurrent cache fills for the same URL.
    let mut state = state.lock().await;
    let now = Instant::now();
    state.cache.retain(|_, entry| entry.stale_until > now);
    if let Some(entry) = state.cache.get_mut(url).filter(|entry| entry.fresh_until > now) {
        entry.last_used = now;
        // Recording and MusicVideo may share a URL; every caller still verifies its typed identity.
        if let Some(status) = entry.body.as_ref().and_then(&validate) {
            return Acquisition { body: None, observed_at: entry.observed_at, status };
        }
        return Acquisition {
            body: entry.body.clone(), observed_at: entry.observed_at, status: entry.status,
        };
    }
    if let Some((_, status, failed_at)) = state.cooldown.filter(|(until, _, _)| *until > now) {
        return fallback(&state, url, status, failed_at, &validate);
    }
    if let Some(next) = state.next_request {
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            tokio::time::sleep(wait).await;
        }
    }
    state.next_request = Some(Instant::now() + provider.interval());
    let answer = fetch(client, url, observed_at()).await;
    let received_at = observed_at();
    match answer {
        Ok((body, bytes)) => {
            if let Some(status) = validate(&body) {
                return fallback(&state, url, status, received_at, &validate);
            }
            let now = Instant::now();
            insert_cache(&mut state, url, Cached {
                body: Some(body.clone()), observed_at: received_at, status: SourceStatus::Complete,
                fresh_until: now + provider.fresh_for(),
                stale_until: now + provider.stale_for(),
                last_used: now, bytes,
            });
            state.cooldown = None;
            Acquisition { body: Some(body), observed_at: received_at, status: SourceStatus::Complete }
        }
        Err(failure) => {
            if failure.status == SourceStatus::NotFound {
                let now = Instant::now();
                // Only an actual HTTP 404 is a negative result; transient failures never are.
                insert_cache(&mut state, url, Cached {
                    body: None, observed_at: received_at, status: SourceStatus::NotFound,
                    fresh_until: now + Duration::from_secs(300),
                    stale_until: now + Duration::from_secs(300),
                    last_used: now, bytes: 0,
                });
                return Acquisition {
                    body: None, observed_at: received_at, status: SourceStatus::NotFound,
                };
            }
            if matches!(failure.status, SourceStatus::RateLimited | SourceStatus::Unavailable) {
                let wait = failure.retry_after.unwrap_or_else(|| {
                    if failure.status == SourceStatus::RateLimited { Duration::from_secs(60) }
                    else { Duration::from_secs(30) }
                });
                let now = Instant::now();
                let until = now.checked_add(wait).unwrap_or(now + Duration::from_secs(86_400));
                state.cooldown = Some((until, failure.status, received_at));
            }
            fallback(&state, url, failure.status, received_at, &validate)
        }
    }
}

fn fallback(
    state: &HttpState, url: &str, status: SourceStatus, failed_at: u64,
    validate: &impl Fn(&Value) -> Option<SourceStatus>,
) -> Acquisition {
    let previous = state.cache.get(url).filter(|entry| {
        entry.stale_until > Instant::now()
            && entry.body.as_ref().is_some_and(|body| validate(body).is_none())
    });
    Acquisition {
        body: previous.and_then(|entry| entry.body.clone()),
        observed_at: previous.map_or(failed_at, |entry| entry.observed_at),
        status,
    }
}

fn insert_cache(state: &mut HttpState, url: &str, entry: Cached) {
    state.cache.insert(url.to_string(), entry);
    while state.cache.len() > MAX_CACHE_ENTRIES ||
        state.cache.values().map(|entry| entry.bytes).sum::<usize>() > MAX_CACHE_BYTES {
        let Some(oldest) = state.cache.iter().min_by_key(|(_, entry)| entry.last_used)
            .map(|(key, _)| key.clone()) else { break };
        state.cache.remove(&oldest);
    }
}

struct FetchFailure {
    status: SourceStatus,
    retry_after: Option<Duration>,
}

impl FetchFailure {
    fn new(status: SourceStatus) -> Self { Self { status, retry_after: None } }
}

async fn fetch(client: &Client, url: &str, now: u64) -> Result<(Value, usize), FetchFailure> {
    let mut response = client.get(url).header("Accept", "application/json")
        .send().await.map_err(|_| FetchFailure::new(SourceStatus::Unavailable))?;
    let status = response.status();
    if !status.is_success() {
        let source_status = match status {
            StatusCode::NOT_FOUND => SourceStatus::NotFound,
            StatusCode::TOO_MANY_REQUESTS => SourceStatus::RateLimited,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => SourceStatus::Denied,
            _ => SourceStatus::Unavailable,
        };
        let retry_after = response.headers().get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok()).and_then(|value| parse_retry_after(value, now));
        return Err(FetchFailure { status: source_status, retry_after });
    }
    if response.content_length().is_some_and(|length| length > MAX_BODY_BYTES as u64) {
        return Err(FetchFailure::new(SourceStatus::InvalidResponse));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await
        .map_err(|_| FetchFailure::new(SourceStatus::Unavailable))? {
        if bytes.len().saturating_add(chunk.len()) > MAX_BODY_BYTES {
            return Err(FetchFailure::new(SourceStatus::InvalidResponse));
        }
        bytes.extend_from_slice(&chunk);
    }
    let value = serde_json::from_slice(&bytes)
        .map_err(|_| FetchFailure::new(SourceStatus::InvalidResponse))?;
    Ok((value, bytes.len()))
}

fn parse_retry_after(value: &str, now: u64) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds.max(1)));
    }
    // HTTP's current IMF-fixdate form. Invalid dates fall back to the conservative service cooldown.
    let fields: Vec<_> = value.split_ascii_whitespace().collect();
    if fields.len() != 6 || fields[5] != "GMT" || !fields[0].ends_with(',') {
        return None;
    }
    let day = fields[1].parse::<u32>().ok()?;
    let month = match fields[2] {
        "Jan" => 1, "Feb" => 2, "Mar" => 3, "Apr" => 4, "May" => 5, "Jun" => 6,
        "Jul" => 7, "Aug" => 8, "Sep" => 9, "Oct" => 10, "Nov" => 11, "Dec" => 12,
        _ => return None,
    };
    let year = fields[3].parse::<i64>().ok()?;
    if !(1970..=9999).contains(&year) { return None; }
    let time: Vec<_> = fields[4].split(':').collect();
    if time.len() != 3 { return None; }
    let hour = time[0].parse::<u32>().ok()?;
    let minute = time[1].parse::<u32>().ok()?;
    let second = time[2].parse::<u32>().ok()?;
    if hour > 23 || minute > 59 || second > 59 { return None; }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 if leap => 29, 2 => 28, 4 | 6 | 9 | 11 => 30, _ => 31,
    };
    if day == 0 || day > days { return None; }
    let adjusted = year - i64::from(month <= 2);
    let era = adjusted.div_euclid(400);
    let year_of_era = adjusted - era * 400;
    let shifted_month = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let timestamp = u64::try_from(days).ok()?.checked_mul(86_400)?
        .checked_add(u64::from(hour * 3600 + minute * 60 + second))?;
    Some(Duration::from_secs(timestamp.saturating_sub(now).max(1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use std::{io::{Read, Write}, net::TcpListener, sync::{Arc, atomic::{AtomicUsize, Ordering}}, thread};
    use serde_json::json;

    fn server(responses: Vec<String>) -> (String, Arc<AtomicUsize>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener");
        let address = listener.local_addr().expect("test address");
        let hits = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&hits);
        let worker = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("test connection");
                stream.set_read_timeout(Some(Duration::from_secs(2))).expect("test timeout");
                let mut request = [0u8; 8192];
                let _ = stream.read(&mut request).expect("test request");
                observed.fetch_add(1, Ordering::SeqCst);
                stream.write_all(response.as_bytes()).expect("test response");
            }
        });
        (format!("http://{address}/"), hits, worker)
    }

    fn ok(body: &Value) -> String {
        let body = body.to_string();
        format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_second_lookup_uses_the_observed_public_response() {
        let body = json!({"id":"known","relations":[]});
        let (url,hits,worker) = server(vec![ok(&body)]);
        let state = Mutex::new(HttpState::default());
        let client = MusicBrainzClient::new().expect("client");
        let first = acquire(&client.client,&state,&url,Provider::MusicBrainz, |_| None).await;
        let second = acquire(&client.client,&state,&url,Provider::MusicBrainz, |_| None).await;
        assert_eq!(first.body,Some(body.clone()));
        assert_eq!(second.body,Some(body));
        assert_eq!(second.observed_at,first.observed_at);
        assert_eq!(hits.load(Ordering::SeqCst),1);
        worker.join().expect("server");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failed_refresh_preserves_old_evidence_and_honors_retry_after() {
        let body = json!({"id":"known","relations":[]});
        let unavailable = "HTTP/1.1 503 Service Unavailable\r\nRetry-After: 120\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string();
        let (url,hits,worker) = server(vec![ok(&body),unavailable]);
        let state = Mutex::new(HttpState::default());
        let client = MusicBrainzClient::new().expect("client");
        let first = acquire(&client.client,&state,&url,Provider::MusicBrainz, |_| None).await;
        {
            let mut state = state.lock().await;
            state.cache.get_mut(&url).expect("cached").fresh_until = Instant::now();
            state.next_request = None;
        }
        let stale = acquire(&client.client,&state,&url,Provider::MusicBrainz, |_| None).await;
        assert_eq!(stale.status,SourceStatus::Unavailable);
        assert_eq!(stale.body,Some(body));
        assert_eq!(stale.observed_at,first.observed_at);
        assert_eq!(hits.load(Ordering::SeqCst),2);
        let gate = state.lock().await;
        assert!(gate.cooldown.expect("retry cooldown").0 > Instant::now() + Duration::from_secs(115));
        worker.join().expect("server");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failure_is_not_negative_cached_but_real_not_found_is() {
        let response = "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string();
        let (url,_,worker) = server(vec![response]);
        let state = Mutex::new(HttpState::default());
        let client = MusicBrainzClient::new().expect("client");
        let failed = acquire(&client.client,&state,&url,Provider::MusicBrainz, |_| None).await;
        assert_eq!(failed.status,SourceStatus::RateLimited);
        assert!(state.lock().await.cache.is_empty());
        worker.join().expect("server");
        let (url,hits,worker) = server(vec![
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
        ]);
        let state = Mutex::new(HttpState::default());
        let first = acquire(&client.client,&state,&url,Provider::MusicBrainz, |_| None).await;
        let second = acquire(&client.client,&state,&url,Provider::MusicBrainz, |_| None).await;
        assert_eq!(first.status,SourceStatus::NotFound);
        assert_eq!(second.status,SourceStatus::NotFound);
        assert!(second.body.is_none());
        assert_eq!(hits.load(Ordering::SeqCst),1);
        worker.join().expect("server");
    }



    #[tokio::test(flavor = "current_thread")]
    async fn cache_never_reuses_an_audio_document_as_a_video_identity() {
        let body = json!({"id":"known","title":"Audio recording","video":false});
        let unavailable = "HTTP/1.1 503 Service Unavailable\r\nRetry-After: 120\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string();
        let (url,hits,worker) = server(vec![ok(&body),unavailable]);
        let state = Mutex::new(HttpState::default());
        let client = MusicBrainzClient::new().expect("client");
        let first = acquire(&client.client,&state,&url,Provider::MusicBrainz, |_| None).await;
        let video = |value:&Value| {
            (value["video"].as_bool()!=Some(true)).then_some(SourceStatus::IdentityMismatch)
        };
        let wrong_kind = acquire(&client.client,&state,&url,Provider::MusicBrainz,video).await;
        assert_eq!(wrong_kind.status,SourceStatus::IdentityMismatch);
        assert!(wrong_kind.body.is_none());
        assert_eq!(hits.load(Ordering::SeqCst),1);
        {
            let mut gate=state.lock().await;
            gate.cache.get_mut(&url).expect("cached").fresh_until=Instant::now();
            gate.next_request=None;
        }
        let failed_video = acquire(&client.client,&state,&url,Provider::MusicBrainz,video).await;
        assert_eq!(failed_video.status,SourceStatus::Unavailable);
        assert!(failed_video.body.is_none());
        let stale_audio = acquire(&client.client,&state,&url,Provider::MusicBrainz, |_| None).await;
        assert_eq!(stale_audio.status,SourceStatus::Unavailable);
        assert_eq!(stale_audio.body,Some(body));
        assert_eq!(stale_audio.observed_at,first.observed_at);
        assert_eq!(hits.load(Ordering::SeqCst),2);
        worker.join().expect("server");
    }

    #[test]
    fn public_search_escapes_query_operators_but_keeps_artist_spelling() {
        assert_eq!(search_query("AC/DC"), r"AC\/DC");
        assert_eq!(search_query("Beyoncé"), "Beyoncé");
        assert_eq!(search_query("Artist: title?"), r"Artist\: title\?");
    }

    #[test]
    fn retry_after_accepts_delta_and_http_date_and_rejects_invalid_dates() {
        assert_eq!(parse_retry_after("120",0),Some(Duration::from_secs(120)));
        assert_eq!(parse_retry_after("Thu, 01 Jan 1970 00:02:00 GMT",0),Some(Duration::from_secs(120)));
        assert_eq!(parse_retry_after("Fri, 30 Feb 2026 00:00:00 GMT",0),None);
        assert_eq!(parse_retry_after("junk",0),None);
    }
}
