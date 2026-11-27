use std::sync::Arc;

use async_trait::async_trait;
use bluefin_contract::public_catalog::{
    CatalogArtwork, CatalogCredit, CatalogId, CatalogKind, CatalogPage,
    CatalogRecord,
};
use ferrofin_api::public_catalog::PublicCatalog;
use ferrofin_providers::{
    MusicBrainzClient, TmdbClient, musicbrainz::CatalogMusicEntity, tmdb::CatalogVideoEntity,
};
use serde_json::Value;
use tokio::sync::Semaphore;
use uuid::Uuid;

pub(crate) struct SpecialistCatalog {
    musicbrainz: Arc<MusicBrainzClient>,
    tmdb: Option<TmdbClient>,
    requests: Semaphore,
}

impl SpecialistCatalog {
    pub(crate) fn new(musicbrainz: Arc<MusicBrainzClient>) -> Self {
        let tmdb = std::env::var("BLUEFIN_TMDB_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty())
            .map(TmdbClient::with_api_key);
        Self {
            musicbrainz,
            tmdb,
            requests: Semaphore::new(8),
        }
    }
}

#[async_trait]
impl PublicCatalog for SpecialistCatalog {
    async fn search(&self, kind: CatalogKind, query: &str, page: u32) -> Option<CatalogPage> {
        let _slot = self.requests.try_acquire().ok()?;
        if let Some(entity) = video_entity(kind) {
            let value = self
                .tmdb
                .as_ref()?
                .catalog_search(entity, query, page)
                .await?;
            let results = value["results"]
                .as_array()?
                .iter()
                .take(20)
                .filter_map(|row| video_record(kind, row))
                .collect();
            let last = value["total_pages"].as_u64()?;
            return Some(CatalogPage {
                results,
                next_page: (u64::from(page) < last && page < 100).then_some(page + 1),
            });
        }
        let (entity, field) = music_entity(kind)?;
        let offset = (page - 1) * 20;
        let value = self
            .musicbrainz
            .catalog_search(entity, query, offset)
            .await?;
        let results = value[field]
            .as_array()?
            .iter()
            .take(20)
            .filter_map(|row| music_record(kind, row))
            .collect();
        let last = value["count"].as_u64()?;
        Some(CatalogPage {
            results,
            next_page: (u64::from(offset + 20) < last && page < 100).then_some(page + 1),
        })
    }

    async fn details(&self, kind: CatalogKind, id: &str) -> Option<CatalogRecord> {
        let _slot = self.requests.try_acquire().ok()?;
        if let Some(entity) = video_entity(kind) {
            let id = id.parse::<u64>().ok().filter(|id| *id != 0)?;
            let value = self.tmdb.as_ref()?.catalog_details(entity, id).await?;
            return (value["id"].as_u64() == Some(id))
                .then(|| video_record(kind, &value))
                .flatten();
        }
        let (entity, _) = music_entity(kind)?;
        let id = Uuid::parse_str(id).ok().filter(|id| !id.is_nil())?;
        let value = self.musicbrainz.catalog_details(entity, id).await?;
        (music_id(&value) == Some(id))
            .then(|| music_record(kind, &value))
            .flatten()
    }

}

fn text(value: &Value, limit: usize) -> Option<String> {
    let value = value.as_str()?.trim();
    (!value.is_empty()).then(|| {
        value
            .chars()
            .filter(|ch| !ch.is_control())
            .take(limit)
            .collect()
    })
}

fn year(value: &Value) -> Option<i32> {
    value.as_str()?.get(..4)?.parse().ok()
}

fn video_entity(kind: CatalogKind) -> Option<CatalogVideoEntity> {
    match kind {
        CatalogKind::Movie => Some(CatalogVideoEntity::Movie),
        CatalogKind::Series => Some(CatalogVideoEntity::Series),
        CatalogKind::Person => Some(CatalogVideoEntity::Person),
        _ => None,
    }
}

fn video_record(kind: CatalogKind, row: &Value) -> Option<CatalogRecord> {
    let id = row["id"].as_u64().filter(|id| *id != 0)?;
    let (id_type, resource, name, date, description, image) = match kind {
        CatalogKind::Movie => (
            CatalogId::TmdbMovie(id),
            "movie",
            "title",
            "release_date",
            "overview",
            "poster_path",
        ),
        CatalogKind::Series => (
            CatalogId::TmdbSeries(id),
            "tv",
            "name",
            "first_air_date",
            "overview",
            "poster_path",
        ),
        CatalogKind::Person => (
            CatalogId::TmdbPerson(id),
            "person",
            "name",
            "birthday",
            "biography",
            "profile_path",
        ),
        _ => return None,
    };
    let artwork = row[image].as_str().and_then(|path| {
        let file = path.strip_prefix('/')?;
        if file.len() > 128
            || file.matches('.').count() != 1
            || !file
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-.".contains(&byte))
        {
            return None;
        }
        Some(CatalogArtwork {
            url: format!("https://image.tmdb.org/t/p/w342/{file}"),
            credit: "TMDB".to_owned(),
            source_url: format!("https://www.themoviedb.org/{resource}/{id}"),
        })
    });
    let credits_key = if kind == CatalogKind::Person {
        "combined_credits"
    } else {
        "credits"
    };
    let credits = ["cast", "crew"]
        .into_iter()
        .flat_map(|role| {
            row[credits_key][role]
                .as_array()
                .into_iter()
                .flatten()
                .take(18)
        })
        .filter_map(|credit| {
            let id = credit["id"].as_u64().filter(|id| *id != 0)?;
            let related = if kind == CatalogKind::Person {
                match credit["media_type"].as_str()? {
                    "movie" => CatalogId::TmdbMovie(id),
                    "tv" => CatalogId::TmdbSeries(id),
                    _ => return None,
                }
            } else {
                CatalogId::TmdbPerson(id)
            };
            let name = text(&credit["name"], 256).or_else(|| text(&credit["title"], 256))?;
            let role = text(&credit["character"], 120)
                .or_else(|| text(&credit["job"], 120))
                .unwrap_or_else(|| "Cast".to_owned());
            Some(CatalogCredit {
                id: related,
                name,
                role,
            })
        })
        .collect();
    let imdb_id = row["external_ids"]["imdb_id"]
        .as_str()
        .or_else(|| row["imdb_id"].as_str())
        .filter(|id| {
            id.strip_prefix("tt").is_some_and(|digits| {
                (7..=12).contains(&digits.len()) && digits.bytes().all(|byte| byte.is_ascii_digit())
            })
        })
        .map(str::to_owned);
    Some(CatalogRecord {
        id: id_type,
        title: text(&row[name], 256)?,
        year: year(&row[date]),
        summary: text(&row[description], 4000),
        artwork,
        imdb_id,
        credits,
    })
}

fn music_entity(kind: CatalogKind) -> Option<(CatalogMusicEntity, &'static str)> {
    match kind {
        CatalogKind::Artist => Some((CatalogMusicEntity::Artist, "artists")),
        CatalogKind::Album => Some((CatalogMusicEntity::ReleaseGroup, "release-groups")),
        CatalogKind::Recording => Some((CatalogMusicEntity::Recording, "recordings")),
        _ => None,
    }
}

fn music_id(row: &Value) -> Option<Uuid> {
    Uuid::parse_str(row["id"].as_str()?)
        .ok()
        .filter(|id| !id.is_nil())
}

fn music_record(kind: CatalogKind, row: &Value) -> Option<CatalogRecord> {
    let id = music_id(row)?;
    let id_type = match kind {
        CatalogKind::Artist => CatalogId::MusicBrainzArtist(id),
        CatalogKind::Album => CatalogId::MusicBrainzReleaseGroup(id),
        CatalogKind::Recording => CatalogId::MusicBrainzRecording(id),
        _ => return None,
    };
    let artists = row["artist-credit"]
        .as_array()
        .into_iter()
        .flatten()
        .take(12)
        .filter_map(|credit| {
            let artist = &credit["artist"];
            Some(CatalogCredit {
                id: CatalogId::MusicBrainzArtist(music_id(artist)?),
                name: text(&credit["name"], 256).or_else(|| text(&artist["name"], 256))?,
                role: "Performer".to_owned(),
            })
        });
    let releases = row["release-groups"]
        .as_array()
        .into_iter()
        .flatten()
        .take(24)
        .filter_map(|credit| {
            Some(CatalogCredit {
                id: CatalogId::MusicBrainzReleaseGroup(music_id(credit)?),
                name: text(&credit["title"], 256)?,
                role: "Release".to_owned(),
            })
        });
    let artwork = (kind == CatalogKind::Album).then(|| CatalogArtwork {
        url: format!("https://coverartarchive.org/release-group/{id}/front-250"),
        credit: "Cover Art Archive".to_owned(),
        source_url: format!("https://musicbrainz.org/release-group/{id}/cover-art"),
    });
    Some(CatalogRecord {
        id: id_type,
        title: text(&row["name"], 256).or_else(|| text(&row["title"], 256))?,
        year: year(&row["first-release-date"]).or_else(|| year(&row["life-span"]["begin"])),
        summary: text(&row["disambiguation"], 4000),
        artwork,
        imdb_id: None,
        credits: artists.chain(releases).collect(),
    })
}
