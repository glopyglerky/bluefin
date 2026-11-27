//! MusicBrainz URL relationships and exact iTunes music listings.
//!
//! Relationship meanings are defined by MusicBrainz, independently of provider names:
//! <https://musicbrainz.org/relationships/artist-url>,
//! <https://musicbrainz.org/relationships/release-url>, and
//! <https://musicbrainz.org/relationships/recording-url>.
//! A relationship identifies a destination. It does not establish a country or user entitlement.

use alloc::{collections::BTreeSet, format, string::{String, ToString}, vec, vec::Vec};
use serde_json::Value;
use url::Url;

use crate::{
    AccessOffer, AccessType, Attribution, CatalogId, CatalogIdentity, Coverage, CoverageScope,
    Destination, DestinationKind, LookupRequest, MediaKind, OfferEvidence, OfferResponse, Price,
    ServiceIdentity, SourceStatus, offer_id, safe_https_url,
};

/// The supported MusicBrainz entity path and its unique, canonical identifier.
pub fn musicbrainz_entity(identity: &CatalogIdentity) -> Option<(&'static str, String)> {
    let entity = match identity.kind {
        MediaKind::Artist => "artist",
        MediaKind::Release => "release",
        MediaKind::ReleaseGroup => "release-group",
        MediaKind::Recording | MediaKind::MusicVideo => "recording",
        MediaKind::Work => "work",
        _ => return None,
    };
    let mut id: Option<String> = None;
    for candidate in identity.ids.iter().filter(|id| id.source.eq_ignore_ascii_case("musicbrainz")) {
        if !valid_mbid(&candidate.id) {
            return None;
        }
        let value = candidate.id.to_ascii_lowercase();
        if id.as_ref().is_some_and(|previous| previous != &value) {
            return None;
        }
        id = Some(value);
    }
    Some((entity, id?))
}

fn valid_mbid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) { byte == b'-' } else { byte.is_ascii_hexdigit() }
        })
        && value.bytes().any(|byte| byte != b'0' && byte != b'-')
}

pub(crate) fn musicbrainz_source(
    request: &LookupRequest,
    status: SourceStatus,
    observed_at: u64,
) -> OfferResponse {
    let mut result = OfferResponse::with_status(
        request, "musicbrainz", status, CoverageScope::LinkedAccess, observed_at,
    );
    let source = &mut result.sources[0];
    source.coverage = Coverage::Partial;
    source.catalog_url = musicbrainz_entity(&request.identity)
        .map(|(entity, id)| format!("https://musicbrainz.org/{entity}/{id}"));
    source.attributions.push(Attribution {
        name: "MusicBrainz".into(),
        url: "https://musicbrainz.org/".into(),
    });
    result
}

/// Projects only forward, non-ended access relationships for the requested entity.
/// Unknown provider domains are retained; region and personal entitlement are never inferred.
pub fn project_musicbrainz_relations(
    request: &LookupRequest,
    body: &Value,
    observed_at: u64,
) -> OfferResponse {
    let Some((_, expected_id)) = musicbrainz_entity(&request.identity) else {
        return musicbrainz_source(request, SourceStatus::UnsupportedIdentity, observed_at);
    };
    if !body["id"].as_str().is_some_and(|actual| actual.eq_ignore_ascii_case(&expected_id)) {
        return musicbrainz_source(request, SourceStatus::IdentityMismatch, observed_at);
    }
    if request.identity.kind == MediaKind::MusicVideo && body["video"].as_bool() != Some(true) {
        return musicbrainz_source(request, SourceStatus::IdentityMismatch, observed_at);
    }
    let Some(relations) = body["relations"].as_array() else {
        return musicbrainz_source(request, SourceStatus::InvalidResponse, observed_at);
    };
    let mut result = musicbrainz_source(request, SourceStatus::Complete, observed_at);
    let mut seen = BTreeSet::new();
    let mut malformed_access = false;
    for relation in relations {
        if !active_url_relation(relation) {
            continue;
        }
        let Some(raw_type) = relation["type"].as_str() else { continue };
        let Some(access_type) = relationship_access(request.identity.kind, raw_type, relation) else {
            continue;
        };
        let Some(url) = relation["url"]["resource"].as_str().and_then(safe_https_url) else {
            malformed_access = true;
            continue;
        };
        let Some(service) = service_from_url(&url) else {
            malformed_access = true;
            continue;
        };
        let mut offer = AccessOffer {
            id: String::new(),
            identity: request.identity.clone(),
            country: None,
            service,
            access_type,
            raw_access_type: Some(raw_type.to_string()),
            addon: None,
            // A download relationship points to a page, not an authorized file download.
            destinations: vec![Destination { kind: DestinationKind::Title, url }],
            price: None,
            quality: None,
            evidence: OfferEvidence {
                source: "musicbrainz".into(),
                observed_at,
                available_since: None,
                expires_at: None,
                expires_soon: false,
                region_verified: false,
            },
        };
        offer.id = offer_id(&offer);
        if seen.insert(offer.id.clone()) {
            result.offers.push(offer);
        }
    }
    result.offers.sort_by(|a, b| (&a.service.name, a.access_type, &a.id)
        .cmp(&(&b.service.name, b.access_type, &b.id)));
    if malformed_access {
        result.sources[0].status = SourceStatus::Partial;
    } else if result.offers.is_empty() {
        result.sources[0].status = SourceStatus::NoOffers;
    }
    result
}


/// Projects an identity-bound page of real release editions for an album release group.
///
/// MusicBrainz browse must include `url-rels+release-groups`. Every edition must explicitly
/// name the requested release group. Offers retain their exact release identity rather than
/// claiming that a particular edition is the entire album concept. Incomplete pagination is
/// reported as partial coverage. See <https://musicbrainz.org/doc/MusicBrainz_API#Browse>.
pub fn project_musicbrainz_release_group_editions(
    request: &LookupRequest,
    body: &Value,
    observed_at: u64,
) -> OfferResponse {
    let Some((entity, group_id)) = musicbrainz_entity(&request.identity) else {
        return musicbrainz_source(request, SourceStatus::UnsupportedIdentity, observed_at);
    };
    if entity != "release-group" {
        return musicbrainz_source(request, SourceStatus::UnsupportedIdentity, observed_at);
    }
    let (Some(total), Some(offset), Some(editions)) = (
        body["release-count"].as_u64(), body["release-offset"].as_u64(),
        body["releases"].as_array(),
    ) else {
        return musicbrainz_source(request, SourceStatus::InvalidResponse, observed_at);
    };
    if offset > total || editions.len() as u64 > total - offset ||
        (editions.is_empty() && offset < total) {
        return musicbrainz_source(request, SourceStatus::InvalidResponse, observed_at);
    }
    let mut response = musicbrainz_source(request, SourceStatus::Complete, observed_at);
    let mut partial = offset != 0 || editions.len() as u64 != total;
    for edition in editions {
        if !edition["release-group"]["id"].as_str()
            .is_some_and(|id| id.eq_ignore_ascii_case(&group_id)) {
            partial = true;
            continue;
        }
        let Some(id) = edition["id"].as_str().filter(|id| valid_mbid(id)) else {
            partial = true;
            continue;
        };
        let edition_request = LookupRequest {
            identity: CatalogIdentity {
                kind: MediaKind::Release,
                ids: vec![CatalogId { source: "musicbrainz".into(), id: id.to_ascii_lowercase() }],
            },
            country: request.country.clone(),
        };
        let projected = project_musicbrainz_relations(&edition_request, edition, observed_at);
        if !matches!(projected.sources[0].status, SourceStatus::Complete | SourceStatus::NoOffers) {
            partial = true;
        }
        response.offers.extend(projected.offers);
    }
    response.offers.sort_by(|a,b| a.id.cmp(&b.id));
    response.offers.dedup_by(|a,b| a.id == b.id);
    response.sources[0].status = if partial { SourceStatus::Partial }
        else if response.offers.is_empty() { SourceStatus::NoOffers }
        else { SourceStatus::Complete };
    response
}

/// Exact Apple edition identifiers from a validated release-group browse response.
pub fn itunes_release_group_ids(identity: &CatalogIdentity, body: &Value) -> Vec<String> {
    let Some(("release-group", group_id)) = musicbrainz_entity(identity) else {
        return Vec::new();
    };
    let Some(editions) = body["releases"].as_array() else { return Vec::new() };
    let mut ids = BTreeSet::new();
    for edition in editions {
        if !edition["release-group"]["id"].as_str()
            .is_some_and(|id| id.eq_ignore_ascii_case(&group_id)) {
            continue;
        }
        let Some(id) = edition["id"].as_str().filter(|id| valid_mbid(id)) else { continue };
        let edition_identity = CatalogIdentity {
            kind: MediaKind::Release,
            ids: vec![CatalogId { source: "musicbrainz".into(), id: id.to_ascii_lowercase() }],
        };
        ids.extend(itunes_music_ids(&edition_identity, Some(edition)));
    }
    ids.into_iter().take(20).collect()
}

fn active_url_relation(relation: &Value) -> bool {
    relation["ended"].as_bool() != Some(true)
        && relation["direction"].as_str() == Some("forward")
        && relation.get("target-type").and_then(Value::as_str).is_none_or(|kind| kind == "url")
}

fn relationship_access(kind: MediaKind, name: &str, relation: &Value) -> Option<AccessType> {
    Some(match name {
        "free streaming" => AccessType::Free,
        "purchase for download" | "purchase for mail-order" => AccessType::Buy,
        "download for free" => AccessType::Download,
        "streaming" => {
            // These exact type IDs explicitly mean subscription streaming in upstream docs.
            // A new or inconsistent type ID remains an unclassified linked access option.
            let expected = match kind {
                MediaKind::Artist => "63cc5d1f-f096-4c94-a43f-ecb32ea94161",
                MediaKind::Release => "320adf26-96fa-4183-9045-1f5f32f833cb",
                MediaKind::Recording | MediaKind::MusicVideo =>
                    "b5f3058a-666c-406f-aafb-f9249fc7b122",
                _ => "",
            };
            if !expected.is_empty() && relation["type-id"].as_str() == Some(expected) {
                AccessType::Subscription
            } else {
                AccessType::Other
            }
        }
        // These are provider pages. They do not themselves describe a purchase or subscription.
        "bandcamp" | "apple music" | "youtube" => AccessType::Other,
        _ => return None,
    })
}

fn service_from_url(value: &str) -> Option<ServiceIdentity> {
    let url = Url::parse(value).ok()?;
    let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    let display_host = host.strip_prefix("www.").unwrap_or(&host);
    let name = match display_host {
        "open.spotify.com" | "play.spotify.com" | "spotify.com" => "Spotify",
        "deezer.com" => "Deezer",
        "tidal.com" | "listen.tidal.com" => "Tidal",
        "qobuz.com" | "open.qobuz.com" => "Qobuz",
        "music.apple.com" => "Apple Music",
        "itunes.apple.com" => "iTunes Store",
        "music.youtube.com" => "YouTube Music",
        "youtube.com" | "youtu.be" => "YouTube",
        "soundcloud.com" => "SoundCloud",
        "bandcamp.com" => "Bandcamp",
        "music.amazon.com" | "music.amazon.co.uk" | "music.amazon.de" |
        "music.amazon.fr" | "music.amazon.it" | "music.amazon.es" |
        "music.amazon.co.jp" | "music.amazon.com.au" | "music.amazon.ca" |
        "music.amazon.in" | "music.amazon.com.br" | "music.amazon.com.mx" => "Amazon Music",
        "beatport.com" => "Beatport",
        _ if display_host.ends_with(".bandcamp.com") => "Bandcamp",
        _ if display_host == "7digital.com" || display_host.ends_with(".7digital.com") => "7digital",
        _ => display_host,
    };
    Some(ServiceIdentity {
        source: "domain".into(),
        id: host.clone(),
        name: name.to_string(),
        home_url: Some(format!("https://{host}/")),
        logo_url: None,
    })
}

/// Exact Apple catalog identifiers already supplied by an identity or MusicBrainz relationship.
/// No title search or fabricated provider URL participates in this cross-reference.
pub fn itunes_music_ids(identity: &CatalogIdentity, body: Option<&Value>) -> Vec<String> {
    if !matches!(identity.kind, MediaKind::Artist | MediaKind::Release | MediaKind::ReleaseGroup |
        MediaKind::Recording | MediaKind::MusicVideo) {
        return Vec::new();
    }
    let mut ids = BTreeSet::new();
    for id in identity.ids.iter().filter(|id| id.source.eq_ignore_ascii_case("itunes")) {
        if numeric_id(&id.id) {
            ids.insert(id.id.clone());
        }
    }
    if let (Some(body), Some((_, expected))) = (body, musicbrainz_entity(identity)) {
        if body["id"].as_str().is_some_and(|actual| actual.eq_ignore_ascii_case(&expected)) {
            if let Some(relations) = body["relations"].as_array() {
                for relation in relations {
                    if !active_url_relation(relation) ||
                        relation["type"].as_str().and_then(|name| {
                            relationship_access(identity.kind, name, relation)
                        }).is_none() {
                        continue;
                    }
                    if let Some(value) = relation["url"]["resource"].as_str() {
                        if let Some((_, id)) = apple_url_identity(value, identity.kind) {
                            ids.insert(id);
                        }
                    }
                }
            }
        }
    }
    ids.into_iter().take(20).collect()
}

fn numeric_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 19 && !value.starts_with('0')
        && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn apple_url_identity(value: &str, kind: MediaKind) -> Option<(String, String)> {
    let normalized = safe_https_url(value)?;
    let url = Url::parse(&normalized).ok()?;
    if !matches!(url.host_str()?, "music.apple.com" | "itunes.apple.com") {
        return None;
    }
    let segments: Vec<_> = url.path_segments()?.filter(|part| !part.is_empty()).collect();
    let country = *segments.first()?;
    if country.len() != 2 || !country.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return None;
    }
    let entity = *segments.get(1)?;
    let id = match kind {
        MediaKind::Artist if entity == "artist" => segments.last()?.to_string(),
        MediaKind::Release | MediaKind::ReleaseGroup if entity == "album" =>
            segments.last()?.to_string(),
        MediaKind::Recording if entity == "song" => segments.last()?.to_string(),
        MediaKind::Recording if entity == "album" =>
            url.query_pairs().find(|(key, _)| key == "i")?.1.into_owned(),
        MediaKind::MusicVideo if entity == "music-video" => segments.last()?.to_string(),
        _ => return None,
    };
    let id = id.strip_prefix("id").unwrap_or(&id);
    if !numeric_id(id) {
        return None;
    }
    Some((country.to_ascii_uppercase(), id.to_string()))
}

pub(crate) fn itunes_source(
    request: &LookupRequest, status: SourceStatus, observed_at: u64,
) -> OfferResponse {
    let mut result = OfferResponse::with_status(
        request, "itunes", status, CoverageScope::RegionalAvailability, observed_at,
    );
    result.sources[0].coverage = Coverage::Partial;
    result.sources[0].attributions.push(Attribution {
        name: "iTunes Search API".into(),
        url: "https://developer.apple.com/library/archive/documentation/AudioVideo/Conceptual/iTuneSearchAPI/".into(),
    });
    result
}

/// Projects an exact-ID response from the explicitly requested iTunes storefront.
/// A returned regional listing is not evidence of the user's subscription or playback rights.
pub fn project_itunes_music_lookup(
    request: &LookupRequest,
    expected_ids: &[String],
    body: &Value,
    observed_at: u64,
) -> OfferResponse {
    let Some(country) = request.country.as_ref() else {
        return itunes_source(request, SourceStatus::UnsupportedCountry, observed_at);
    };
    if expected_ids.is_empty() || expected_ids.iter().any(|id| !numeric_id(id)) {
        return itunes_source(request, SourceStatus::UnsupportedIdentity, observed_at);
    }
    let Some(rows) = body["results"].as_array() else {
        return itunes_source(request, SourceStatus::InvalidResponse, observed_at);
    };
    if body["resultCount"].as_u64() != Some(rows.len() as u64) {
        return itunes_source(request, SourceStatus::InvalidResponse, observed_at);
    }
    let mut result = itunes_source(request, SourceStatus::Complete, observed_at);
    let mut checked = 0usize;
    let mut mismatched_storefront = false;
    for row in rows {
        let Some((id, link, price_field)) = itunes_row(row, request.identity.kind) else { continue };
        if !expected_ids.iter().any(|expected| expected == &id) {
            continue;
        }
        checked += 1;
        let Some((returned_country, returned_id)) = apple_url_identity(link, request.identity.kind) else {
            mismatched_storefront = true;
            continue;
        };
        if returned_country != country.as_str() || returned_id != id {
            mismatched_storefront = true;
            continue;
        }
        let Some(destination) = safe_https_url(link) else { continue };
        let currency = row["currency"].as_str().filter(|value| {
            value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_uppercase())
        });
        let amount = row[price_field].as_number().filter(|number| {
            number.as_f64().is_some_and(|amount| amount.is_finite() && amount >= 0.0)
        }).map(ToString::to_string);
        let purchase = amount.is_some() && currency.is_some() &&
            request.identity.kind != MediaKind::Artist;
        let price = if purchase {
            Some(Price {
                amount,
                currency: currency.map(ToString::to_string),
                formatted: None,
            })
        } else { None };
        let mut identity = request.identity.clone();
        if !identity.ids.iter().any(|entry| entry.source.eq_ignore_ascii_case("itunes") && entry.id == id) {
            identity.ids.push(CatalogId { source: "itunes".into(), id });
        }
        let mut offer = AccessOffer {
            id: String::new(),
            identity,
            country: Some(country.clone()),
            service: ServiceIdentity {
                source: "itunes".into(), id: "itunes-store".into(), name: "iTunes Store".into(),
                home_url: Some("https://itunes.apple.com/".into()), logo_url: None,
            },
            access_type: if purchase { AccessType::Buy } else { AccessType::Other },
            raw_access_type: Some(if purchase { "purchase listing" } else { "catalog listing" }.into()),
            addon: None,
            destinations: vec![Destination { kind: DestinationKind::Title, url: destination }],
            price,
            quality: None,
            evidence: OfferEvidence {
                source: "itunes".into(), observed_at, available_since: None, expires_at: None,
                expires_soon: false, region_verified: true,
            },
        };
        offer.id = offer_id(&offer);
        result.offers.push(offer);
    }
    result.offers.sort_by(|a,b| a.id.cmp(&b.id));
    result.offers.dedup_by(|a,b| a.id == b.id);
    if mismatched_storefront {
        result.sources[0].status = if result.offers.is_empty() {
            SourceStatus::UnsupportedCountry
        } else { SourceStatus::Partial };
    } else if result.offers.is_empty() {
        result.sources[0].status = if rows.is_empty() {
            SourceStatus::NoOffers
        } else if checked == 0 {
            SourceStatus::IdentityMismatch
        } else { SourceStatus::InvalidResponse };
    }
    result
}

fn itunes_row(row: &Value, kind: MediaKind) -> Option<(String, &str, &'static str)> {
    let (id_field, link_field, price_field) = match kind {
        MediaKind::Artist if row["wrapperType"] == "artist" =>
            ("artistId", "artistLinkUrl", ""),
        MediaKind::Release | MediaKind::ReleaseGroup if row["wrapperType"] == "collection" &&
            row["collectionType"] == "Album" =>
            ("collectionId", "collectionViewUrl", "collectionPrice"),
        MediaKind::Recording if row["wrapperType"] == "track" && row["kind"] == "song" =>
            ("trackId", "trackViewUrl", "trackPrice"),
        MediaKind::MusicVideo if row["wrapperType"] == "track" && row["kind"] == "music-video" =>
            ("trackId", "trackViewUrl", "trackPrice"),
        _ => return None,
    };
    let id = row[id_field].as_u64()?.to_string();
    if !numeric_id(&id) { return None; }
    Some((id, row[link_field].as_str()?, price_field))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CountryCode;
    use serde_json::json;

    const MBID: &str = "a74b1b7f-71a5-4011-9441-d0b5e4122711";

    fn request(kind: MediaKind, country: Option<&str>) -> LookupRequest {
        LookupRequest {
            identity: CatalogIdentity {
                kind,
                ids: vec![CatalogId { source: "musicbrainz".into(), id: MBID.into() }],
            },
            country: country.map(|code| CountryCode::parse(code).expect("test country")),
        }
    }
    fn relation(kind: &str, url: &str) -> Value {
        json!({"type":kind, "direction":"forward", "ended":false, "url":{"resource":url}})
    }


    #[test]
    fn album_group_links_retain_actual_release_identity_and_reject_unrelated_editions() {
        let edition_id = "49754d9a-6fe2-4b0e-9243-909f0795d189";
        let linked = json!({
            "id":edition_id,"release-group":{"id":MBID},
            "relations":[relation("free streaming","https://open.spotify.com/album/exact-album")]
        });
        let unrelated = json!({
            "id":"6f1d72ec-6410-422e-bc3f-f665df1f8965",
            "release-group":{"id":"58e42935-eede-4e6c-804c-d135625c278e"},
            "relations":[relation("free streaming","https://other.example.org/wrong-album")]
        });
        let req = request(MediaKind::ReleaseGroup, None);
        let body = json!({"release-count":2,"release-offset":0,"releases":[linked,unrelated]});
        let output = project_musicbrainz_release_group_editions(&req,&body,42);
        assert_eq!(output.identity, req.identity);
        assert_eq!(output.offers.len(),1);
        assert_eq!(output.offers[0].identity.kind,MediaKind::Release);
        assert_eq!(output.offers[0].identity.id("musicbrainz"),Some(edition_id));
        assert_eq!(output.sources[0].status,SourceStatus::Partial);
        assert!(!output.offers[0].evidence.region_verified);
    }

    #[test]
    fn release_group_browse_validates_pagination_and_exact_apple_cross_reference() {
        let edition = json!({
            "id":"49754d9a-6fe2-4b0e-9243-909f0795d189","release-group":{"id":MBID},
            "relations":[relation("purchase for download","https://music.apple.com/gb/album/example/12345")]
        });
        let req = request(MediaKind::ReleaseGroup,Some("GB"));
        let mut body = json!({"release-count":3,"release-offset":0,"releases":[edition]});
        assert_eq!(project_musicbrainz_release_group_editions(&req,&body,42).sources[0].status,
            SourceStatus::Partial);
        assert_eq!(itunes_release_group_ids(&req.identity,&body),vec!["12345"]);
        body["release-count"] = json!(0);
        assert_eq!(project_musicbrainz_release_group_editions(&req,&body,42).sources[0].status,
            SourceStatus::InvalidResponse);
        let empty = json!({"release-count":0,"release-offset":0,"releases":[]});
        assert_eq!(project_musicbrainz_release_group_editions(&req,&empty,42).sources[0].status,
            SourceStatus::NoOffers);
    }

    #[test]
    fn source_links_preserve_future_provider_and_unknown_country() {
        let req = request(MediaKind::Artist, Some("US"));
        let body = json!({"id":MBID,"relations":[
            relation("free streaming","https://future-music.example.org/artists/42"),
            relation("purchase for download","https://music.apple.com/gb/artist/657515")
        ]});
        let output = project_musicbrainz_relations(&req, &body, 42);
        assert_eq!(output.offers.len(), 2);
        assert!(output.offers.iter().all(|offer| offer.country.is_none() && !offer.evidence.region_verified));
        assert!(output.offers.iter().any(|offer| offer.service.id == "future-music.example.org"));
        assert_eq!(output.sources[0].scope, CoverageScope::LinkedAccess);
        assert_eq!(output.sources[0].coverage, Coverage::Partial);
    }

    #[test]
    fn ended_reverse_and_unsafe_destinations_never_become_offers() {
        let mut ended = relation("free streaming", "https://old.example.org/song");
        ended["ended"] = json!(true);
        let mut reverse = relation("free streaming", "https://reverse.example.org/song");
        reverse["direction"] = json!("backward");
        let body = json!({"id":MBID,"relations":[ended,reverse,
            relation("free streaming","http://plain.example.org/song"),
            relation("free streaming","https://127.0.0.1/song"),
            relation("free streaming","https://user:pass@music.example.org/song"),
            relation("official homepage","https://site.example.org/")
        ]});
        let output = project_musicbrainz_relations(&request(MediaKind::Artist,None), &body, 42);
        assert!(output.offers.is_empty());
        assert_eq!(output.sources[0].status, SourceStatus::Partial);
    }

    #[test]
    fn subscription_requires_the_documented_relationship_type() {
        let mut paid = relation("streaming", "https://tidal.com/artist/64518");
        paid["type-id"] = json!("63cc5d1f-f096-4c94-a43f-ecb32ea94161");
        let body = json!({"id":MBID,"relations":[paid,
            relation("streaming","https://new.example.org/artist"),
            relation("apple music","https://music.apple.com/gb/artist/657515")
        ]});
        let output = project_musicbrainz_relations(&request(MediaKind::Artist,None), &body, 42);
        assert_eq!(output.offers.iter().filter(|offer| offer.access_type == AccessType::Subscription).count(), 1);
        assert_eq!(output.offers.iter().filter(|offer| offer.access_type == AccessType::Other).count(), 2);
    }

    #[test]
    fn wrong_mbid_and_missing_relations_are_errors_not_empty_catalogs() {
        let req = request(MediaKind::Artist,None);
        let mismatch = project_musicbrainz_relations(&req, &json!({"id":"other","relations":[]}), 42);
        assert_eq!(mismatch.sources[0].status, SourceStatus::IdentityMismatch);
        let malformed = project_musicbrainz_relations(&req, &json!({"id":MBID}), 42);
        assert_eq!(malformed.sources[0].status, SourceStatus::InvalidResponse);
    }

    #[test]
    fn typed_ids_cannot_be_ambiguous_or_used_as_paths() {
        let mut req = request(MediaKind::Artist,None);
        req.identity.ids.push(CatalogId {
            source:"musicbrainz".into(), id:"11111111-1111-4111-8111-111111111111".into(),
        });
        assert!(musicbrainz_entity(&req.identity).is_none());
        req.identity.ids.truncate(1);
        req.identity.ids[0].id = "../release/657515".into();
        assert!(musicbrainz_entity(&req.identity).is_none());
    }

    #[test]
    fn distinct_access_types_survive_deduplication_and_refresh_ids_stay_stable() {
        let free = relation("free streaming", "https://shop.example.org/album/1");
        let buy = relation("purchase for download", "https://shop.example.org/album/1");
        let body = json!({"id":MBID,"relations":[free.clone(),free,buy]});
        let req = request(MediaKind::Release,None);
        let one = project_musicbrainz_relations(&req,&body,42);
        let two = project_musicbrainz_relations(&req,&body,100);
        assert_eq!(one.offers.len(),2);
        assert_eq!(one.offers.iter().map(|offer| &offer.id).collect::<Vec<_>>(),
                   two.offers.iter().map(|offer| &offer.id).collect::<Vec<_>>());
    }

    #[test]
    fn exact_apple_ids_follow_the_requested_music_entity() {
        let mut req = request(MediaKind::Recording,Some("US"));
        let body = json!({"id":MBID,"relations":[
            relation("purchase for download","https://music.apple.com/gb/album/example/100?i=200"),
            relation("streaming","https://music.apple.com/us/song/example/200"),
            relation("streaming","https://music.apple.com/us/artist/example/300"),
            relation("streaming","https://evil.example.org/us/song/example/400")
        ]});
        assert_eq!(itunes_music_ids(&req.identity,Some(&body)),vec!["200"]);
        req.identity.kind = MediaKind::Release;
        assert_eq!(itunes_music_ids(&req.identity,Some(&body)),vec!["100"]);
    }

    fn song() -> Value {
        json!({"wrapperType":"track","kind":"song","trackId":200,
            "trackViewUrl":"https://music.apple.com/us/album/example/100?i=200",
            "trackPrice":1.29,"currency":"USD"})
    }

    #[test]
    fn itunes_purchase_needs_exact_identity_and_returned_storefront() {
        let req = request(MediaKind::Recording,Some("US"));
        let body = json!({"resultCount":1,"results":[song()]});
        let output = project_itunes_music_lookup(&req,&["200".into()],&body,42);
        assert_eq!(output.offers.len(),1);
        let offer = &output.offers[0];
        assert_eq!(offer.access_type,AccessType::Buy);
        assert_eq!(offer.country.as_ref().map(CountryCode::as_str),Some("US"));
        assert!(offer.evidence.region_verified);
        assert_eq!(offer.price.as_ref().and_then(|p| p.amount.as_deref()),Some("1.29"));
        let different = project_itunes_music_lookup(&req,&["201".into()],&body,42);
        assert!(different.offers.is_empty());
        assert_eq!(different.sources[0].status,SourceStatus::IdentityMismatch);
        let wrong_country = project_itunes_music_lookup(
            &request(MediaKind::Recording,Some("GB")),&["200".into()],&body,42);
        assert!(wrong_country.offers.is_empty());
        assert_eq!(wrong_country.sources[0].status,SourceStatus::UnsupportedCountry);
    }

    #[test]
    fn missing_country_never_defaults_to_us_and_unpriced_is_not_subscription() {
        let body = json!({"resultCount":1,"results":[song()]});
        let output = project_itunes_music_lookup(
            &request(MediaKind::Recording,None),&["200".into()],&body,42);
        assert_eq!(output.sources[0].status,SourceStatus::UnsupportedCountry);
        let mut row = song();
        row["trackPrice"] = json!(-1);
        row["isStreamable"] = json!(true);
        let output = project_itunes_music_lookup(
            &request(MediaKind::Recording,Some("US")),&["200".into()],
            &json!({"resultCount":1,"results":[row]}),42);
        assert_eq!(output.offers[0].access_type,AccessType::Other);
        assert!(output.offers[0].price.is_none());
    }
}
