//! Read-only public metadata fetched by this Bluefin instance.
use std::collections::HashSet;

use super::{Client, Duration, OnceLock, Read, Result, Url, Value, err, json, text};

const CATALOG_LIMIT: u64 = 1024 * 1024;

fn query(input: &Value) -> Result<&str> {
    let term = text(input, "query")?.trim();
    if term.chars().count() < 2 || term.chars().count() > 160 || term.chars().any(char::is_control)
    {
        return Err(err("Invalid public catalog query"));
    }
    Ok(term)
}

fn read_catalog(url: &str, params: &[(&str, &str)]) -> Result<Value> {
    static CLIENT: OnceLock<Result<Client>> = OnceLock::new();
    let client = CLIENT
        .get_or_init(|| {
            Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("Bluefin catalog (https://github.com/glopyglerky/bluefin)")
                .timeout(Duration::from_secs(6))
                .pool_max_idle_per_host(1)
                .build()
                .map_err(|_| err("Public catalog is unavailable"))
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let response = client
        .get(url)
        .query(params)
        .send()
        .map_err(|_| err("Public catalog is unavailable"))?;
    if !response.status().is_success() {
        return Err(err("Public catalog is unavailable"));
    }
    let mut bytes = Vec::new();
    response
        .take(CATALOG_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| err("Public catalog response could not be read"))?;
    if bytes.len() as u64 > CATALOG_LIMIT {
        return Err(err("Public catalog response is too large"));
    }
    serde_json::from_slice(&bytes).map_err(|_| err("Invalid public catalog response"))
}

/// Public lookup runs on this instance, without a peer origin or account token.
pub(super) fn public_tv_search(input: &Value) -> Result<Value> {
    let term = query(input)?;
    let value = read_catalog("https://api.tvmaze.com/search/shows", &[("q", term)])?;
    let rows = value
        .as_array()
        .ok_or_else(|| err("Invalid public TV catalog response"))?;
    let mut seen = HashSet::new();
    let results: Vec<_> = rows
        .iter()
        .filter_map(public_tv_title)
        .filter(|title| title["id"].as_u64().is_some_and(|id| seen.insert(id)))
        .take(20)
        .collect();
    Ok(json!({"results": results}))
}

fn wikidata(params: &[(&str, &str)]) -> Result<Value> {
    let mut query = vec![("format", "json"), ("formatversion", "2")];
    query.extend_from_slice(params);
    let value = read_catalog("https://www.wikidata.org/w/api.php", &query)?;
    if value.get("error").is_some() {
        return Err(err("Public film catalog is unavailable"));
    }
    Ok(value)
}

fn wikidata_id(value: &str) -> bool {
    value.len() <= 16
        && value.strip_prefix('Q').is_some_and(|digits| {
            !digits.is_empty()
                && !digits.starts_with('0')
                && digits.bytes().all(|digit| digit.is_ascii_digit())
        })
}

pub(super) fn public_film_search(input: &Value) -> Result<Value> {
    let term: String = query(input)?
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
        .collect();
    let term = term.trim();
    if term.is_empty() {
        return Ok(json!({"results": []}));
    }
    let search = format!("{term} haswbstatement:P31=Q11424");
    let response = wikidata(&[
        ("action", "query"),
        ("list", "search"),
        ("srnamespace", "0"),
        ("srlimit", "12"),
        ("srprop", ""),
        ("srsearch", &search),
    ])?;
    let rows = response["query"]["search"]
        .as_array()
        .ok_or_else(|| err("Invalid public film search response"))?;
    let mut seen = HashSet::new();
    let ids: Vec<_> = rows
        .iter()
        .filter_map(|row| row["title"].as_str())
        .filter(|id| wikidata_id(id) && seen.insert(*id))
        .take(12)
        .collect();
    if ids.is_empty() {
        return Ok(json!({"results": []}));
    }
    let language = input["language"].as_str().unwrap_or("en");
    let language = language.split('-').next().unwrap_or("en");
    let language = if (2..=3).contains(&language.len())
        && language.bytes().all(|ch| ch.is_ascii_lowercase())
    {
        language
    } else {
        "en"
    };
    let languages = if language == "en" {
        "en".into()
    } else {
        format!("{language}|en")
    };
    let detail = wikidata(&[
        ("action", "wbgetentities"),
        ("ids", &ids.join("|")),
        ("props", "labels|descriptions"),
        ("languages", &languages),
    ])?;
    let entities = detail["entities"]
        .as_object()
        .ok_or_else(|| err("Invalid public film catalog response"))?;
    let results: Vec<_> = ids
        .into_iter()
        .filter_map(|id| public_film_title(id, entities.get(id)?, language))
        .collect();
    Ok(json!({"results": results}))
}

fn public_film_title(id: &str, entity: &Value, language: &str) -> Option<Value> {
    if !wikidata_id(id) {
        return None;
    }
    let localized = |field: &str| {
        entity[field][language]["value"]
            .as_str()
            .or_else(|| entity[field]["en"]["value"].as_str())
    };
    let title = localized("labels")?.trim();
    if title.is_empty() || title.chars().count() > 256 || title.chars().any(char::is_control) {
        return None;
    }
    let description = localized("descriptions").map(|text| {
        text.chars()
            .filter(|ch| !ch.is_control())
            .take(180)
            .collect::<String>()
    });
    Some(json!({
        "id": id,
        "title": title,
        "description": description,
        "sourceUrl": format!("https://www.wikidata.org/wiki/{id}")
    }))
}

pub(super) fn public_title_sources(input: &Value) -> Result<Value> {
    let id = text(input, "id")?;
    let kind = text(input, "kind")?;
    let entity = match kind {
        "movie" if wikidata_id(id) => Some(id.to_owned()),
        "series" if decimal(id, 1, 12) => match find_wikidata_id("P8600", id)? {
            Some(entity) => Some(entity),
            None => match input["imdbId"].as_str().filter(|value| imdb_id(value)) {
                Some(imdb) => find_wikidata_id("P345", imdb)?,
                None => None,
            },
        },
        _ => return Err(err("Invalid public title identity")),
    };
    let Some(entity) = entity else {
        return Ok(json!({"links": [], "imdbIds": []}));
    };
    let response = wikidata(&[
        ("action", "wbgetentities"),
        ("ids", &entity),
        ("props", "claims"),
    ])?;
    let claims = &response["entities"][&entity]["claims"];
    if !claims.is_object() {
        return Err(err("Provider identities unavailable"));
    }
    Ok(json!({
        "links": direct_provider_links(claims, kind == "movie"),
        "imdbIds": claim_values(claims, "P345", imdb_id),
    }))
}

fn find_wikidata_id(property: &str, value: &str) -> Result<Option<String>> {
    let search = format!("haswbstatement:{property}={value}");
    let response = wikidata(&[
        ("action", "query"),
        ("list", "search"),
        ("srnamespace", "0"),
        ("srlimit", "3"),
        ("srprop", ""),
        ("srsearch", &search),
    ])?;
    let rows = response["query"]["search"]
        .as_array()
        .ok_or_else(|| err("Catalog identity unavailable"))?;
    let ids: HashSet<_> = rows
        .iter()
        .filter_map(|row| row["title"].as_str())
        .filter(|id| wikidata_id(id))
        .collect();
    Ok(if ids.len() == 1 {
        ids.into_iter().next().map(str::to_owned)
    } else {
        None
    })
}

fn claim_values<'a>(
    claims: &'a Value,
    property: &str,
    valid: impl Fn(&str) -> bool,
) -> Vec<&'a str> {
    let mut seen = HashSet::new();
    claims[property]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["rank"] != "deprecated" && row["mainsnak"]["snaktype"] == "value")
        .filter_map(|row| row["mainsnak"]["datavalue"]["value"].as_str())
        .filter(|value| valid(value) && seen.insert(*value))
        .take(6)
        .collect()
}

fn decimal(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.len()) && value.bytes().all(|ch| ch.is_ascii_digit())
}
fn imdb_id(value: &str) -> bool {
    value
        .strip_prefix("tt")
        .is_some_and(|id| decimal(id, 7, 12))
}
fn uuid_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn direct_provider_links(claims: &Value, movie: bool) -> Vec<Value> {
    type Rule<'a> = (&'a str, &'a str, fn(&str) -> bool, &'a str, &'a str, bool);
    let rules: [Rule<'_>; 9] = [
        (
            "Netflix",
            "P1874",
            |id| decimal(id, 6, 8),
            if movie {
                "https://www.netflix.com/watch/"
            } else {
                "https://www.netflix.com/title/"
            },
            "",
            movie,
        ),
        (
            "Disney+",
            "P13902",
            |id| {
                id.strip_prefix("entity-")
                    .or_else(|| id.strip_prefix("page-"))
                    .is_some_and(uuid_id)
            },
            "https://www.disneyplus.com/browse/",
            "",
            false,
        ),
        (
            "Disney+",
            if movie { "P7595" } else { "P7596" },
            |id| id.len() == 12 && id.bytes().all(|c| c.is_ascii_alphanumeric()),
            if movie {
                "https://www.disneyplus.com/movies/wd/"
            } else {
                "https://www.disneyplus.com/series/wp/"
            },
            "",
            false,
        ),
        (
            "Hulu",
            if movie { "P6466" } else { "P6467" },
            uuid_id,
            if movie {
                "https://www.hulu.com/movie/"
            } else {
                "https://www.hulu.com/series/"
            },
            "",
            false,
        ),
        (
            "Paramount+",
            "P13147",
            |id| {
                (8..=80).contains(&id.len())
                    && id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            },
            "https://www.paramountplus.com/shows/video/",
            "/",
            true,
        ),
        (
            "Peacock",
            "P11815",
            |id| {
                id.split_once('/').is_some_and(|(kind, slug)| {
                    matches!(kind, "movies" | "tv")
                        && slug.split('-').all(|s| {
                            !s.is_empty()
                                && s.bytes()
                                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                        })
                        && slug.len() <= 200
                })
            },
            "https://www.peacocktv.com/stream-",
            "",
            false,
        ),
        (
            "Prime Video",
            "P8055",
            |id| {
                id.len() == 10
                    && id.starts_with('B')
                    && id
                        .bytes()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            },
            "https://www.amazon.com/gp/video/detail/",
            "",
            false,
        ),
        (
            "HBO Max",
            "P8298",
            |id| {
                id.split_once('/')
                    .is_some_and(|(kind, id)| matches!(kind, "movie" | "show") && uuid_id(id))
            },
            "https://www.hbomax.com/",
            "",
            false,
        ),
        (
            "Tubi",
            if movie { "P7760" } else { "P7761" },
            |id| !id.starts_with('0') && decimal(id, 1, 12),
            if movie {
                "https://tubitv.com/movies/"
            } else {
                "https://tubitv.com/series/"
            },
            "",
            false,
        ),
    ];
    let mut seen = HashSet::new();
    rules
        .into_iter()
        .filter_map(|(provider, property, valid, prefix, suffix, player)| {
            let id =
                claim_values(claims, property, valid)
                    .into_iter()
                    .find(|id| match property {
                        "P11815" => id.starts_with(if movie { "movies/" } else { "tv/" }),
                        "P8298" => id.starts_with(if movie { "movie/" } else { "show/" }),
                        _ => true,
                    })?;
            seen.insert(provider).then(|| {
                json!({"provider": provider,
            "url": format!("{prefix}{id}{suffix}"), "kind": if player {"player"} else {"title"}})
            })
        })
        .collect()
}

fn public_tv_title(row: &Value) -> Option<Value> {
    let show = row.get("show")?;
    let id = show["id"].as_u64().filter(|id| *id > 0)?;
    let title = show["name"].as_str()?.trim();
    if title.is_empty() || title.chars().count() > 256 || title.chars().any(char::is_control) {
        return None;
    }
    let year = show["premiered"]
        .as_str()
        .and_then(|date| date.get(..4))
        .filter(|year| year.bytes().all(|byte| byte.is_ascii_digit()));
    let artwork = show["image"]["medium"]
        .as_str()
        .filter(|value| value.len() <= 1024)
        .and_then(|value| Url::parse(value).ok())
        .filter(|url| {
            url.scheme() == "https"
                && url.host_str() == Some("static.tvmaze.com")
                && url.port().is_none()
                && url.username().is_empty()
                && url.password().is_none()
                && url.path().starts_with("/uploads/images/medium_portrait/")
                && url.query().is_none()
                && url.fragment().is_none()
        })
        .map(|url| url.to_string());
    Some(json!({
        "id": id,
        "title": title,
        "year": year,
        "imdbId": show["externals"]["imdb"].as_str().filter(|id| imdb_id(id)),
        "artwork": artwork,
        "sourceUrl": format!("https://www.tvmaze.com/shows/{id}")
    }))
}

#[cfg(test)]
mod public_catalog_tests {
    use super::*;

    #[test]
    fn public_title_uses_the_provider_identity_and_rejects_unknown_artwork_origins() {
        let result = public_tv_title(&json!({"show": {"id": 47, "name": "A Stage",
            "url": "https://example.test/title", "premiered": "2025-04-01",
            "image": {"medium": "https://example.test/poster.jpg"}}}))
        .unwrap();
        assert_eq!(result["sourceUrl"], "https://www.tvmaze.com/shows/47");
        assert_eq!(result["year"], "2025");
        assert_eq!(result["artwork"], Value::Null);
        assert!(public_tv_title(&json!({"show": {"id": 0, "name": "Invalid"}})).is_none());
    }

    #[test]
    fn public_film_title_uses_a_typed_identity_and_localizes_metadata() {
        let entity = json!({
            "labels": {"en": {"value": "Example"}, "fr": {"value": "Exemple"}},
            "descriptions": {"en": {"value": "1982 film"}},
            "sourceUrl": "https://unrelated.test/title"
        });
        let title = public_film_title("Q184843", &entity, "fr").unwrap();
        assert_eq!(title["title"], "Exemple");
        assert_eq!(title["description"], "1982 film");
        assert_eq!(title["sourceUrl"], "https://www.wikidata.org/wiki/Q184843");
        assert!(public_film_title("Q184843/extra", &entity, "fr").is_none());
    }

    #[test]
    fn title_links_use_first_party_targets_for_exact_typed_ids() {
        let claims = json!({
            "P1874":[{"rank":"normal","mainsnak":{"snaktype":"value","datavalue":{"value":"80234304"}}}],
            "P6466":[{"rank":"normal","mainsnak":{"snaktype":"value","datavalue":{"value":"e47078f3-1c0e-49a8-9da9-c571a7a20fec"}}}],
            "P11815":[{"rank":"normal","mainsnak":{"snaktype":"value","datavalue":{"value":"movies/psych-3-this-is-gus"}}}],
            "P13147":[{"rank":"deprecated","mainsnak":{"snaktype":"value","datavalue":{"value":"WwH_xKtpmHlHq2_4qrlBjfFDKIsgeI0e"}}}],
            "P8298":[{"rank":"normal","mainsnak":{"snaktype":"value","datavalue":{"value":"movie/../../unrelated.test"}}}],
            "P9586":[{"rank":"normal","mainsnak":{"snaktype":"value","datavalue":{"value":"unused"}}}]
        });
        let movies = direct_provider_links(&claims, true);
        assert_eq!(movies.len(), 3);
        assert_eq!(movies[0]["url"], "https://www.netflix.com/watch/80234304");
        assert_eq!(movies[0]["kind"], "player");
        assert_eq!(
            movies[1]["url"],
            "https://www.hulu.com/movie/e47078f3-1c0e-49a8-9da9-c571a7a20fec"
        );
        assert_eq!(
            movies[2]["url"],
            "https://www.peacocktv.com/stream-movies/psych-3-this-is-gus"
        );
        let shows = direct_provider_links(&claims, false);
        assert_eq!(shows.len(), 1);
        assert_eq!(shows[0]["url"], "https://www.netflix.com/title/80234304");
        assert_eq!(shows[0]["kind"], "title");
    }
}
