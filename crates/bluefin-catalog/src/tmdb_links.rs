//! Projects public offer anchors from one successfully fetched TMDB watch page.
//! No I/O, script execution, redirect following, or invented app links.
//! The caller bounds normal HTTP acquisition and caches by title and country.
use alloc::{format, string::{String, ToString}, vec, vec::Vec};
use base64::{engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD}, Engine};
use serde_json::Value;
use url::Url;
use crate::{
    offer_id, safe_https_url, AccessOffer, AccessType, Attribution, CountryCode, Coverage,
    CoverageScope, Destination, DestinationKind, LookupRequest, MediaKind, OfferEvidence,
    OfferResponse, Price, ServiceIdentity, SourceObservation, SourceStatus,
};

const SOURCE: &str = "tmdb-watch-page";
const MAX_HTML: usize = 2_500_000;

/// page_url is the actual final successful response URL with an explicit locale.
/// A request without a country retains this page's proved region; it does not
/// turn a single page into evidence for all countries.
pub fn project_tmdb_watch_page(
    request: &LookupRequest, page_url: &str, html: &str, observed_at: u64,
) -> OfferResponse {
    let expected_kind = match request.identity.kind {
        MediaKind::Movie => "movie", MediaKind::Series => "tv",
        _ => return failure(request, SourceStatus::UnsupportedIdentity, observed_at),
    };
    let Some(expected_id) = request.identity.id("tmdb").and_then(numeric_id) else {
        return failure(request, SourceStatus::UnsupportedIdentity, observed_at);
    };
    if html.is_empty() || html.len() > MAX_HTML {
        return failure(request, SourceStatus::InvalidResponse, observed_at);
    }
    let Some(page) = tmdb_url(page_url) else {
        return failure(request, SourceStatus::InvalidResponse, observed_at);
    };
    if tmdb_identity(&page) != Some((expected_kind.to_string(), expected_id)) ||
        !page.path().trim_end_matches('/').ends_with("/watch") {
        return failure(request, SourceStatus::IdentityMismatch, observed_at);
    }
    let Some(country) = unique_query(&page, "locale").and_then(|s| CountryCode::parse(&s).ok()) else {
        return failure(request, SourceStatus::InvalidResponse, observed_at);
    };
    if request.country.as_ref().is_some_and(|c| c != &country) {
        return failure(request, SourceStatus::UnsupportedCountry, observed_at);
    }
    let mut result = OfferResponse::empty(request);
    let mut canonical_seen = false;
    let mut mismatch = false;
    let mut partial = false;
    let mut candidates = 0;
    let mut source_title = None;
    let mut attribution = String::from("https://www.justwatch.com/");
    let mut anchor_offer = None;
    let mut stack: Vec<(String, bool)> = Vec::new();
    for (count, tag) in Tags::new(html).enumerate() {
        if count >= 50_000 || stack.len() > 512 {
            return failure(request, SourceStatus::InvalidResponse, observed_at);
        }
        if tag.closing {
            if let Some(index) = stack.iter().rposition(|(name, _)| name == &tag.name) {
                stack.truncate(index);
            }
            if tag.name == "a" || !stack.iter().any(|(name, _)| name == "a") { anchor_offer = None; }
            continue;
        }
        let hidden = stack.last().is_some_and(|(_, h)| *h) || tag.hidden() ||
            matches!(tag.name.as_str(), "template" | "script" | "style" | "noscript");
        if !tag.empty && !is_void(&tag.name) { stack.push((tag.name.clone(), hidden)); }
        if hidden { continue; }
        let identity_link = if tag.name == "link" &&
            tag.attr("rel").is_some_and(|r| r.split_ascii_whitespace().any(|v| v.eq_ignore_ascii_case("canonical"))) {
            tag.attr("href")
        } else if tag.name == "meta" && tag.attr("property") == Some("og:url") {
            tag.attr("content")
        } else { None };
        if let Some(link) = identity_link {
            canonical_seen = true;
            mismatch |= tmdb_url(link).and_then(|u| tmdb_identity(&u)) !=
                Some((expected_kind.to_string(), expected_id));
        }
        if tag.name == "a" {
            anchor_offer = None;
            let Some(href) = tag.attr("href") else { continue };
            let Ok(link) = Url::parse(href) else { continue };
            if is_justwatch_attribution(&link) {
                if let Some(url) = safe_https_url(href) { attribution = url; }
            }
            if link.scheme() != "https" || link.host_str() != Some("click.justwatch.com") ||
                link.path() != "/a" || link.port().is_some() ||
                !link.username().is_empty() || link.password().is_some() { continue; }
            candidates += 1;
            if candidates > 4_096 { partial = true; break; }
            match project_anchor(request, &country, &link, observed_at, &mut source_title) {
                Some(offer) => {
                    partial |= offer.destinations.is_empty();
                    anchor_offer = Some(result.offers.len());
                    result.offers.push(offer);
                }
                None => partial = true,
            }
        } else if tag.name == "img" {
            if let Some(index) = anchor_offer {
                if let Some(logo) = tag.attr("src").and_then(safe_https_url) {
                    result.offers[index].service.logo_url = Some(logo);
                }
            }
        }
    }
    if mismatch { return failure(request, SourceStatus::IdentityMismatch, observed_at); }
    if !canonical_seen { return failure(request, SourceStatus::InvalidResponse, observed_at); }
    // Missing markup can mean a changed layout, not proof of no availability.
    let status = if candidates == 0 || result.offers.is_empty() { SourceStatus::InvalidResponse }
        else if partial { SourceStatus::Partial } else { SourceStatus::Complete };
    for offer in &mut result.offers { offer.id = offer_id(offer); }
    result.offers.sort_by(|a, b| a.id.cmp(&b.id));
    result.offers.dedup_by(|a, b| a.id == b.id);
    result.sources.push(SourceObservation {
        source: SOURCE.into(), status, observed_at,
        scope: CoverageScope::RegionalAvailability, coverage: Coverage::Partial,
        catalog_url: Some(page.to_string()),
        attributions: vec![
            Attribution { name: "TMDB".into(), url: page.to_string() },
            Attribution { name: "JustWatch".into(), url: attribution },
        ],
    });
    result
}

fn failure(request: &LookupRequest, status: SourceStatus, at: u64) -> OfferResponse {
    OfferResponse::with_status(request, SOURCE, status, CoverageScope::RegionalAvailability, at)
}
fn numeric_id(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) { return None; }
    s.parse::<u64>().ok().filter(|id| *id > 0)
}
fn tmdb_url(s: &str) -> Option<Url> {
    let parsed = Url::parse(&safe_https_url(s)?).ok()?;
    matches!(parsed.host_str(), Some("www.themoviedb.org" | "themoviedb.org")).then_some(parsed)
}
fn tmdb_identity(url: &Url) -> Option<(String, u64)> {
    let parts: Vec<_> = url.path_segments()?.collect();
    if parts.len() < 2 || !matches!(parts[0], "movie" | "tv") { return None; }
    let id = parts[1].split('-').next().and_then(numeric_id)?;
    if parts.iter().skip(2).any(|p| !p.is_empty() && *p != "watch") { return None; }
    Some((parts[0].into(), id))
}
fn unique_query(url: &Url, name: &str) -> Option<String> {
    let mut found = url.query_pairs().filter(|(key, _)| key == name);
    let value = found.next()?.1.into_owned();
    if found.next().is_some() { return None; }
    Some(value)
}
fn source_context<'a>(json: &'a Value, schema: &str) -> Option<&'a Value> {
    let contexts = json.get("data")?.as_array()?;
    if contexts.len() > 16 { return None; }
    let prefix = format!("iglu:com.justwatch/{schema}/jsonschema/");
    let mut found = contexts.iter().filter(|entry| entry.get("schema").and_then(Value::as_str)
        .is_some_and(|s| s.starts_with(&prefix)));
    let value = found.next()?.get("data")?;
    if found.next().is_some() { return None; }
    Some(value)
}
fn text(value: &Value, key: &str, max: usize) -> Option<String> {
    let value = value.get(key)?.as_str()?.trim();
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) { return None; }
    Some(value.into())
}
fn opaque_id(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() && s.len() <= 128 &&
            !s.chars().any(|c| c.is_control() || c.is_whitespace()) => Some(s.clone()),
        Value::Number(n) => n.as_u64().filter(|n| *n > 0).map(|n| n.to_string()),
        _ => None,
    }
}
fn project_anchor(
    request: &LookupRequest, page_country: &CountryCode, link: &Url, observed_at: u64,
    page_title: &mut Option<String>,
) -> Option<AccessOffer> {
    let country = CountryCode::parse(&unique_query(link, "uct_country")?).ok()?;
    if &country != page_country { return None; }
    let encoded = unique_query(link, "cx")?;
    if encoded.len() > 16_384 { return None; }
    let bytes = URL_SAFE_NO_PAD.decode(encoded.as_bytes()).or_else(|_| URL_SAFE.decode(encoded.as_bytes())).ok()?;
    let json: Value = serde_json::from_slice(&bytes).ok()?;
    let data = source_context(&json, "clickout_context")?;
    let title = source_context(&json, "title_context")?;
    let object_type = title.get("objectType")?.as_str()?;
    if !matches!((request.identity.kind, object_type),
        (MediaKind::Movie, "movie") | (MediaKind::Series, "show")) { return None; }
    let title_id = opaque_id(title.get("titleId")?)?;
    let title_key = format!("{object_type}:{title_id}");
    if page_title.as_ref().is_some_and(|seen| seen != &title_key) { return None; }
    if let Some(expected) = request.identity.id("justwatch") {
        if expected != title_id && title.get("jwEntityId").and_then(Value::as_str) != Some(expected) { return None; }
    }
    let provider_id = opaque_id(data.get("providerId")?)?;
    let provider_name = text(data, "provider", 256)?;
    let raw_type = text(data, "monetizationType", 64)?;
    let access_type = match raw_type.as_str() {
        "flatrate" | "subscription" => AccessType::Subscription, "rent" => AccessType::Rent,
        "buy" => AccessType::Buy, "free" => AccessType::Free, "ads" => AccessType::Ads,
        "addon" => AccessType::Addon, "download" => AccessType::Download, _ => AccessType::Other,
    };
    let mut destinations = Vec::new();
    if let Some(url) = unique_query(link, "r").and_then(|r| exact_destination(&r, &provider_name)) {
        // A provider title URL is not proof of an authenticated player or native launch.
        destinations.push(Destination { kind: DestinationKind::Title, url });
    }
    let amount = data.get("price").and_then(|price| match price {
        Value::Number(n) if n.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0) => Some(n.to_string()),
        Value::String(s) if !s.is_empty() && s.len() <= 64 &&
            s.bytes().all(|c| c.is_ascii_digit() || c == b'.') &&
            s.parse::<f64>().ok().is_some_and(|n| n.is_finite() && n >= 0.0) => Some(s.clone()),
        _ => None,
    });
    let currency = text(data, "currency", 8);
    let price = if amount.is_some() || currency.is_some() { Some(Price { amount, currency, formatted: None }) } else { None };
    *page_title = Some(title_key);
    Some(AccessOffer {
        id: String::new(), identity: request.identity.clone(), country: Some(country),
        service: ServiceIdentity { source: "tmdb".into(), id: provider_id, name: provider_name,
            home_url: None, logo_url: None },
        access_type, raw_access_type: Some(raw_type), addon: None, destinations, price,
        quality: text(data, "presentationType", 64),
        evidence: OfferEvidence { source: SOURCE.into(), observed_at, available_since: None,
            expires_at: None, expires_soon: false, region_verified: true },
    })
}
fn is_justwatch_attribution(url: &Url) -> bool {
    if url.scheme() != "https" || !matches!(url.host_str(), Some("justwatch.com" | "www.justwatch.com")) { return false; }
    let p: Vec<_> = url.path_segments().into_iter().flatten().collect();
    p.len() >= 3 && matches!(p[1], "movie" | "tv-show" | "tv-series")
}
fn exact_destination(value: &str, provider_name: &str) -> Option<String> {
    let safe = safe_https_url(value)?;
    let url = Url::parse(&safe).ok()?;
    let host = url.host_str()?.trim_end_matches('.');
    if ["themoviedb.org", "justwatch.com"].iter().any(|base| host == *base || host.ends_with(&format!(".{base}"))) { return None; }
    if host == "tv.apple.com" && !provider_name.to_ascii_lowercase().contains("apple") { return None; }
    let parts: Vec<_> = url.path_segments()?.filter(|s| !s.is_empty()).map(str::to_ascii_lowercase).collect();
    if parts.is_empty() { return None; }
    if parts.iter().any(|s| matches!(s.as_str(), "search" | "results" | "login" | "signin" | "sign-in" |
        "signup" | "sign-up" | "redirect" | "click" | "subscribe" | "register")) { return None; }
    if url.query_pairs().any(|(key, _)| matches!(key.to_ascii_lowercase().as_str(), "search" | "search_query" |
        "searchquery" | "searchterm" | "search_term" | "query" | "keyword" | "keywords" | "q")) { return None; }
    let meaningful: Vec<_> = parts.iter().filter(|s| {
        let b = s.as_bytes();
        !(b.len() == 2 && b.iter().all(u8::is_ascii_alphabetic)) &&
        !(b.len() == 5 && b[2] == b'-' && b[..2].iter().chain(b[3..].iter()).all(u8::is_ascii_alphabetic))
    }).collect();
    if meaningful.is_empty() { return None; }
    if meaningful.len() == 1 && matches!(meaningful[0].as_str(), "home" | "browse" | "movies" | "movie" |
        "tv" | "shows" | "series" | "watch" | "welcome" | "stream" | "all" | "account") { return None; }
    Some(safe)
}

// Conservative bounded tag extraction, not a DOM or script engine.
struct Tag { name: String, attrs: Vec<(String, String)>, closing: bool, empty: bool }
impl Tag {
    fn attr(&self, key: &str) -> Option<&str> { self.attrs.iter().find(|(n, _)| n == key).map(|(_, v)| v.as_str()) }
    fn hidden(&self) -> bool {
        self.attr("hidden").is_some() || self.attr("aria-hidden").is_some_and(|v| v.eq_ignore_ascii_case("true")) ||
            self.attr("style").is_some_and(|s| {
                let compact: String = s.chars().filter(|c| !c.is_ascii_whitespace()).collect();
                let lower = compact.to_ascii_lowercase();
                lower.contains("display:none") || lower.contains("visibility:hidden")
            })
    }
}
struct Tags<'a> { html: &'a str, offset: usize, raw: Option<String> }
impl<'a> Tags<'a> { fn new(html: &'a str) -> Self { Self { html, offset: 0, raw: None } } }
impl Iterator for Tags<'_> {
    type Item = Tag;
    fn next(&mut self) -> Option<Self::Item> {
        let bytes = self.html.as_bytes();
        loop {
            if self.offset >= bytes.len() { return None; }
            if let Some(raw) = self.raw.take() {
                let needle = format!("</{raw}");
                let found = bytes[self.offset..].windows(needle.len()).position(|w| w.eq_ignore_ascii_case(needle.as_bytes()))?;
                self.offset += found;
            }
            let start = self.offset + self.html[self.offset..].find('<')?;
            self.offset = start + 1;
            if self.html[start..].starts_with("<!--") {
                self.offset = start + 4 + self.html[start + 4..].find("-->")? + 3;
                continue;
            }
            let mut at = start + 1;
            let closing = bytes.get(at) == Some(&b'/');
            if closing { at += 1; }
            let from = at;
            while bytes.get(at).is_some_and(|c| c.is_ascii_alphanumeric() || matches!(*c, b'-' | b':')) { at += 1; }
            if at == from || !bytes[from].is_ascii_alphabetic() { continue; }
            let name = self.html[from..at].to_ascii_lowercase();
            let mut end = at;
            let mut quote = 0;
            while let Some(&c) = bytes.get(end) {
                if quote != 0 { if c == quote { quote = 0; } }
                else if c == b'\'' || c == b'"' { quote = c; }
                else if c == b'>' { break; }
                end += 1;
            }
            if end >= bytes.len() { return None; }
            self.offset = end + 1;
            if end - start > 32_768 { continue; }
            let empty = bytes[..end].last() == Some(&b'/');
            let Some(attrs) = attributes(&self.html[at..end]) else { continue };
            if !closing && matches!(name.as_str(), "script" | "style" | "title" | "textarea" | "xmp" | "iframe" | "noembed") {
                self.raw = Some(name.clone());
            }
            return Some(Tag { name, attrs, closing, empty });
        }
    }
}
fn attributes(raw: &str) -> Option<Vec<(String, String)>> {
    let b = raw.as_bytes();
    let mut at = 0;
    let mut attrs = Vec::new();
    while at < b.len() {
        while b.get(at).is_some_and(u8::is_ascii_whitespace) { at += 1; }
        if at >= b.len() || b[at] == b'/' { break; }
        let from = at;
        while b.get(at).is_some_and(|c| !c.is_ascii_whitespace() && !matches!(*c, b'=' | b'/' | b'>' | b'<' | b'\'' | b'"')) { at += 1; }
        if from == at { return None; }
        let name = raw[from..at].to_ascii_lowercase();
        if attrs.iter().any(|(key, _)| key == &name) { return None; }
        while b.get(at).is_some_and(u8::is_ascii_whitespace) { at += 1; }
        let mut value = String::new();
        if b.get(at) == Some(&b'=') {
            at += 1;
            while b.get(at).is_some_and(u8::is_ascii_whitespace) { at += 1; }
            let quote = b.get(at).copied().filter(|c| *c == b'\'' || *c == b'"');
            if quote.is_some() { at += 1; }
            let from = at;
            if let Some(q) = quote {
                while b.get(at).is_some_and(|c| *c != q) { at += 1; }
                if at >= b.len() { return None; }
                value = entities(&raw[from..at]); at += 1;
            } else {
                while b.get(at).is_some_and(|c| !c.is_ascii_whitespace() && *c != b'>') { at += 1; }
                value = entities(&raw[from..at]);
            }
        }
        attrs.push((name, value));
        if attrs.len() > 64 { return None; }
    }
    Some(attrs)
}
fn entities(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(index) = rest.find('&') {
        out.push_str(&rest[..index]); rest = &rest[index..];
        let Some(end) = rest.find(';').filter(|end| *end <= 12) else {
            out.push('&'); rest = &rest[1..]; continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'), "quot" => Some('"'), "apos" => Some('\''), "lt" => Some('<'), "gt" => Some('>'),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => u32::from_str_radix(&entity[2..], 16).ok().and_then(char::from_u32),
            _ if entity.starts_with('#') => entity[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        if let Some(c) = decoded { out.push(c); rest = &rest[end + 1..]; }
        else { out.push('&'); rest = &rest[1..]; }
    }
    out.push_str(rest); out
}
fn is_void(name: &str) -> bool {
    matches!(name, "area" | "base" | "br" | "col" | "embed" | "hr" | "img" | "input" |
        "link" | "meta" | "param" | "source" | "track" | "wbr")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CatalogId, CatalogIdentity};
    fn request(kind: MediaKind, id: &str, country: Option<&str>) -> LookupRequest {
        LookupRequest { identity: CatalogIdentity { kind, ids: vec![CatalogId { source: "tmdb".into(), id: id.into() }] },
            country: country.map(|s| CountryCode::parse(s).unwrap()) }
    }
    fn fixture(raw: &str) -> (String, String) {
        let json: Value = serde_json::from_str(raw).unwrap();
        let page = json["sourceUrl"].as_str().unwrap().to_string();
        let mut html = format!("<html><head><link rel='canonical' href='{page}'></head><body>");
        for item in json["anchors"].as_array().unwrap() {
            let href = item["href"].as_str().unwrap().replace('&', "&amp;").replace('"', "&quot;");
            html.push_str(&format!("<a href=\"{href}\">"));
            if let Some(logo) = item["logo"].as_str() { html.push_str(&format!("<img src=\"{logo}\">")); }
            html.push_str("</a>");
        }
        html.push_str("</body></html>"); (page, html)
    }
    fn anchor(destination: &str, country: &str, category: &str, provider: &str) -> String {
        let context = serde_json::json!({"data":[
            {"schema":"iglu:com.justwatch/clickout_context/jsonschema/1-3-2","data":{"provider":provider,"providerId":"future-service",
             "monetizationType":category,"presentationType":"4k","currency":"USD","price":2.99}},
            {"schema":"iglu:com.justwatch/title_context/jsonschema/1-3-0","data":{"titleId":12881,"objectType":"movie","jwEntityId":"tm12881"}}
        ]});
        let cx = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&context).unwrap());
        let mut url = Url::parse("https://click.justwatch.com/a").unwrap();
        url.query_pairs_mut().append_pair("cx", &cx).append_pair("r", destination).append_pair("uct_country", country);
        format!("<a href=\"{}\"></a>", url.as_str().replace('&', "&amp;"))
    }
    fn envelope(anchor: &str) -> String {
        format!("<link rel=canonical href='https://www.themoviedb.org/movie/10331/watch'>{anchor}")
    }
    const NIGHT_US: &str = include_str!("../tests/fixtures/tmdb-watch/movie-10331-US.json");
    const NIGHT_GB: &str = include_str!("../tests/fixtures/tmdb-watch/movie-10331-GB.json");
    const BREAKING_US: &str = include_str!("../tests/fixtures/tmdb-watch/tv-1396-US.json");
    const PAGE: &str = "https://www.themoviedb.org/movie/10331/watch?locale=US";

    #[test]
    fn live_movie_preserves_categories_small_services_prices_and_evidence() {
        let (page, html) = fixture(NIGHT_US);
        let result = project_tmdb_watch_page(&request(MediaKind::Movie, "10331", Some("US")), &page, &html, 123);
        for kind in [AccessType::Ads, AccessType::Buy, AccessType::Rent, AccessType::Free, AccessType::Subscription] {
            assert!(result.offers.iter().any(|o| o.access_type == kind), "missing {kind:?}");
        }
        for name in ["Tubi TV", "Pluto TV", "Kanopy", "Hoopla", "Bloodstream", "Brew", "Fawesome"] {
            assert!(result.offers.iter().any(|o| o.service.name == name), "missing {name}");
        }
        assert!(result.offers.iter().all(|o| o.country.as_ref().unwrap().as_str() == "US"));
        assert!(result.offers.iter().all(|o| o.evidence.observed_at == 123 && o.evidence.region_verified));
        assert!(result.offers.iter().any(|o| o.service.logo_url.is_some()));
        assert!(result.offers.iter().any(|o| o.price.as_ref().and_then(|p| p.amount.as_deref()) == Some("2.99")));
        assert!(result.sources[0].attributions.iter().any(|a| a.name == "JustWatch"));
    }
    #[test]
    fn observed_html_structure_preserves_the_same_public_offers() {
        let (page, constructed) = fixture(NIGHT_US);
        let request = request(MediaKind::Movie, "10331", Some("US"));
        let expected = project_tmdb_watch_page(&request, &page, &constructed, 1);
        let observed = include_str!("../tests/fixtures/tmdb-watch/movie-10331-US.structure.html");
        let actual = project_tmdb_watch_page(&request, &page, observed, 1);
        assert!(!actual.offers.is_empty());
        let expected_ids: Vec<_> = expected.offers.iter().map(|o| &o.id).collect();
        let actual_ids: Vec<_> = actual.offers.iter().map(|o| &o.id).collect();
        assert_eq!(actual_ids, expected_ids);
    }
    #[test]
    fn live_regions_and_tv_identity_remain_distinct() {
        let (page, html) = fixture(NIGHT_GB);
        let result = project_tmdb_watch_page(&request(MediaKind::Movie, "10331", None), &page, &html, 1);
        assert!(result.country.is_none());
        assert!(!result.offers.is_empty());
        assert!(result.offers.iter().all(|o| o.country.as_ref().unwrap().as_str() == "GB"));
        assert!(result.offers.iter().any(|o| o.service.name == "BFI Player"));
        let (page, html) = fixture(BREAKING_US);
        let result = project_tmdb_watch_page(&request(MediaKind::Series, "1396", Some("US")), &page, &html, 1);
        assert!(result.offers.iter().any(|o| o.service.name == "Netflix" &&
            o.destinations.iter().any(|d| d.url == "https://www.netflix.com/title/70143836")));
        assert_eq!(result.sources[0].status, SourceStatus::Complete);
    }
    #[test]
    fn generic_links_keep_availability_without_actions() {
        let (page, html) = fixture(NIGHT_US);
        let result = project_tmdb_watch_page(&request(MediaKind::Movie, "10331", Some("US")), &page, &html, 1);
        assert!(result.offers.iter().filter(|o| o.service.name == "YouTube").all(|o| o.destinations.is_empty()));
        assert!(result.offers.iter().filter(|o| o.service.name == "Starz Apple TV channel").all(|o| o.destinations.is_empty()));
        assert_eq!(result.sources[0].status, SourceStatus::Partial);
        for url in ["https://video.example/", "https://video.example/en-us", "https://video.example/search?q=title",
            "https://www.justwatch.com/us/movie/title", "http://video.example/title/12", "https://localhost/title/12",
            "https://127.0.0.1/title/12", "https://video.example/login?title=12",
            "https://user:password@video.example/title/12", "https://tv.apple.com/us/movie/title/id"] {
            assert!(exact_destination(url, "Unrelated service").is_none(), "{url}");
        }
    }
    #[test]
    fn identity_country_and_duplicate_parameters_fail_closed() {
        let req = request(MediaKind::Movie, "10331", Some("US"));
        let anchor = anchor("https://new.example/title/abc", "us", "free", "New service");
        let wrong = envelope(&anchor).replace("movie/10331/watch'", "movie/603/watch'");
        assert_eq!(project_tmdb_watch_page(&req, PAGE, &wrong, 1).sources[0].status, SourceStatus::IdentityMismatch);
        assert_eq!(project_tmdb_watch_page(&req, &PAGE.replace("locale=US", "locale=GB"), &envelope(&anchor), 1)
            .sources[0].status, SourceStatus::UnsupportedCountry);
        assert!(project_tmdb_watch_page(&req, PAGE, &envelope(&anchor.replace("uct_country=us", "uct_country=gb")), 1).offers.is_empty());
        assert!(project_tmdb_watch_page(&req, PAGE, &envelope(&anchor.replace("uct_country=us", "uct_country=us&amp;uct_country=gb")), 1).offers.is_empty());
        assert!(project_tmdb_watch_page(&req, PAGE, "<html>Sign in</html>", 1).offers.is_empty());
    }
    #[test]
    fn unknown_services_and_categories_are_dynamic_and_deduplicated() {
        let a = anchor("https://brand-new.example/title/abc", "us", "future-pass", "New provider");
        let result = project_tmdb_watch_page(&request(MediaKind::Movie, "10331", Some("US")), PAGE, &envelope(&format!("{a}{a}")), 1);
        assert_eq!(result.offers.len(), 1);
        let o = &result.offers[0];
        assert_eq!(o.service.id, "future-service"); assert_eq!(o.service.source, "tmdb");
        assert_eq!(o.access_type, AccessType::Other); assert_eq!(o.raw_access_type.as_deref(), Some("future-pass"));
        assert_eq!(o.quality.as_deref(), Some("4k")); assert_eq!(o.destinations[0].url, "https://brand-new.example/title/abc");
    }
    #[test]
    fn hidden_and_non_anchor_content_cannot_supply_offers() {
        let a = anchor("https://new.example/title/abc", "us", "free", "New provider");
        for hidden in [format!("<!-- {a} -->"), format!("<script>{a}</script>"), format!("<template>{a}</template>"),
            format!("<div hidden>{a}</div>"), format!("<div aria-hidden='true'>{a}</div>"), format!("<div style='display: none'>{a}</div>")] {
            assert!(project_tmdb_watch_page(&request(MediaKind::Movie, "10331", Some("US")), PAGE, &envelope(&hidden), 1).offers.is_empty());
        }
    }
}
