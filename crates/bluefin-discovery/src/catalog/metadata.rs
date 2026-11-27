//! Public catalog metadata, composed through the existing discovery-owned clients.
//! Adapted from the specialist metadata contract; no legacy offers or private runtime is imported.
use super::{CatalogService, cache_entry, timestamp};
use bluefin_catalog::{
    Attribution, CatalogId as AccessId, CatalogIdentity, Coverage, CoverageScope, MediaKind,
    SourceObservation, SourceStatus,
};
use bluefin_contract::public_catalog::{
    CatalogArtwork, CatalogCredit, CatalogFailure, CatalogId, CatalogKind, CatalogPage,
    CatalogRecord,
};
use serde_json::Value;
use std::{collections::HashSet, future::Future, pin::Pin, time::Duration};
use uuid::Uuid;

const METADATA_TTL: Duration = Duration::from_secs(900);
const MAX_METADATA_CACHE: usize = 128;
const MAX_CREDITS: usize = 4096;
type MetadataResult<T> = std::result::Result<T, CatalogFailure>;

/// Acquisition result from a configured process-owned reader. It grants no library access.
#[derive(Clone, Debug)]
pub struct MetadataDocument {
    pub body: Option<Value>,
    pub observed_at: u64,
    pub status: SourceStatus,
}

impl MetadataDocument {
    pub fn failure(status: SourceStatus) -> Self {
        Self {
            body: None,
            observed_at: timestamp(),
            status,
        }
    }
}

impl From<bluefin_catalog::http::TmdbDocument> for MetadataDocument {
    fn from(value: bluefin_catalog::http::TmdbDocument) -> Self {
        Self {
            body: value.body,
            observed_at: value.observed_at,
            status: value.status,
        }
    }
}

impl From<bluefin_catalog::music_http::MusicBrainzDocument> for MetadataDocument {
    fn from(value: bluefin_catalog::music_http::MusicBrainzDocument) -> Self {
        Self {
            body: value.body,
            observed_at: value.observed_at,
            status: value.status,
        }
    }
}

pub type MetadataFuture<'a> = Pin<Box<dyn Future<Output = MetadataDocument> + Send + 'a>>;

/// Reads public source documents through the server's existing credentials and quotas.
pub trait MetadataReader: Send + Sync {
    fn search<'a>(&'a self, kind: CatalogKind, term: &'a str, page: u32) -> MetadataFuture<'a>;
    fn details<'a>(&'a self, identity: &'a CatalogIdentity) -> MetadataFuture<'a>;
}

struct Document {
    value: Value,
    sources: Vec<SourceObservation>,
    seerr: bool,
}
impl CatalogService {
    /// Searches a source-owned catalog before any library pairing is required.
    pub async fn metadata_search(
        &self,
        kind: CatalogKind,
        term: &str,
        page: u32,
    ) -> MetadataResult<CatalogPage> {
        let term = term.trim();
        if !(2..=160).contains(&term.chars().count())
            || term.chars().any(char::is_control)
            || !(1..=100).contains(&page)
        {
            return Err(failure(vec![source(
                "catalog",
                SourceStatus::UnsupportedIdentity,
                timestamp(),
            )]));
        }
        let key = serde_json::to_string(&(kind, term, page)).map_err(|_| {
            failure(vec![source(
                "catalog",
                SourceStatus::InvalidResponse,
                timestamp(),
            )])
        })?;
        let cell = cache_entry(&self.metadata_pages, &key, METADATA_TTL, MAX_METADATA_CACHE).await;
        let result = cell
            .get_or_init(|| async {
                let _permit = match tokio::time::timeout(
                    Duration::from_secs(3),
                    self.network.acquire(),
                )
                .await
                {
                    Ok(Ok(permit)) => permit,
                    _ => {
                        return Err(failure(vec![source(
                            "catalog",
                            SourceStatus::RateLimited,
                            timestamp(),
                        )]));
                    }
                };
                if video_kind(kind).is_some() {
                    let document = self.video_search_document(kind, term, page).await?;
                    project_video_page(kind, page, document)
                } else {
                    let media_kind = music_kind(kind);
                    if !matches!(
                        media_kind,
                        Some(MediaKind::Artist | MediaKind::ReleaseGroup | MediaKind::Recording)
                    ) {
                        return Err(failure(vec![source(
                            "musicbrainz",
                            SourceStatus::UnsupportedIdentity,
                            timestamp(),
                        )]));
                    }
                    let document = if let Some(reader) = &self.metadata_reader {
                        reader.search(kind, term, page).await
                    } else {
                        self.music
                            .search(
                                media_kind.unwrap_or(MediaKind::Artist),
                                term,
                                u64::from(page - 1) * 20,
                            )
                            .await
                            .into()
                    };
                    let sources =
                        vec![source("musicbrainz", document.status, document.observed_at)];
                    let value = document.body.ok_or_else(|| failure(sources.clone()))?;
                    project_music_page(kind, page, value, sources)
                }
            })
            .await
            .clone();
        if result
            .as_ref()
            .map_or(true, |page| transient(&page.sources))
        {
            self.metadata_pages.lock().await.remove(&key);
        }
        result
    }

    /// Retrieves exact typed metadata, retaining public artwork, all supplied credits and their identities.
    pub async fn metadata_details(
        &self,
        kind: CatalogKind,
        id: &str,
    ) -> MetadataResult<CatalogRecord> {
        let identity = identity(kind, id).ok_or_else(|| {
            failure(vec![source(
                "catalog",
                SourceStatus::UnsupportedIdentity,
                timestamp(),
            )])
        })?;
        let key = serde_json::to_string(&(kind, &identity)).map_err(|_| {
            failure(vec![source(
                "catalog",
                SourceStatus::InvalidResponse,
                timestamp(),
            )])
        })?;
        let cell = cache_entry(
            &self.metadata_records,
            &key,
            METADATA_TTL,
            MAX_METADATA_CACHE,
        )
        .await;
        let result = cell
            .get_or_init(|| async {
                let _permit = match tokio::time::timeout(
                    Duration::from_secs(3),
                    self.network.acquire(),
                )
                .await
                {
                    Ok(Ok(permit)) => permit,
                    _ => {
                        return Err(failure(vec![source(
                            "catalog",
                            SourceStatus::RateLimited,
                            timestamp(),
                        )]));
                    }
                };
                if video_kind(kind).is_some() {
                    let document = self.video_details_document(&identity).await?;
                    let mut result = video_record(kind, &document.value)
                        .ok_or_else(|| invalid_document(&document.sources))?;
                    result.sources = document.sources;
                    if result.credits_truncated {
                        mark_partial(&mut result.sources);
                    }
                    Ok(result)
                } else {
                    let document = if let Some(reader) = &self.metadata_reader {
                        reader.details(&identity).await
                    } else {
                        self.music.metadata(&identity).await.into()
                    };
                    let mut sources =
                        vec![source("musicbrainz", document.status, document.observed_at)];
                    let value = document.body.ok_or_else(|| failure(sources.clone()))?;
                    if value["id"].as_str().and_then(|id| Uuid::parse_str(id).ok())
                        != identity
                            .id("musicbrainz")
                            .and_then(|id| Uuid::parse_str(id).ok())
                        || kind == CatalogKind::MusicVideo && value["video"].as_bool() != Some(true)
                    {
                        sources.push(source(
                            "musicbrainz",
                            SourceStatus::IdentityMismatch,
                            document.observed_at,
                        ));
                        return Err(failure(sources));
                    }
                    let mut record =
                        music_record(kind, &value).ok_or_else(|| invalid_document(&sources))?;
                    record.sources = sources;
                    if record.credits_truncated {
                        mark_partial(&mut record.sources);
                    }
                    Ok(record)
                }
            })
            .await
            .clone();
        if result
            .as_ref()
            .map_or(true, |record| transient(&record.sources))
        {
            self.metadata_records.lock().await.remove(&key);
        }
        result
    }

    async fn video_search_document(
        &self,
        kind: CatalogKind,
        term: &str,
        page: u32,
    ) -> MetadataResult<Document> {
        let media_kind = video_kind(kind).ok_or_else(|| {
            failure(vec![source(
                "catalog",
                SourceStatus::UnsupportedIdentity,
                timestamp(),
            )])
        })?;
        let mut direct: MetadataDocument = if !self.configured_tmdb {
            if let Some(reader) = &self.metadata_reader {
                reader.search(kind, term, page).await
            } else {
                self.direct.metadata_search(media_kind, term, page).await.into()
            }
        } else {
            self.direct.metadata_search(media_kind, term, page).await.into()
        };
        if direct.body.as_ref().is_some_and(|body| {
            body["page"].as_u64() != Some(u64::from(page))
                || body["results"].as_array().is_none()
                || body["total_pages"].as_u64().is_none()
        }) {
            direct.body = None;
            direct.status = SourceStatus::InvalidResponse;
        }
        let mut sources = vec![source("tmdb", direct.status, direct.observed_at)];
        if let Some(value) = direct.body {
            return Ok(Document {
                value,
                sources,
                seerr: false,
            });
        }
        if !fallback_allowed(direct.status) {
            return Err(failure(sources));
        }
        let Some(seerr) = &self.seerr else {
            return Err(failure(sources));
        };
        match seerr.metadata_search(term, page).await {
            Ok(document) => {
                sources.push(source(
                    "seerr",
                    SourceStatus::Complete,
                    document.observed_at,
                ));
                Ok(Document {
                    value: document.value,
                    sources,
                    seerr: true,
                })
            }
            Err(status) => {
                sources.push(source("seerr", status, timestamp()));
                Err(failure(sources))
            }
        }
    }
    async fn video_details_document(&self, identity: &CatalogIdentity) -> MetadataResult<Document> {
        let mut direct: MetadataDocument = if !self.configured_tmdb {
            if let Some(reader) = &self.metadata_reader {
                reader.details(identity).await
            } else {
                self.direct.metadata(identity).await.into()
            }
        } else {
            self.direct.metadata(identity).await.into()
        };
        if let Some(body) = direct.body.as_ref() {
            let expected = identity.id("tmdb")
                .and_then(|id| id.rsplit('/').next())
                .and_then(|id| id.parse::<u64>().ok());
            let actual = body["id"].as_u64();
            if actual.is_none() || actual != expected {
                direct.status = if actual.is_some() {
                    SourceStatus::IdentityMismatch
                } else {
                    SourceStatus::InvalidResponse
                };
                direct.body = None;
            }
        }
        let mut sources = vec![source("tmdb", direct.status, direct.observed_at)];
        if let Some(value) = direct.body {
            return Ok(Document {
                value,
                sources,
                seerr: false,
            });
        }
        if !fallback_allowed(direct.status) {
            return Err(failure(sources));
        }
        let Some(seerr) = &self.seerr else {
            return Err(failure(sources));
        };
        let id = identity
            .id("tmdb")
            .and_then(|id| id.rsplit('/').next())
            .and_then(|id| id.parse::<u64>().ok())
            .ok_or_else(|| failure(sources.clone()))?;
        match seerr.metadata_details(identity.kind, id).await {
            Ok(document) => {
                sources.push(source(
                    "seerr",
                    if document.credits_status.is_some() {
                        SourceStatus::Partial
                    } else {
                        SourceStatus::Complete
                    },
                    document.observed_at,
                ));
                if let Some(status) = document.credits_status {
                    sources.push(source("seerrCredits", status, document.observed_at));
                }
                Ok(Document {
                    value: document.value,
                    sources,
                    seerr: true,
                })
            }
            Err(status) => {
                sources.push(source("seerr", status, timestamp()));
                Err(failure(sources))
            }
        }
    }
}

fn fallback_allowed(status: SourceStatus) -> bool {
    matches!(
        status,
        SourceStatus::MissingCredentials
            | SourceStatus::Denied
            | SourceStatus::RateLimited
            | SourceStatus::Unavailable
            | SourceStatus::InvalidResponse
    )
}
fn transient(sources: &[SourceObservation]) -> bool {
    sources.iter().any(|source| {
        matches!(
            source.status,
            SourceStatus::Unavailable
                | SourceStatus::RateLimited
                | SourceStatus::InvalidResponse
                | SourceStatus::IdentityMismatch
        )
    })
}
fn source(name: &str, status: SourceStatus, observed_at: u64) -> SourceObservation {
    let attributions = if name == "musicbrainz" {
        vec![Attribution {
            name: "MusicBrainz".into(),
            url: "https://musicbrainz.org/".into(),
        }]
    } else if name == "catalog" {
        Vec::new()
    } else {
        vec![Attribution {
            name: "TMDB".into(),
            url: "https://www.themoviedb.org/".into(),
        }]
    };
    SourceObservation {
        source: name.into(),
        status,
        observed_at,
        scope: CoverageScope::CatalogMetadata,
        coverage: if matches!(status, SourceStatus::Complete | SourceStatus::NoOffers) {
            Coverage::SourceCatalog
        } else if status == SourceStatus::Partial {
            Coverage::Partial
        } else {
            Coverage::Unknown
        },
        catalog_url: None,
        attributions,
    }
}
fn failure(sources: Vec<SourceObservation>) -> CatalogFailure {
    let status = sources
        .last()
        .map(|source| source.status)
        .unwrap_or(SourceStatus::Unavailable);
    let error = match status {
        SourceStatus::NotFound => "The requested catalog identity was not found",
        SourceStatus::UnsupportedIdentity => "Invalid or unsupported catalog identity or search",
        SourceStatus::IdentityMismatch => "The catalog source returned a different identity",
        SourceStatus::InvalidResponse => {
            "The catalog source returned incomplete or invalid metadata"
        }
        SourceStatus::RateLimited => "The catalog source is rate limited; try again shortly",
        SourceStatus::Denied => "The configured catalog source denied access",
        SourceStatus::MissingCredentials => "No authorized source is configured for this catalog",
        _ => "The catalog source is temporarily unavailable",
    };
    CatalogFailure {
        error: error.into(),
        sources,
    }
}
fn invalid_document(sources: &[SourceObservation]) -> CatalogFailure {
    let mut sources = sources.to_vec();
    if let Some(last) = sources.last_mut() {
        last.status = SourceStatus::InvalidResponse;
        last.coverage = Coverage::Unknown;
    }
    failure(sources)
}
fn mark_partial(sources: &mut [SourceObservation]) {
    if let Some(last) = sources.last_mut() {
        if matches!(last.status, SourceStatus::Complete | SourceStatus::NoOffers) {
            last.status = SourceStatus::Partial;
        }
        last.coverage = Coverage::Partial;
    }
}
fn video_kind(kind: CatalogKind) -> Option<MediaKind> {
    match kind {
        CatalogKind::Movie => Some(MediaKind::Movie),
        CatalogKind::Series => Some(MediaKind::Series),
        CatalogKind::Person => Some(MediaKind::Person),
        _ => None,
    }
}
fn music_kind(kind: CatalogKind) -> Option<MediaKind> {
    match kind {
        CatalogKind::Artist => Some(MediaKind::Artist),
        CatalogKind::Album => Some(MediaKind::ReleaseGroup),
        CatalogKind::Release => Some(MediaKind::Release),
        CatalogKind::Recording => Some(MediaKind::Recording),
        CatalogKind::MusicVideo => Some(MediaKind::MusicVideo),
        CatalogKind::Work => Some(MediaKind::Work),
        _ => None,
    }
}
fn identity(kind: CatalogKind, value: &str) -> Option<CatalogIdentity> {
    if let Some(media_kind) = video_kind(kind) {
        let resource = match kind {
            CatalogKind::Movie => "movie",
            CatalogKind::Series => "tv",
            _ => "person",
        };
        let raw = value.strip_prefix(&format!("{resource}/")).unwrap_or(value);
        if raw.is_empty() || raw.len() > 20 || !raw.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let id = raw.parse::<u64>().ok().filter(|id| *id > 0)?;
        Some(CatalogIdentity {
            kind: media_kind,
            ids: vec![AccessId {
                source: "tmdb".into(),
                id: format!("{resource}/{id}"),
            }],
        })
    } else {
        let kind = music_kind(kind)?;
        let id = Uuid::parse_str(value).ok().filter(|id| !id.is_nil())?;
        Some(CatalogIdentity {
            kind,
            ids: vec![AccessId {
                source: "musicbrainz".into(),
                id: id.to_string(),
            }],
        })
    }
}
fn project_video_page(
    kind: CatalogKind,
    page: u32,
    document: Document,
) -> MetadataResult<CatalogPage> {
    let mut sources = document.sources;
    if document.value["page"].as_u64() != Some(u64::from(page)) {
        return Err(invalid_document(&sources));
    }
    let Some(rows) = document.value["results"]
        .as_array()
        .filter(|rows| rows.len() <= 100)
    else {
        return Err(invalid_document(&sources));
    };
    let last = field(&document.value, "total_pages", "totalPages")
        .as_u64()
        .ok_or_else(|| invalid_document(&sources))?;
    let mut results = Vec::new();
    for row in rows {
        if document.seerr {
            let expected = match kind {
                CatalogKind::Movie => "movie",
                CatalogKind::Series => "tv",
                _ => "person",
            };
            if field(row, "media_type", "mediaType").as_str() != Some(expected) {
                continue;
            }
        }
        if let Some(mut record) = video_record(kind, row) {
            record.sources = sources.last().cloned().into_iter().collect();
            results.push(record);
        } else {
            mark_partial(&mut sources);
        }
    }
    if page == 100 && last > 100 {
        mark_partial(&mut sources);
    }
    if rows.is_empty() {
        if let Some(last) = sources.last_mut() {
            if last.status == SourceStatus::Complete {
                last.status = SourceStatus::NoOffers;
            }
        }
    }
    let total_results = if document.seerr {
        None
    } else {
        document.value["total_results"].as_u64()
    };
    Ok(CatalogPage {
        results,
        next_page: (u64::from(page) < last && page < 100).then_some(page + 1),
        total_results,
        sources,
    })
}
fn project_music_page(
    kind: CatalogKind,
    page: u32,
    value: Value,
    mut sources: Vec<SourceObservation>,
) -> MetadataResult<CatalogPage> {
    let field = match kind {
        CatalogKind::Artist => "artists",
        CatalogKind::Album => "release-groups",
        CatalogKind::Recording => "recordings",
        _ => return Err(invalid_document(&sources)),
    };
    let Some(rows) = value[field].as_array().filter(|rows| rows.len() <= 20) else {
        return Err(invalid_document(&sources));
    };
    let count = value["count"]
        .as_u64()
        .ok_or_else(|| invalid_document(&sources))?;
    let offset = u64::from(page - 1) * 20;
    if value["offset"].as_u64() != Some(offset) {
        return Err(invalid_document(&sources));
    }
    let mut results = Vec::new();
    for row in rows {
        if let Some(mut record) = music_record(kind, row) {
            record.sources = sources.clone();
            results.push(record);
        } else {
            mark_partial(&mut sources);
        }
    }
    if page == 100 && offset + 20 < count {
        mark_partial(&mut sources);
    }
    if rows.is_empty() {
        if let Some(last) = sources.last_mut() {
            if last.status == SourceStatus::Complete {
                last.status = SourceStatus::NoOffers;
            }
        }
    }
    Ok(CatalogPage {
        results,
        next_page: (offset + 20 < count && page < 100).then_some(page + 1),
        total_results: Some(count),
        sources,
    })
}

fn field<'a>(value: &'a Value, snake: &str, camel: &str) -> &'a Value {
    value
        .get(snake)
        .or_else(|| value.get(camel))
        .unwrap_or(&Value::Null)
}
fn text(value: &Value, limit: usize) -> Option<String> {
    let text = value.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    let text: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(limit)
        .collect();
    (!text.trim().is_empty()).then_some(text)
}
fn year(value: &Value) -> Option<i32> {
    let year = value.as_str()?.get(..4)?;
    if !year.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    year.parse().ok()
}
fn artwork(value: &Value, resource: &str, id: u64, width: &str) -> Option<CatalogArtwork> {
    let path = value.as_str()?.strip_prefix('/')?;
    if path.is_empty()
        || path.len() > 128
        || path.matches('.').count() != 1
        || !path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-. ".contains(&byte))
        || path.contains(' ')
    {
        return None;
    }
    Some(CatalogArtwork {
        url: format!("https://image.tmdb.org/t/p/{width}/{path}"),
        credit: "TMDB".into(),
        source_url: format!("https://www.themoviedb.org/{resource}/{id}"),
    })
}
fn video_record(kind: CatalogKind, row: &Value) -> Option<CatalogRecord> {
    let numeric = row["id"].as_u64().filter(|id| *id > 0)?;
    let (id, resource, title, date, image, description) = match kind {
        CatalogKind::Movie => (
            CatalogId::TmdbMovie(numeric),
            "movie",
            &row["title"],
            field(row, "release_date", "releaseDate"),
            field(row, "poster_path", "posterPath"),
            &row["overview"],
        ),
        CatalogKind::Series => (
            CatalogId::TmdbSeries(numeric),
            "tv",
            &row["name"],
            field(row, "first_air_date", "firstAirDate"),
            field(row, "poster_path", "posterPath"),
            &row["overview"],
        ),
        CatalogKind::Person => (
            CatalogId::TmdbPerson(numeric),
            "person",
            &row["name"],
            &row["birthday"],
            field(row, "profile_path", "profilePath"),
            &row["biography"],
        ),
        _ => return None,
    };
    let credit_source = if kind == CatalogKind::Person {
        field(row, "combined_credits", "combinedCredits")
    } else {
        &row["credits"]
    };
    let supplied = credit_source["cast"].as_array().map_or(0, Vec::len)
        + credit_source["crew"].as_array().map_or(0, Vec::len);
    let mut credits = Vec::new();
    for (section, fallback) in [("cast", "Cast"), ("crew", "Crew")] {
        for credit in credit_source[section]
            .as_array()
            .into_iter()
            .flatten()
            .take(MAX_CREDITS)
        {
            let credit = if kind == CatalogKind::Person {
                work_credit(credit, fallback)
            } else {
                person_credit(credit, fallback)
            };
            if let Some(credit) = credit {
                credits.push(credit);
            }
        }
    }
    if kind == CatalogKind::Person && credits.is_empty() {
        for credit in field(row, "known_for", "knownFor")
            .as_array()
            .into_iter()
            .flatten()
            .take(MAX_CREDITS)
        {
            if let Some(credit) = work_credit(credit, "Known for") {
                credits.push(credit);
            }
        }
    }
    if kind == CatalogKind::Series {
        for credit in field(row, "created_by", "createdBy")
            .as_array()
            .into_iter()
            .flatten()
            .take(MAX_CREDITS)
        {
            if let Some(credit) = person_credit(credit, "Creator") {
                credits.push(credit);
            }
        }
    }
    let credits_truncated = supplied > MAX_CREDITS || credits.len() > MAX_CREDITS;
    dedup_credits(&mut credits);
    credits.truncate(MAX_CREDITS);
    let imdb_id = field(row, "external_ids", "externalIds")
        .get("imdb_id")
        .or_else(|| field(row, "external_ids", "externalIds").get("imdbId"))
        .or_else(|| row.get("imdb_id"))
        .or_else(|| row.get("imdbId"))
        .and_then(Value::as_str)
        .filter(|id| {
            let prefix = if kind == CatalogKind::Person {
                "nm"
            } else {
                "tt"
            };
            id.strip_prefix(prefix).is_some_and(|digits| {
                (7..=12).contains(&digits.len()) && digits.bytes().all(|byte| byte.is_ascii_digit())
            })
        })
        .map(str::to_owned);
    Some(CatalogRecord {
        id,
        title: text(title, 256)?,
        year: year(date),
        summary: text(description, 16_000),
        artwork: artwork(image, resource, numeric, "w342"),
        imdb_id,
        credits,
        backdrop: artwork(
            field(row, "backdrop_path", "backdropPath"),
            resource,
            numeric,
            "w780",
        ),
        genres: genres(row),
        credits_truncated,
        sources: Vec::new(),
    })
}
fn person_credit(row: &Value, fallback: &str) -> Option<CatalogCredit> {
    let id = row["id"].as_u64().filter(|id| *id > 0)?;
    Some(CatalogCredit {
        id: CatalogId::TmdbPerson(id),
        name: text(&row["name"], 256)?,
        role: text(&row["character"], 256)
            .or_else(|| text(&row["job"], 256))
            .unwrap_or_else(|| fallback.into()),
        artwork: artwork(
            field(row, "profile_path", "profilePath"),
            "person",
            id,
            "w185",
        ),
        year: None,
    })
}
fn work_credit(row: &Value, fallback: &str) -> Option<CatalogCredit> {
    let id = row["id"].as_u64().filter(|id| *id > 0)?;
    let (id_type, resource, name, date) = match field(row, "media_type", "mediaType").as_str()? {
        "movie" => (
            CatalogId::TmdbMovie(id),
            "movie",
            &row["title"],
            field(row, "release_date", "releaseDate"),
        ),
        "tv" => (
            CatalogId::TmdbSeries(id),
            "tv",
            &row["name"],
            field(row, "first_air_date", "firstAirDate"),
        ),
        _ => return None,
    };
    Some(CatalogCredit {
        id: id_type,
        name: text(name, 256)?,
        role: text(&row["character"], 256)
            .or_else(|| text(&row["job"], 256))
            .unwrap_or_else(|| fallback.into()),
        artwork: artwork(
            field(row, "poster_path", "posterPath"),
            resource,
            id,
            "w185",
        ),
        year: year(date),
    })
}
fn dedup_credits(credits: &mut Vec<CatalogCredit>) {
    let mut seen = HashSet::new();
    credits.retain(|credit| seen.insert((credit.id, credit.role.clone())));
}
fn genres(row: &Value) -> Vec<String> {
    let mut seen = HashSet::new();
    row["genres"]
        .as_array()
        .into_iter()
        .flatten()
        .take(128)
        .filter_map(|genre| text(&genre["name"], 128))
        .filter(|name| seen.insert(name.clone()))
        .collect()
}
fn music_id(value: &Value) -> Option<Uuid> {
    Uuid::parse_str(value["id"].as_str()?)
        .ok()
        .filter(|id| !id.is_nil())
}
fn music_record(kind: CatalogKind, row: &Value) -> Option<CatalogRecord> {
    let uuid = music_id(row)?;
    if kind == CatalogKind::MusicVideo && row["video"].as_bool() != Some(true) {
        return None;
    }
    let id = match kind {
        CatalogKind::Artist => CatalogId::MusicBrainzArtist(uuid),
        CatalogKind::Album => CatalogId::MusicBrainzReleaseGroup(uuid),
        CatalogKind::Release => CatalogId::MusicBrainzRelease(uuid),
        CatalogKind::Recording | CatalogKind::MusicVideo => CatalogId::MusicBrainzRecording(uuid),
        CatalogKind::Work => CatalogId::MusicBrainzWork(uuid),
        _ => return None,
    };
    let mut credits = Vec::new();
    for credit in row["artist-credit"]
        .as_array()
        .into_iter()
        .flatten()
        .take(MAX_CREDITS)
    {
        let artist = &credit["artist"];
        if let (Some(id), Some(name)) = (
            music_id(artist),
            text(&credit["name"], 256).or_else(|| text(&artist["name"], 256)),
        ) {
            credits.push(CatalogCredit {
                id: CatalogId::MusicBrainzArtist(id),
                name,
                role: "Performer".into(),
                artwork: None,
                year: None,
            });
        }
    }
    for (field, group) in [("release-groups", true), ("releases", false)] {
        for release in row[field]
            .as_array()
            .into_iter()
            .flatten()
            .take(MAX_CREDITS)
        {
            if let (Some(id), Some(name)) = (music_id(release), text(&release["title"], 256)) {
                credits.push(CatalogCredit {
                    id: if group {
                        CatalogId::MusicBrainzReleaseGroup(id)
                    } else {
                        CatalogId::MusicBrainzRelease(id)
                    },
                    name,
                    role: if group {
                        "Album".into()
                    } else {
                        "Edition".into()
                    },
                    artwork: None,
                    year: year(&release["first-release-date"]).or_else(|| year(&release["date"])),
                });
            }
        }
    }
    if let Some(group) = row.get("release-group") {
        if let (Some(id), Some(name)) = (music_id(group), text(&group["title"], 256)) {
            credits.push(CatalogCredit {
                id: CatalogId::MusicBrainzReleaseGroup(id),
                name,
                role: "Album".into(),
                artwork: None,
                year: year(&group["first-release-date"]),
            });
        }
    }
    let mut supplied_tracks = 0usize;
    let mut tracks_incomplete = false;
    for medium in row["media"]
        .as_array()
        .into_iter()
        .flatten()
        .take(MAX_CREDITS)
    {
        let tracks = medium["tracks"].as_array();
        supplied_tracks = supplied_tracks.saturating_add(tracks.map_or(0, Vec::len));
        tracks_incomplete |= medium["track-count"]
            .as_u64()
            .is_some_and(|count| count > tracks.map_or(0, Vec::len) as u64);
        for track in tracks.into_iter().flatten().take(MAX_CREDITS) {
            let recording = &track["recording"];
            if let (Some(id), Some(name)) = (
                music_id(recording),
                text(&track["title"], 256).or_else(|| text(&recording["title"], 256)),
            ) {
                let number = text(&track["number"], 32).or_else(|| {
                    track["position"]
                        .as_u64()
                        .map(|position| position.to_string())
                });
                credits.push(CatalogCredit {
                    id: CatalogId::MusicBrainzRecording(id),
                    name,
                    role: number.map_or_else(|| "Track".into(), |number| format!("Track {number}")),
                    artwork: None,
                    year: None,
                });
            }
        }
    }
    let supplied = ["artist-credit", "release-groups", "releases"]
        .iter()
        .map(|field| row[*field].as_array().map_or(0, Vec::len))
        .sum::<usize>()
        .saturating_add(supplied_tracks);
    let credits_truncated = tracks_incomplete
        || supplied > MAX_CREDITS
        || credits.len() > MAX_CREDITS
        || row["release-group-count"].as_u64().is_some_and(|count| {
            count > row["release-groups"].as_array().map_or(0, Vec::len) as u64
        })
        || row["release-count"]
            .as_u64()
            .is_some_and(|count| count > row["releases"].as_array().map_or(0, Vec::len) as u64);
    dedup_credits(&mut credits);
    credits.truncate(MAX_CREDITS);
    // An unverified Cover Art Archive resolver URL is not evidence that artwork exists.
    let artwork = (kind == CatalogKind::Release
        && row["cover-art-archive"]["front"].as_bool() == Some(true))
    .then(|| CatalogArtwork {
        url: format!("https://coverartarchive.org/release/{uuid}/front-250"),
        credit: "Cover Art Archive".into(),
        source_url: format!("https://musicbrainz.org/release/{uuid}/cover-art"),
    });
    Some(CatalogRecord {
        id,
        title: text(&row["name"], 256).or_else(|| text(&row["title"], 256))?,
        year: year(&row["first-release-date"])
            .or_else(|| year(&row["date"]))
            .or_else(|| year(&row["life-span"]["begin"])),
        summary: text(&row["disambiguation"], 4000),
        artwork,
        imdb_id: None,
        credits,
        backdrop: None,
        genres: genres(row),
        credits_truncated,
        sources: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn camel_case_video_fields_keep_exact_people_art_and_all_supplied_credits() {
        let cast: Vec<Value> = (1..=32)
            .map(|id| {
                json!({
                    "id": id, "name": format!("Fixture performer {id}"),
                    "character": format!("Role {id}"), "profilePath": "/fixture.jpg"
                })
            })
            .collect();
        let record = video_record(CatalogKind::Series, &json!({
            "id": 1396, "name": "Breaking Bad", "firstAirDate": "2008-01-20",
            "posterPath": "/poster.jpg", "backdropPath": "/backdrop.jpg",
            "overview": "Public description", "genres": [{"id": 18, "name": "Drama"}],
            "externalIds": {"imdbId": "tt0903747"},
            "credits": {"cast": cast, "crew": [
                {"id": 100, "name": "Fixture writer", "job": "Writer", "profilePath": "/writer.jpg"}
            ]},
            "createdBy": [{"id": 200, "name": "Fixture creator"}],
            "mediaInfo": {"requests": [{"requestedBy": {"email": "PRIVATE_SHOULD_NOT_LEAK"}}]},
            "onUserWatchlist": true
        })).unwrap();
        assert_eq!(record.id, CatalogId::TmdbSeries(1396));
        assert_eq!(record.year, Some(2008));
        assert_eq!(record.imdb_id.as_deref(), Some("tt0903747"));
        assert_eq!(record.credits.len(), 34);
        assert_eq!(record.credits[0].id, CatalogId::TmdbPerson(1));
        assert!(record.credits.iter().any(|credit| credit.role == "Writer"));
        assert!(record.credits.iter().any(|credit| credit.role == "Creator"));
        assert_eq!(
            record.artwork.unwrap().url,
            "https://image.tmdb.org/t/p/w342/poster.jpg"
        );
        assert_eq!(
            record.backdrop.unwrap().url,
            "https://image.tmdb.org/t/p/w780/backdrop.jpg"
        );
        assert!(!record.credits_truncated);
    }

    #[test]
    fn mixed_source_pages_keep_upstream_pagination_without_false_category_totals() {
        let page = project_video_page(
            CatalogKind::Movie,
            2,
            Document {
                value: json!({"page": 2, "totalPages": 3, "totalResults": 60, "results": [
                    {"id": 1396, "mediaType": "tv", "name": "Breaking Bad"},
                    {"id": 6384, "mediaType": "person", "name": "Keanu Reeves"}
                ]}),
                sources: vec![source("seerr", SourceStatus::Complete, 42)],
                seerr: true,
            },
        )
        .unwrap();
        assert!(page.results.is_empty());
        assert_eq!(page.next_page, Some(3));
        assert_eq!(page.total_results, None);
        assert_eq!(page.sources[0].status, SourceStatus::Complete);
    }

    #[test]
    fn music_related_records_keep_release_and_group_identities_and_stale_status() {
        let artist = "4cbe2c2f-7f75-465a-8ad9-8aeb62b2a288";
        let group = "a1b2c3d4-1111-2222-3333-444455556666";
        let release = "b1b2c3d4-1111-2222-3333-444455556666";
        let record = music_record(CatalogKind::Artist, &json!({
            "id": artist, "name": "Fixture artist", "release-group-count": 2,
            "release-groups": [{"id": group, "title": "Fixture album", "first-release-date": "2001"}],
            "releases": [{"id": release, "title": "Fixture edition", "date": "2002"}]
        })).unwrap();
        assert_eq!(
            record.credits[0].id,
            CatalogId::MusicBrainzReleaseGroup(Uuid::parse_str(group).unwrap())
        );
        assert_eq!(
            record.credits[1].id,
            CatalogId::MusicBrainzRelease(Uuid::parse_str(release).unwrap())
        );
        assert!(record.credits_truncated);
        assert!(record.artwork.is_none());
        let page = project_music_page(
            CatalogKind::Artist,
            1,
            json!({
                "count": 1, "offset": 0, "artists": [{"id": artist, "name": "Fixture artist"}]
            }),
            vec![source("musicbrainz", SourceStatus::RateLimited, 42)],
        )
        .unwrap();
        assert_eq!(page.sources[0].status, SourceStatus::RateLimited);
        assert_eq!(page.results[0].sources[0].observed_at, 42);
        assert!(transient(&page.sources));
    }

    #[test]
    fn typed_identity_and_source_art_require_evidence() {
        assert!(identity(CatalogKind::Movie, "tv/1396").is_none());
        assert_eq!(
            identity(CatalogKind::Movie, "000603").unwrap().id("tmdb"),
            Some("movie/603")
        );
        assert!(identity(CatalogKind::Artist, "00000000-0000-0000-0000-000000000000").is_none());
        assert!(music_record(CatalogKind::MusicVideo, &json!({
            "id": "b1b2c3d4-1111-2222-3333-444455556666", "title": "Fixture audio", "video": false
        })).is_none());
        assert!(
            artwork(
                &json!("https://untrusted.example/image.jpg"),
                "movie",
                603,
                "w342"
            )
            .is_none()
        );
        let release = music_record(
            CatalogKind::Release,
            &json!({
                "id": "b1b2c3d4-1111-2222-3333-444455556666", "title": "Fixture edition",
                "cover-art-archive": {"front": true},
                "media": [{"track-count": 2, "tracks": [{"number": "1", "recording": {
                    "id": "c1b2c3d4-1111-2222-3333-444455556666", "title": "Fixture track"
                }}]}]
            }),
        )
        .unwrap();
        assert_eq!(
            release.credits[0].id,
            CatalogId::MusicBrainzRecording(
                Uuid::parse_str("c1b2c3d4-1111-2222-3333-444455556666").unwrap()
            )
        );
        assert_eq!(release.credits[0].role, "Track 1");
        assert!(release.credits_truncated);
        assert!(
            release
                .artwork
                .unwrap()
                .url
                .starts_with("https://coverartarchive.org/release/")
        );
    }
}
