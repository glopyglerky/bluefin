//! Authenticated Sports reads from fixed public-provider origins.
//! Account preferences and local broadcast playback keep their existing authority.
use crate::{auth::RequireAuth, error::ApiError, extract::Query, state::AppState};
use axum::{
    Json, Router,
    extract::Path,
    http::header,
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::{Datelike, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{Duration, Instant},
};

#[path = "sports_feed/mapping.rs"]
mod mapping;

const JSON_LIMIT: usize = 4 * 1024 * 1024;
const IMAGE_LIMIT: usize = 1024 * 1024;
const STALE_LIMIT: Duration = Duration::from_secs(30 * 60);
const ASSET_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const DEADLINE: Duration = Duration::from_secs(6);

#[derive(Debug)]
enum Kind {
    Scoreboard,
    Event(String),
    Standings,
}
struct Endpoint {
    league: String,
    key: String,
    url: String,
    kind: Kind,
    fresh: Duration,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScoreboardQuery {
    league: String,
    date: Option<String>,
}
impl crate::extract::QueryParameters for ScoreboardQuery {
    const COLLECTIONS: &'static [(&'static str, char)] = &[];
}
fn unavailable() -> ApiError {
    ApiError::ServiceUnavailable("Sports provider temporarily unavailable".into())
}
fn league_path(league: &str) -> Result<&'static str, ApiError> {
    match league {
        "nhl" => Ok("hockey/nhl"),
        "nfl" => Ok("football/nfl"),
        "nba" => Ok("basketball/nba"),
        "mlb" => Ok("baseball/mlb"),
        "wnba" => Ok("basketball/wnba"),
        "epl" => Ok("soccer/eng.1"),
        "mls" => Ok("soccer/usa.1"),
        _ => Err(ApiError::BadRequest("unsupported Sports league".into())),
    }
}
fn calendar_date(raw: &str) -> Result<NaiveDate, ApiError> {
    let invalid = || ApiError::BadRequest("date must be a calendar date YYYY-MM-DD".into());
    if raw.len() != 10
        || !raw.bytes().enumerate().all(|(index, byte)| {
            if index == 4 || index == 7 {
                byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        })
    {
        return Err(invalid());
    }
    let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|_| invalid())?;
    if !(1900..=2199).contains(&date.year()) {
        return Err(invalid());
    }
    Ok(date)
}
fn valid_event_id(raw: &str) -> bool {
    !raw.is_empty() && raw.len() <= 20 && raw.bytes().all(|byte| byte.is_ascii_digit())
}
fn logo_url(raw: &str) -> Option<String> {
    if raw.len() > 512 {
        return None;
    }
    let url = url::Url::parse(raw).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("a.espncdn.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with("/i/teamlogos/")
        || !url.path().ends_with(".png")
        || !url
            .path()
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.'))
    {
        return None;
    }
    Some(url.to_string())
}
fn logo_id(url: &str) -> String {
    Sha256::digest(url.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn valid_logo_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn valid_png(bytes: &[u8]) -> bool {
    if bytes.len() < 33 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return false;
    }
    let dimension = |start| u32::from_be_bytes(bytes[start..start + 4].try_into().unwrap());
    (1..=2048).contains(&dimension(16)) && (1..=2048).contains(&dimension(20))
}

#[derive(Clone)]
struct Cached<T> {
    value: T,
    created: Instant,
    accessed: Instant,
    fetched_at: String,
    bytes: usize,
}
struct Cache<T> {
    entries: HashMap<String, Cached<T>>,
    bytes: usize,
    max_entries: usize,
    max_bytes: usize,
}
impl<T: Clone> Cache<T> {
    fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            bytes: 0,
            max_entries,
            max_bytes,
        }
    }
    fn remove(&mut self, key: &str) {
        if let Some(entry) = self.entries.remove(key) {
            self.bytes -= entry.bytes;
        }
    }
    fn get(&mut self, key: &str, max_age: Duration) -> Option<Cached<T>> {
        if self
            .entries
            .get(key)
            .is_some_and(|entry| entry.created.elapsed() > max_age)
        {
            self.remove(key);
        }
        let entry = self.entries.get_mut(key)?;
        entry.accessed = Instant::now();
        Some(entry.clone())
    }
    fn put(&mut self, key: String, value: T, bytes: usize) -> Result<Cached<T>, ApiError> {
        if bytes > self.max_bytes {
            return Err(unavailable());
        }
        self.remove(&key);
        while self.entries.len() >= self.max_entries || self.bytes + bytes > self.max_bytes {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.accessed)
                .map(|(key, _)| key.clone())
                .ok_or_else(unavailable)?;
            self.remove(&oldest);
        }
        let entry = Cached {
            value,
            created: Instant::now(),
            accessed: Instant::now(),
            fetched_at: Utc::now().to_rfc3339(),
            bytes,
        };
        self.bytes += bytes;
        self.entries.insert(key, entry.clone());
        Ok(entry)
    }
}
type Gate = tokio::sync::Mutex<()>;
struct Provider {
    client: reqwest::Client,
    feeds: Mutex<Cache<Arc<Value>>>,
    asset_urls: Mutex<Cache<String>>,
    images: Mutex<Cache<Arc<Vec<u8>>>>,
    gates: Mutex<HashMap<String, Weak<Gate>>>,
    feed_network: tokio::sync::Semaphore,
    image_network: tokio::sync::Semaphore,
}
fn lock<T>(value: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, ApiError> {
    value.lock().map_err(|_| unavailable())
}
impl Provider {
    fn new() -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: reqwest::Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(2))
                .timeout(DEADLINE)
                .user_agent("Bluefin-Sports/1")
                .build()?,
            feeds: Mutex::new(Cache::new(48, 16 * 1024 * 1024)),
            asset_urls: Mutex::new(Cache::new(1024, 1024 * 1024)),
            images: Mutex::new(Cache::new(256, 16 * 1024 * 1024)),
            gates: Mutex::new(HashMap::new()),
            feed_network: tokio::sync::Semaphore::new(4),
            image_network: tokio::sync::Semaphore::new(6),
        })
    }
    fn gate(&self, key: &str) -> Result<Arc<Gate>, ApiError> {
        let mut gates = lock(&self.gates)?;
        gates.retain(|_, gate| gate.strong_count() > 0);
        if let Some(gate) = gates.get(key).and_then(Weak::upgrade) {
            return Ok(gate);
        }
        if gates.len() >= 64 {
            return Err(unavailable());
        }
        let gate = Arc::new(Gate::new(()));
        gates.insert(key.to_owned(), Arc::downgrade(&gate));
        Ok(gate)
    }
    async fn bytes(
        &self,
        url: &str,
        limit: usize,
        content_type: &str,
    ) -> Result<Vec<u8>, ApiError> {
        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| unavailable())?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|size| size > limit as u64)
            || response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next())
                .is_none_or(|value| value.trim() != content_type)
        {
            return Err(unavailable());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(unavailable());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
    fn cached(&self, key: &str) -> Result<Option<Cached<Arc<Value>>>, ApiError> {
        Ok(lock(&self.feeds)?.get(key, STALE_LIMIT))
    }
    async fn refresh(&self, endpoint: &Endpoint) -> Result<Cached<Arc<Value>>, ApiError> {
        let _permit = tokio::time::timeout(DEADLINE, self.feed_network.acquire())
            .await
            .map_err(|_| unavailable())?
            .map_err(|_| unavailable())?;
        let bytes = self
            .bytes(&endpoint.url, JSON_LIMIT, "application/json")
            .await?;
        let raw = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
        let mapped = mapping::normalize(&endpoint.league, &endpoint.kind, &raw)
            .map_err(|_| unavailable())?;
        let size = serde_json::to_vec(&mapped.payload)
            .map_err(|_| unavailable())?
            .len();
        if size > JSON_LIMIT {
            return Err(unavailable());
        }
        {
            let mut assets = lock(&self.asset_urls)?;
            for (id, url) in mapped.logos {
                let bytes = url.len() + id.len();
                assets.put(id, url, bytes)?;
            }
        }
        lock(&self.feeds)?.put(endpoint.key.clone(), Arc::new(mapped.payload), size)
    }
    async fn feed(&self, endpoint: Endpoint) -> Result<Response, ApiError> {
        let cached = self.cached(&endpoint.key)?;
        if let Some(cached) = cached
            .as_ref()
            .filter(|cached| cached.created.elapsed() < endpoint.fresh)
        {
            return envelope(cached, &endpoint.league, false);
        }
        let gate = self.gate(&endpoint.key)?;
        let _guard = match tokio::time::timeout(DEADLINE, gate.lock()).await {
            Ok(guard) => guard,
            Err(_) => return stale(cached.as_ref(), &endpoint.league),
        };
        // A waiting request reuses the preceding request's completed fetch.
        let cached = self.cached(&endpoint.key)?;
        if let Some(cached) = cached
            .as_ref()
            .filter(|cached| cached.created.elapsed() < endpoint.fresh)
        {
            return envelope(cached, &endpoint.league, false);
        }
        match self.refresh(&endpoint).await {
            Ok(value) => envelope(&value, &endpoint.league, false),
            Err(_) => stale(cached.as_ref(), &endpoint.league),
        }
    }
    async fn logo(&self, id: &str) -> Result<Response, ApiError> {
        if let Some(cached) = lock(&self.images)?.get(id, ASSET_TTL) {
            return Ok(image_response(&cached.value));
        }
        let url = lock(&self.asset_urls)?
            .get(id, ASSET_TTL)
            .ok_or_else(|| ApiError::NotFound("Sports artwork unavailable".into()))?
            .value;
        let gate = self.gate(&format!("logo:{id}"))?;
        let _guard = tokio::time::timeout(DEADLINE, gate.lock())
            .await
            .map_err(|_| unavailable())?;
        if let Some(cached) = lock(&self.images)?.get(id, ASSET_TTL) {
            return Ok(image_response(&cached.value));
        }
        let _permit = tokio::time::timeout(DEADLINE, self.image_network.acquire())
            .await
            .map_err(|_| unavailable())?
            .map_err(|_| unavailable())?;
        let bytes = self.bytes(&url, IMAGE_LIMIT, "image/png").await?;
        if !valid_png(&bytes) {
            return Err(unavailable());
        }
        let size = bytes.len();
        let bytes = Arc::new(bytes);
        lock(&self.images)?.put(id.to_owned(), Arc::clone(&bytes), size)?;
        Ok(image_response(&bytes))
    }
}
fn provider() -> Result<&'static Provider, ApiError> {
    static PROVIDER: OnceLock<Result<Provider, reqwest::Error>> = OnceLock::new();
    PROVIDER
        .get_or_init(Provider::new)
        .as_ref()
        .map_err(|_| unavailable())
}
fn envelope(
    cached: &Cached<Arc<Value>>,
    league: &str,
    is_stale: bool,
) -> Result<Response, ApiError> {
    let mut value = cached.value.as_ref().clone();
    let object = value.as_object_mut().ok_or_else(unavailable)?;
    object.insert("schema".into(), Value::from(1));
    object.insert("provider".into(), Value::from("ESPN"));
    object.insert("fetchedAt".into(), Value::from(cached.fetched_at.clone()));
    object.insert("stale".into(), Value::from(is_stale));
    object.insert("league".into(), Value::from(league));
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response())
}
fn stale(cached: Option<&Cached<Arc<Value>>>, league: &str) -> Result<Response, ApiError> {
    let cached = cached
        .filter(|cached| cached.created.elapsed() <= STALE_LIMIT)
        .ok_or_else(unavailable)?;
    envelope(cached, league, true)
}
fn image_response(bytes: &Arc<Vec<u8>>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "private, max-age=3600"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes.as_ref().clone(),
    )
        .into_response()
}
async fn scoreboard(
    RequireAuth(auth): RequireAuth,
    Query(query): Query<ScoreboardQuery>,
) -> Result<Response, ApiError> {
    super::viewer(&auth)?;
    let path = league_path(&query.league)?;
    let date = query
        .date
        .as_deref()
        .map(calendar_date)
        .transpose()?
        .unwrap_or_else(|| Utc::now().date_naive());
    provider()?
        .feed(Endpoint {
            key: format!("scoreboard:{}:{date}", query.league),
            url: format!(
                "https://site.api.espn.com/apis/site/v2/sports/{path}/scoreboard?dates={}",
                date.format("%Y%m%d")
            ),
            league: query.league,
            kind: Kind::Scoreboard,
            fresh: Duration::from_secs(30),
        })
        .await
}
async fn event(
    RequireAuth(auth): RequireAuth,
    Path((league, id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    super::viewer(&auth)?;
    let path = league_path(&league)?;
    if !valid_event_id(&id) {
        return Err(ApiError::BadRequest("invalid Sports event".into()));
    }
    provider()?
        .feed(Endpoint {
            key: format!("event:{league}:{id}"),
            url: format!("https://site.api.espn.com/apis/site/v2/sports/{path}/summary?event={id}"),
            league,
            kind: Kind::Event(id),
            fresh: Duration::from_secs(30),
        })
        .await
}
async fn standings(
    RequireAuth(auth): RequireAuth,
    Path(league): Path<String>,
) -> Result<Response, ApiError> {
    super::viewer(&auth)?;
    let path = league_path(&league)?;
    provider()?
        .feed(Endpoint {
            key: format!("standings:{league}"),
            url: format!("https://site.api.espn.com/apis/v2/sports/{path}/standings"),
            league,
            kind: Kind::Standings,
            fresh: Duration::from_secs(300),
        })
        .await
}
async fn logo(
    RequireAuth(auth): RequireAuth,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    super::viewer(&auth)?;
    if !valid_logo_id(&id) {
        return Err(ApiError::BadRequest("invalid Sports artwork".into()));
    }
    provider()?.logo(&id).await
}
pub(super) fn register(router: Router<AppState>) -> Router<AppState> {
    router
        .route("/sports/api/v1/scoreboard", get(scoreboard))
        .route("/sports/api/v1/events/{league}/{id}", get(event))
        .route("/sports/api/v1/standings/{league}", get(standings))
        .route("/sports/api/v1/logos/{id}", get(logo))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_inputs_cannot_select_arbitrary_origins_or_dates() {
        for league in ["nhl", "nfl", "nba", "mlb", "wnba", "epl", "mls"] {
            assert!(league_path(league).is_ok());
        }
        for league in [
            "",
            "NHL",
            "../nfl",
            "nhl?url=https://internal",
            "https://internal",
        ] {
            assert!(league_path(league).is_err());
        }
        assert!(calendar_date("2026-11-27").is_ok());
        assert!(calendar_date("2024-02-29").is_ok());
        for date in [
            "2026-02-29",
            "2026-2-01",
            "2026-11-27&x=1",
            "20261127",
            "0000-01-01",
        ] {
            assert!(calendar_date(date).is_err());
        }
        for id in ["123", "401892781"] {
            assert!(valid_event_id(id));
        }
        for id in ["", "-1", "1?x=2", "../1", "1".repeat(21).as_str()] {
            assert!(!valid_event_id(id));
        }
    }
    #[test]
    fn only_bounded_primary_team_png_assets_are_admitted() {
        let good = "https://a.espncdn.com/i/teamlogos/nhl/500/utah.png";
        assert_eq!(logo_url(good).as_deref(), Some(good));
        assert!(valid_logo_id(&logo_id(good)));
        for url in [
            "http://a.espncdn.com/i/teamlogos/nhl/500/a.png",
            "https://a.espncdn.com.attacker.test/i/teamlogos/a.png",
            "https://a.espncdn.com@internal/i/teamlogos/a.png",
            "https://a.espncdn.com:444/i/teamlogos/a.png",
            "https://a.espncdn.com/i/teamlogos/a.svg",
            "https://a.espncdn.com/i/teamlogos/a.png?target=internal",
            "https://a.espncdn.com/guid/a.png",
        ] {
            assert!(logo_url(url).is_none(), "{url}");
        }
        let mut png = vec![0; 33];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[12..16].copy_from_slice(b"IHDR");
        png[16..20].copy_from_slice(&500u32.to_be_bytes());
        png[20..24].copy_from_slice(&500u32.to_be_bytes());
        assert!(valid_png(&png));
        png[16..20].copy_from_slice(&10000u32.to_be_bytes());
        assert!(!valid_png(&png));
        assert!(!valid_png(b"<html>not an image</html>"));
    }
    #[test]
    fn shared_public_cache_respects_byte_count_entry_count_and_expiry() {
        let mut cache = Cache::new(2, 12);
        cache.put("first".into(), 1, 6).unwrap();
        cache.put("second".into(), 2, 6).unwrap();
        assert_eq!(cache.get("first", ASSET_TTL).unwrap().value, 1);
        cache.put("third".into(), 3, 6).unwrap();
        assert!(cache.get("second", ASSET_TTL).is_none());
        assert_eq!(cache.bytes, 12);
        assert_eq!(cache.entries.len(), 2);
        assert!(cache.put("oversized".into(), 4, 13).is_err());
        cache.entries.get_mut("third").unwrap().created = Instant::now() - Duration::from_secs(3);
        assert!(cache.get("third", Duration::from_secs(1)).is_none());
        assert_eq!(cache.bytes, 6);
    }
}
