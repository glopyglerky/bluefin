//! Read-only public metadata fetched by this Bluefin instance.
use std::collections::{HashMap, HashSet};

use super::{err, json, text, Client, Duration, OnceLock, Read, Result, Url, Value};

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
        ("props", "labels|descriptions|claims"),
        ("languages", &languages),
    ])?;
    let entities = detail["entities"]
        .as_object()
        .ok_or_else(|| err("Invalid public film catalog response"))?;
    let mut results: Vec<_> = ids
        .into_iter()
        .filter_map(|id| public_film_title(id, entities.get(id)?, language))
        .collect();
    add_commons_artwork(&mut results);
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
    let claims = &entity["claims"];
    let artwork_file = ["P3383", "P18"].into_iter().find_map(|property| {
        claim_values(claims, property, commons_filename)
            .into_iter()
            .next()
    });
    Some(json!({
        "id": id,
        "title": title,
        "description": description,
        "artworkFile": artwork_file,
        "links": direct_provider_links(claims, true),
        "imdbId": claim_values(claims, "P345", imdb_id).into_iter().next(),
        "sourceUrl": format!("https://www.wikidata.org/wiki/{id}")
    }))
}

fn commons_filename(name: &str) -> bool {
    name.len() <= 224
        && !name
            .chars()
            .any(|ch| ch.is_control() || "|:/\\#<>[]{}".contains(ch))
        && name.rsplit_once('.').is_some_and(|(_, extension)| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp"
            )
        })
}

fn add_commons_artwork(titles: &mut [Value]) {
    let files = titles
        .iter()
        .filter_map(|title| title["artworkFile"].as_str())
        .map(|name| format!("File:{name}"))
        .collect::<Vec<_>>()
        .join("|");
    if files.is_empty() {
        return;
    }
    let Ok(response) = read_catalog(
        "https://commons.wikimedia.org/w/api.php",
        &[
            ("action", "query"),
            ("format", "json"),
            ("formatversion", "2"),
            ("prop", "imageinfo"),
            ("iiprop", "url|extmetadata"),
            ("iiurlwidth", "240"),
            (
                "iiextmetadatafilter",
                "Artist|LicenseShortName|LicenseUrl|Attribution",
            ),
            ("titles", &files),
        ],
    ) else {
        return;
    };
    let key = |name: &str| name.replace('_', " ").to_lowercase();
    let pages = response["query"]["pages"].as_array().into_iter().flatten();
    let artwork: HashMap<_, _> = pages.filter_map(|page| {
        let name = key(page["title"].as_str()?.strip_prefix("File:")?);
        let info = page["imageinfo"].as_array()?.first()?;
        let mut image = Url::parse(info["thumburl"].as_str()?).ok()?;
        if image.scheme() != "https" || !matches!(image.host_str(), Some("upload.wikimedia.org" | "thumb.wikimedia.org"))
            || image.port().is_some() || !image.username().is_empty() || image.password().is_some()
            || !image.path().starts_with("/wikipedia/commons/") { return None; }
        image.set_query(None);
        image.set_fragment(None);
        let source = info["descriptionurl"].as_str()?.strip_prefix("https://commons.wikimedia.org/wiki/File:")?;
        let metadata = &info["extmetadata"];
        let author = metadata["Attribution"]["value"].as_str().or_else(|| metadata["Artist"]["value"].as_str());
        let license = metadata["LicenseShortName"]["value"].as_str()?;
        let license_url = metadata["LicenseUrl"]["value"].as_str().filter(|url| {
            url.strip_prefix("https://creativecommons.org/licenses/").is_some_and(|path|
                ["1.0/", "2.0/", "3.0/", "4.0/"].iter().any(|version|
                    path == format!("by/{version}") || path == format!("by-sa/{version}")))
        });
        let credit = author.map(strip_markup).filter(|credit| !credit.is_empty());
        if !matches!(license, "Public domain" | "CC0") && (credit.is_none() || license_url.is_none()) { return None; }
        Some((name, json!({"image": image.as_str(),
            "url": format!("https://commons.wikimedia.org/wiki/File:{source}"),
            "author": credit.unwrap_or_else(|| license.to_owned()), "license": license, "licenseUrl": license_url})))
    }).collect();
    for title in titles {
        let Some(asset) = title["artworkFile"]
            .as_str()
            .and_then(|name| artwork.get(&key(name)))
        else {
            continue;
        };
        title["artwork"] = asset["image"].clone();
        title["artworkCredit"] = asset.clone();
    }
}

fn strip_markup(value: &str) -> String {
    let mut tag = false;
    value
        .chars()
        .filter(|ch| {
            match ch {
                '<' => tag = true,
                '>' => {
                    tag = false;
                    return false;
                }
                _ => {}
            }
            !tag && !ch.is_control()
        })
        .take(1800)
        .collect::<String>()
        .trim()
        .to_owned()
}

pub(super) fn public_title_sources(input: &Value) -> Result<Value> {
    let id = text(input, "id")?;
    let kind = text(input, "kind")?;
    let mut links = Vec::new();
    let mut description = None;
    let mut credits = Vec::new();
    let entity = match kind {
        "movie" if wikidata_id(id) => Some(id.to_owned()),
        "series" if decimal(id, 1, 12) => {
            if let Some(tv) = read_catalog(
                &format!("https://api.tvmaze.com/shows/{id}"),
                &[("embed", "cast")],
            )
            .ok()
            .filter(|tv| {
                tv["id"]
                    .as_u64()
                    .is_some_and(|value| value.to_string() == id)
            }) {
                links.extend(tv["officialSite"].as_str().and_then(official_provider_site));
                description = tv["summary"].as_str().map(strip_markup);
                credits.extend(tv["_embedded"]["cast"].as_array().into_iter().flatten().take(20).filter_map(|credit| {
                    Some(json!({"name": credit["person"]["name"].as_str()?, "role": credit["character"]["name"].as_str().unwrap_or("Cast")}))
                }));
            }
            find_wikidata_id("P8600", id).ok().flatten().or_else(|| {
                let imdb = input["imdbId"].as_str().filter(|value| imdb_id(value))?;
                find_wikidata_id("P345", imdb).ok().flatten()
            })
        }
        _ => return Err(err("Invalid public title identity")),
    };
    let Some(entity) = entity else {
        return Ok(
            json!({"links": links, "imdbIds": [], "description": description, "credits": credits}),
        );
    };
    let response = match wikidata(&[
        ("action", "wbgetentities"),
        ("ids", &entity),
        ("props", "claims|descriptions"),
        ("languages", "en"),
    ]) {
        Ok(value) => value,
        Err(_) if kind == "series" => {
            return Ok(
                json!({"links": links, "imdbIds": [], "description": description, "credits": credits}),
            )
        }
        Err(error) => return Err(error),
    };
    let claims = &response["entities"][&entity]["claims"];
    if !claims.is_object() {
        return Err(err("Provider identities unavailable"));
    }
    links.extend(direct_provider_links(claims, kind == "movie"));
    let mut seen = HashSet::new();
    links.retain(|link| seen.insert(link["url"].as_str().unwrap_or_default().to_owned()));
    description = description.or_else(|| {
        response["entities"][&entity]["descriptions"]["en"]["value"]
            .as_str()
            .map(strip_markup)
    });
    Ok(
        json!({"links": links, "imdbIds": claim_values(claims, "P345", imdb_id), "description": description, "credits": credits}),
    )
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

fn official_provider_site(value: &str) -> Option<Value> {
    if value.len() > 512
        || !value.starts_with("https://")
        || value.contains("..")
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"./:_-".contains(&c))
    {
        return None;
    }
    let url = Url::parse(value).ok()?;
    if url.port().is_some() || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let path = url.path().trim_matches('/').split('/').collect::<Vec<_>>();
    let slug = |value: &str| {
        !value.is_empty()
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    };
    let (provider, player) = match (url.host_str()?, path.as_slice()) {
        ("www.netflix.com", [path @ ("title" | "watch"), id]) if decimal(id, 6, 8) => {
            ("Netflix", *path == "watch")
        }
        ("www.disneyplus.com", ["browse", entity])
            if entity
                .strip_prefix("entity-")
                .or_else(|| entity.strip_prefix("page-"))
                .is_some_and(uuid_id) =>
        {
            ("Disney+", false)
        }
        ("www.hulu.com", [path @ ("movie" | "series" | "watch"), entity])
            if entity
                .get(entity.len().saturating_sub(36)..)
                .is_some_and(uuid_id) =>
        {
            ("Hulu", *path == "watch")
        }
        ("www.paramountplus.com", ["shows" | "movies", name]) if slug(name) => {
            ("Paramount+", false)
        }
        ("www.peacocktv.com", ["stream-movies" | "stream-tv", name]) if slug(name) => {
            ("Peacock", false)
        }
        ("www.amazon.com", ["gp", "video", "detail", id])
            if id.starts_with('B') && id.len() == 10 && slug(id) =>
        {
            ("Prime Video", false)
        }
        ("www.hbomax.com", ["movie" | "show", id]) if uuid_id(id) => ("HBO Max", false),
        ("tubitv.com", ["movies" | "series", id, ..]) if decimal(id, 1, 12) && path.len() <= 3 => {
            ("Tubi", false)
        }
        ("www.espn.com", ["watch", "player", "_", "id", id]) if uuid_id(id) => ("ESPN", true),
        ("www.crunchyroll.com", [path @ ("series" | "watch"), id, ..])
            if id.len() == 9 && slug(id) && path.len() <= 3 =>
        {
            ("Crunchyroll", *path == "watch")
        }
        _ => return None,
    };
    Some(
        json!({"provider": provider, "url": url.as_str(), "kind": if player {"player"} else {"title"}}),
    )
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
    let mut links: Vec<_> = rules
        .into_iter()
        .flat_map(|(provider, property, valid, prefix, suffix, player)| {
            claim_values(claims, property, valid)
                .into_iter()
                .filter(move |id| match property {
                    "P11815" => id.starts_with(if movie { "movies/" } else { "tv/" }),
                    "P8298" => id.starts_with(if movie { "movie/" } else { "show/" }),
                    _ => true,
                })
                .map(move |id| {
                    json!({"provider": provider,
            "url": format!("{prefix}{id}{suffix}"), "kind": if player {"player"} else {"title"}})
                })
        })
        .collect();
    links.extend(
        claim_values(claims, "P856", |_| true)
            .into_iter()
            .filter_map(official_provider_site),
    );
    let mut seen = HashSet::new();
    links.retain(|link| seen.insert(link["url"].as_str().unwrap_or_default().to_owned()));
    links
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
        "description": show["summary"].as_str().map(strip_markup),
        "links": show["officialSite"].as_str().and_then(official_provider_site).into_iter().collect::<Vec<_>>(),
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
    fn tvmaze_supplies_the_exact_official_provider_with_the_result() {
        let result = public_tv_title(&json!({"show": {"id": 47, "name": "Series", "summary": "<p>The premise.</p>",
            "officialSite": "https://www.netflix.com/title/80057281",
            "image": {"medium": "https://static.tvmaze.com/uploads/images/medium_portrait/2/47.png"}}})).unwrap();
        assert_eq!(result["description"], "The premise.");
        assert_eq!(
            result["links"][0]["url"],
            "https://www.netflix.com/title/80057281"
        );
        assert!(result["artwork"].as_str().unwrap().ends_with("/47.png"));
        assert!(
            official_provider_site("https://www.netflix.com/title/80057281/redirect").is_none()
        );
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
