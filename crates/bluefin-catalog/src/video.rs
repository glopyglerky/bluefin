//! Pure projections of documented TMDB and Movie of the Night v4 responses.
use alloc::{format, string::{String, ToString}, vec, vec::Vec};
use serde_json::Value;
use crate::*;

pub const TMDB_SOURCE: &str = "tmdb";
pub const MOVIE_OF_THE_NIGHT_SOURCE: &str = "movieofthenight";

fn string(value: &Value, name: &str) -> Option<String> {
    value.get(name)?.as_str().filter(|s| !s.trim().is_empty()).map(ToString::to_string)
}
fn id_string(value: &Value) -> Option<String> {
    value.as_u64().filter(|id| *id > 0).map(|id| id.to_string())
        .or_else(|| value.as_str().filter(|id| !id.is_empty() && id.len() < 128).map(ToString::to_string))
}
fn consistent_id<'a>(identity: &'a CatalogIdentity, source: &str) -> Result<Option<&'a str>, SourceStatus> {
    let mut matching = identity.ids.iter().filter(|id| id.source.eq_ignore_ascii_case(source));
    let first = matching.next().map(|id| id.id.as_str());
    if matching.any(|id| Some(id.id.as_str()) != first) { return Err(SourceStatus::IdentityMismatch); }
    Ok(first)
}

/// Exact TMDB resource key, normalized using the explicit media kind.
pub fn tmdb_key(identity: &CatalogIdentity) -> Result<String, SourceStatus> {
    let prefix = match identity.kind {
        MediaKind::Movie => "movie", MediaKind::Series => "tv", _ => return Err(SourceStatus::UnsupportedIdentity),
    };
    let value = consistent_id(identity, TMDB_SOURCE)?.ok_or(SourceStatus::UnsupportedIdentity)?;
    let id = if let Some((kind, id)) = value.split_once('/') {
        if kind != prefix { return Err(SourceStatus::IdentityMismatch); }
        id
    } else { value };
    if id.is_empty() || id.len() > 20 || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(SourceStatus::UnsupportedIdentity);
    }
    let id: u64 = id.parse().map_err(|_| SourceStatus::UnsupportedIdentity)?;
    if id == 0 { return Err(SourceStatus::UnsupportedIdentity); }
    Ok(format!("{prefix}/{id}"))
}
/// Validated exact IMDb identifier, never a title search.
pub fn imdb_key(identity: &CatalogIdentity) -> Result<String, SourceStatus> {
    let value = consistent_id(identity, "imdb")?.ok_or(SourceStatus::UnsupportedIdentity)?;
    if !value.starts_with("tt") || value.len() < 9 || value.len() > 16 ||
        !value.as_bytes()[2..].iter().all(u8::is_ascii_digit) {
        return Err(SourceStatus::UnsupportedIdentity);
    }
    Ok(value.to_string())
}
/// Preferred exact identifier accepted by the v4 show lookup endpoint.
pub fn movie_of_the_night_key(identity: &CatalogIdentity) -> Result<String, SourceStatus> {
    if !matches!(identity.kind, MediaKind::Movie | MediaKind::Series) { return Err(SourceStatus::UnsupportedIdentity); }
    if consistent_id(identity, TMDB_SOURCE)?.is_some() { return tmdb_key(identity); }
    if consistent_id(identity, "imdb")?.is_some() { return imdb_key(identity); }
    let value = consistent_id(identity, MOVIE_OF_THE_NIGHT_SOURCE)?.ok_or(SourceStatus::UnsupportedIdentity)?;
    if value.is_empty() || value.len() > 64 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SourceStatus::UnsupportedIdentity);
    }
    Ok(value.to_string())
}
fn attribution(source: &str) -> Vec<Attribution> {
    if source == TMDB_SOURCE {
        vec![Attribution { name: "TMDB".into(), url: "https://www.themoviedb.org/".into() },
             Attribution { name: "JustWatch".into(), url: "https://www.justwatch.com/".into() }]
    } else {
        vec![Attribution { name: "Movie of the Night".into(), url: "https://www.movieofthenight.com/".into() }]
    }
}
fn observation(source: &str, status: SourceStatus, observed_at: u64, coverage: Coverage) -> SourceObservation {
    SourceObservation { source: source.into(), status, observed_at,
        scope: CoverageScope::RegionalAvailability, coverage, catalog_url: None,
        attributions: attribution(source) }
}
fn failure(request: &LookupRequest, source: &str, status: SourceStatus, observed_at: u64) -> OfferResponse {
    let mut response = OfferResponse::empty(request);
    response.sources.push(observation(source, status, observed_at, Coverage::Unknown));
    response
}
fn finish(response: &mut OfferResponse, source: &str, observed_at: u64, invalid: bool, catalog_url: Option<String>) {
    response.offers.sort_by(|a,b| a.id.cmp(&b.id));
    response.offers.dedup_by(|a,b| a.id == b.id);
    let status = if invalid { SourceStatus::Partial } else if response.offers.is_empty() {
        SourceStatus::NoOffers
    } else { SourceStatus::Complete };
    let mut source = observation(source, status, observed_at,
        if invalid { Coverage::Partial } else { Coverage::SourceCatalog });
    source.catalog_url = catalog_url;
    response.sources.push(source);
}

/// Projects country availability only. TMDB's link is a catalog page, not a provider deep link.
/// A host using a compatible Seerr response must preserve/verify its exact TMDB ID before projection.
pub fn project_tmdb_watch_providers(request: &LookupRequest, body: &Value, observed_at: u64) -> OfferResponse {
    let key = match tmdb_key(&request.identity) {
        Ok(key) => key, Err(status) => return failure(request, TMDB_SOURCE, status, observed_at),
    };
    let expected_id = key.split('/').nth(1).and_then(|id| id.parse::<u64>().ok());
    if body.get("id").and_then(Value::as_u64) != expected_id {
        return failure(request, TMDB_SOURCE, if body.get("id").is_some() { SourceStatus::IdentityMismatch }
            else { SourceStatus::InvalidResponse }, observed_at);
    }
    let Some(regions) = body.get("results").and_then(Value::as_object) else {
        return failure(request, TMDB_SOURCE, SourceStatus::InvalidResponse, observed_at);
    };
    let mut response = OfferResponse::empty(request);
    let mut invalid = false;
    let mut catalog_url = None;
    for (region, data) in regions {
        let country = match CountryCode::parse(region) {
            Ok(country) => country, Err(_) => { invalid = true; continue; }
        };
        if request.country.as_ref().is_some_and(|wanted| *wanted != country) { continue; }
        let Some(categories) = data.as_object() else { invalid = true; continue; };
        if request.country.is_some() {
            catalog_url = data.get("link").and_then(Value::as_str).and_then(safe_https_url);
        }
        for (category, providers) in categories {
            if category == "link" { continue; }
            if providers.is_null() { continue; }
            let Some(providers) = providers.as_array() else { invalid = true; continue; };
            let access_type = match category.as_str() {
                "flatrate" | "subscription" => AccessType::Subscription,
                "rent" => AccessType::Rent, "buy" => AccessType::Buy,
                "free" => AccessType::Free, "ads" => AccessType::Ads,
                "addon" => AccessType::Addon, _ => AccessType::Other,
            };
            for provider in providers {
                let Some(service) = tmdb_service(provider) else { invalid = true; continue; };
                let mut offer = AccessOffer {
                    id: String::new(), identity: request.identity.clone(), country: Some(country.clone()),
                    service, access_type, raw_access_type: Some(category.clone()), addon: None,
                    destinations: Vec::new(), price: None, quality: None,
                    evidence: OfferEvidence { source: TMDB_SOURCE.into(), observed_at, available_since: None,
                        expires_at: None, expires_soon: false, region_verified: true },
                };
                offer.id = offer_id(&offer);
                response.offers.push(offer);
            }
        }
    }
    if request.country.is_none() { catalog_url = Some(format!("https://www.themoviedb.org/{key}/watch")); }
    finish(&mut response, TMDB_SOURCE, observed_at, invalid, catalog_url);
    response
}

fn tmdb_service(value: &Value) -> Option<ServiceIdentity> {
    let id = id_string(value.get("provider_id")?)?;
    let name = string(value, "provider_name")?;
    let logo_url = value.get("logo_path").and_then(Value::as_str)
        .filter(|path| path.starts_with('/') && !path.starts_with("//") && !path.contains(".."))
        .and_then(|path| safe_https_url(&format!("https://image.tmdb.org/t/p/w185{path}")));
    Some(ServiceIdentity { source: TMDB_SOURCE.into(), id, name, home_url: None, logo_url })
}
fn motn_service(value: &Value) -> Option<ServiceIdentity> {
    let id = id_string(value.get("id")?)?;
    let name = string(value, "name")?;
    let home_url = value.get("homePage").and_then(Value::as_str).and_then(safe_https_url);
    let logo_url = value.get("imageSet").and_then(|images|
        ["lightThemeImage", "darkThemeImage", "whiteImage"].iter()
            .find_map(|field| images.get(field).and_then(Value::as_str).and_then(safe_https_url)));
    Some(ServiceIdentity { source: MOVIE_OF_THE_NIGHT_SOURCE.into(), id, name, home_url, logo_url })
}
fn direct_video_destination(value: &str, service: &ServiceIdentity) -> Option<String> {
    let normalized = safe_https_url(value)?;
    let parsed = url::Url::parse(&normalized).ok()?;
    let host = parsed.host_str()?.trim_end_matches('.');
    if host == "themoviedb.org" || host.ends_with(".themoviedb.org") ||
        host == "justwatch.com" || host.ends_with(".justwatch.com") {
        return None;
    }
    let path = parsed.path().trim_matches('/').to_ascii_lowercase();
    if path.is_empty() && parsed.query().is_none() && parsed.fragment().is_none() { return None; }
    if path.split('/').any(|segment| matches!(segment, "search" | "search-results" | "results")) {
        return None;
    }
    if service.home_url.as_deref().and_then(safe_https_url).as_ref() == Some(&normalized) {
        return None;
    }
    Some(normalized)
}
fn motn_type(value: &str) -> AccessType {
    match value {
        "subscription" => AccessType::Subscription, "rent" => AccessType::Rent,
        "buy" => AccessType::Buy, "free" => AccessType::Free,
        "ads" => AccessType::Ads, "addon" => AccessType::Addon, _ => AccessType::Other,
    }
}
fn validate_motn_identity(request: &LookupRequest, body: &Value) -> Result<(), SourceStatus> {
    movie_of_the_night_key(&request.identity)?;
    let expected_kind = match request.identity.kind {
        MediaKind::Movie => "movie", MediaKind::Series => "series",
        _ => return Err(SourceStatus::UnsupportedIdentity),
    };
    if body.get("showType").and_then(Value::as_str) != Some(expected_kind) { return Err(SourceStatus::IdentityMismatch); }
    if body.get("itemType").and_then(Value::as_str).is_some_and(|item| item != "show") {
        return Err(SourceStatus::IdentityMismatch);
    }
    for (source, field) in [(TMDB_SOURCE, "tmdbId"), ("imdb", "imdbId"), (MOVIE_OF_THE_NIGHT_SOURCE, "id")] {
        if let Some(expected) = consistent_id(&request.identity, source)? {
            let actual = body.get(field).and_then(id_string).ok_or(SourceStatus::InvalidResponse)?;
            let expected = if source == TMDB_SOURCE { tmdb_key(&request.identity)? } else { expected.to_string() };
            if actual != expected { return Err(SourceStatus::IdentityMismatch); }
        }
    }
    Ok(())
}
/// Projects every reported region when no country is requested, and only exact country evidence otherwise.
pub fn project_movie_of_the_night(request: &LookupRequest, body: &Value, observed_at: u64) -> OfferResponse {
    if let Err(status) = validate_motn_identity(request, body) {
        return failure(request, MOVIE_OF_THE_NIGHT_SOURCE, status, observed_at);
    }
    let Some(regions) = body.get("streamingOptions").and_then(Value::as_object) else {
        return failure(request, MOVIE_OF_THE_NIGHT_SOURCE, SourceStatus::InvalidResponse, observed_at);
    };
    let mut response = OfferResponse::empty(request);
    let mut invalid = false;
    for (region, options) in regions {
        let country = match CountryCode::parse(region) { Ok(country) => country, Err(_) => { invalid = true; continue; } };
        if request.country.as_ref().is_some_and(|wanted| *wanted != country) { continue; }
        let Some(options) = options.as_array() else { invalid = true; continue; };
        for option in options {
            let Some(service) = option.get("service").and_then(motn_service) else { invalid = true; continue; };
            let Some(raw_type) = string(option, "type") else { invalid = true; continue; };
            let addon = option.get("addon").filter(|a| !a.is_null()).and_then(motn_service);
            if raw_type == "addon" && addon.is_none() { invalid = true; }
            let expires_at = option.get("expiresOn").and_then(Value::as_u64).filter(|time| *time > 0);
            if expires_at.is_some_and(|time| time <= observed_at) { continue; }
            let available_since = option.get("availableSince").and_then(Value::as_u64);
            if available_since.is_some_and(|time| time > observed_at) { continue; }
            let mut destinations = Vec::new();
            if let Some(url) = option.get("link").and_then(Value::as_str).and_then(|link| direct_video_destination(link, &service)) {
                destinations.push(Destination { kind: DestinationKind::Title, url });
            } else { invalid = true; }
            if let Some(link) = option.get("videoLink").filter(|link| !link.is_null()) {
                if let Some(url) = link.as_str().and_then(|link| direct_video_destination(link, &service)) {
                    destinations.push(Destination { kind: DestinationKind::Player, url });
                } else { invalid = true; }
            }
            let price = option.get("price").filter(|price| price.is_object()).map(|price| Price {
                amount: string(price, "amount"), currency: string(price, "currency"), formatted: string(price, "formatted"),
            });
            let mut offer = AccessOffer {
                id: String::new(), identity: request.identity.clone(), country: Some(country.clone()),
                service, access_type: motn_type(&raw_type), raw_access_type: Some(raw_type), addon, destinations,
                price, quality: string(option, "quality"),
                evidence: OfferEvidence { source: MOVIE_OF_THE_NIGHT_SOURCE.into(), observed_at, available_since,
                    expires_at, expires_soon: option.get("expiresSoon").and_then(Value::as_bool).unwrap_or(false),
                    region_verified: true },
            };
            offer.id = offer_id(&offer);
            response.offers.push(offer);
        }
    }
    finish(&mut response, MOVIE_OF_THE_NIGHT_SOURCE, observed_at, invalid, None);
    response
}

/// The v4 country/service registry contains source-owned IDs and all known access modes.
pub fn project_movie_of_the_night_registry(country: Option<&CountryCode>, body: &Value, observed_at: u64) -> RegistryResponse {
    let mut response = RegistryResponse::default();
    let Some(regions) = body.as_object() else {
        response.sources.push(observation(MOVIE_OF_THE_NIGHT_SOURCE, SourceStatus::InvalidResponse, observed_at, Coverage::Unknown));
        return response;
    };
    let mut invalid = false;
    for (region, item) in regions {
        let Ok(code) = CountryCode::parse(region) else { invalid = true; continue; };
        if country.is_some_and(|wanted| *wanted != code) { continue; }
        if item.get("countryCode").and_then(Value::as_str)
            .and_then(|value| CountryCode::parse(value).ok()).as_ref() != Some(&code) {
            invalid = true; continue;
        }
        let Some(name) = string(item, "name") else { invalid = true; continue; };
        let Some(services) = item.get("services").and_then(Value::as_array) else { invalid = true; continue; };
        let mut projected = Vec::new();
        for item in services {
            let Some(service) = motn_service(item) else { invalid = true; continue; };
            let mut access_types = Vec::new();
            if let Some(types) = item.get("streamingOptionTypes").and_then(Value::as_object) {
                for (kind, enabled) in types {
                    if enabled.as_bool() == Some(true) { access_types.push(motn_type(kind)); }
                }
            } else { invalid = true; }
            access_types.sort();
            access_types.dedup();
            let mut addons = Vec::new();
            if let Some(items) = item.get("addons").and_then(Value::as_array) {
                for addon in items { if let Some(addon) = motn_service(addon) { addons.push(addon); } else { invalid = true; } }
            }
            projected.push(RegistryService { service, access_types, addons });
        }
        response.countries.push(CountryRecord { code, name, services: projected });
    }
    response.countries.sort_by(|a,b| a.code.cmp(&b.code));
    response.sources.push(observation(MOVIE_OF_THE_NIGHT_SOURCE,
        if invalid { SourceStatus::Partial } else if country.is_some() && response.countries.is_empty() {
            SourceStatus::UnsupportedCountry
        } else { SourceStatus::Complete },
        observed_at, if invalid { Coverage::Partial } else { Coverage::SourceCatalog }));
    response
}

/// Projects TMDB regions and movie/TV provider registries. Registry membership does not imply an access mode.
pub fn project_tmdb_registry(country: Option<&CountryCode>, regions: &Value,
    movie_providers: &Value, tv_providers: &Value, observed_at: u64) -> RegistryResponse {
    let mut response = RegistryResponse::default();
    let Some(regions) = regions.get("results").and_then(Value::as_array) else {
        response.sources.push(observation(TMDB_SOURCE, SourceStatus::InvalidResponse, observed_at, Coverage::Unknown));
        return response;
    };
    let mut invalid = false;
    for region in regions {
        let Some(code) = region.get("iso_3166_1").and_then(Value::as_str).and_then(|c| CountryCode::parse(c).ok()) else {
            invalid = true; continue;
        };
        if country.is_some_and(|wanted| *wanted != code) { continue; }
        let Some(name) = string(region, "english_name").or_else(|| string(region, "native_name")) else { invalid = true; continue; };
        response.countries.push(CountryRecord { code, name, services: Vec::new() });
    }
    for providers in [movie_providers, tv_providers] {
        let Some(providers) = providers.get("results").and_then(Value::as_array) else { invalid = true; continue; };
        for provider in providers {
            let Some(service) = tmdb_service(provider) else { invalid = true; continue; };
            let priorities = provider.get("display_priorities").and_then(Value::as_object);
            for region in &mut response.countries {
                let included = match priorities {
                    Some(priorities) => priorities.contains_key(region.code.as_str()),
                    None => country.is_some_and(|wanted| *wanted == region.code),
                };
                if included && !region.services.iter().any(|p| p.service.id == service.id) {
                    region.services.push(RegistryService { service: service.clone(), access_types: Vec::new(), addons: Vec::new() });
                }
            }
            if priorities.is_none() && country.is_none() { invalid = true; }
        }
    }
    response.countries.sort_by(|a,b| a.code.cmp(&b.code));
    response.sources.push(observation(TMDB_SOURCE,
        if invalid { SourceStatus::Partial } else if country.is_some() && response.countries.is_empty() {
            SourceStatus::UnsupportedCountry
        } else { SourceStatus::Complete },
        observed_at, if invalid { Coverage::Partial } else { Coverage::SourceCatalog }));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn request(country: Option<&str>) -> LookupRequest {
        LookupRequest { identity: CatalogIdentity { kind: MediaKind::Movie,
            ids: vec![CatalogId { source: "tmdb".into(), id: "movie/238".into() }] },
            country: country.map(|country| CountryCode::parse(country).unwrap()) }
    }
    fn provider() -> Value { json!({"provider_id":123456,"provider_name":"A newly indexed provider","logo_path":"/logo.png"}) }
    fn tmdb() -> Value { json!({"id":238,"results":{"US":{"link":"https://www.themoviedb.org/movie/238/watch?locale=US",
        "flatrate":[provider()],"rent":[provider()],"buy":[provider()],"ads":[provider()],"free":[provider()]},
        "GB":{"free":[provider()]}}}) }
    fn motn() -> Value { json!({"itemType":"show","showType":"movie","id":"82","tmdbId":"movie/238","imdbId":"tt0068646",
        "streamingOptions":{"us":[{"service":{"id":"new-service","name":"New Service","homePage":"https://new-service.example.com",
          "imageSet":{"lightThemeImage":"https://images.example.com/logo.svg"}},"type":"rent",
          "link":"https://new-service.example.com/movie/238","videoLink":"https://new-service.example.com/watch/238",
          "quality":"uhd","price":{"amount":"3.99","currency":"USD","formatted":"$3.99"},
          "availableSince":1,"expiresSoon":false}],"gb":[]}}) }
    #[test] fn tmdb_keeps_all_categories_and_dynamic_services_without_fake_deep_links() {
        let result=project_tmdb_watch_providers(&request(Some("US")),&tmdb(),100);
        assert_eq!(result.offers.len(),5);
        assert!(result.offers.iter().all(|offer| offer.service.id=="123456" && offer.destinations.is_empty()));
        assert_eq!(result.sources[0].status,SourceStatus::Complete);
        assert!(result.sources[0].catalog_url.is_some());
    }
    #[test] fn no_country_does_not_default_to_us() {
        let result=project_tmdb_watch_providers(&request(None),&tmdb(),100);
        assert_eq!(result.offers.len(),6);
        assert!(result.country.is_none());
        assert!(result.offers.iter().any(|offer| offer.country.as_ref().unwrap().as_str()=="GB"));
    }
    #[test] fn malformed_and_wrong_identity_are_not_absence() {
        let mut body=tmdb();body["id"]=json!(999);
        assert_eq!(project_tmdb_watch_providers(&request(None),&body,100).sources[0].status,SourceStatus::IdentityMismatch);
        assert_eq!(project_tmdb_watch_providers(&request(None),&json!({"id":238}),100).sources[0].status,SourceStatus::InvalidResponse);
    }
    #[test] fn motn_preserves_exact_destinations_price_quality_and_country() {
        let result=project_movie_of_the_night(&request(Some("US")),&motn(),100);
        assert_eq!(result.sources[0].status,SourceStatus::Complete);
        assert_eq!(result.offers.len(),1);
        assert_eq!(result.offers[0].destinations.len(),2);
        assert_eq!(result.offers[0].price.as_ref().unwrap().amount.as_deref(),Some("3.99"));
        assert_eq!(result.offers[0].quality.as_deref(),Some("uhd"));
        assert_eq!(project_movie_of_the_night(&request(Some("GB")),&motn(),100).sources[0].status,SourceStatus::NoOffers);
    }
    #[test] fn unsafe_destination_retains_reported_availability_as_partial() {
        let mut body=motn();body["streamingOptions"]["us"][0]["link"]=json!("https://127.0.0.1/private");
        body["streamingOptions"]["us"][0]["videoLink"]=Value::Null;
        let result=project_movie_of_the_night(&request(Some("US")),&body,100);
        assert_eq!(result.sources[0].status,SourceStatus::Partial);
        assert_eq!(result.offers.len(),1);assert!(result.offers[0].destinations.is_empty());
    }
    #[test] fn expired_and_future_offers_are_not_current_access() {
        let mut body=motn();body["streamingOptions"]["us"][0]["expiresOn"]=json!(50);
        assert!(project_movie_of_the_night(&request(Some("US")),&body,100).offers.is_empty());
        body["streamingOptions"]["us"][0]["expiresOn"]=Value::Null;
        body["streamingOptions"]["us"][0]["availableSince"]=json!(150);
        assert!(project_movie_of_the_night(&request(Some("US")),&body,100).offers.is_empty());
    }
    #[test] fn conflicting_external_ids_fail_closed() {
        let mut req=request(None);req.identity.ids.push(CatalogId{source:"imdb".into(),id:"tt1234567".into()});
        assert_eq!(project_movie_of_the_night(&req,&motn(),100).sources[0].status,SourceStatus::IdentityMismatch);
    }
    #[test] fn service_and_country_registry_has_no_name_allowlist() {
        let body=json!({"nz":{"countryCode":"nz","name":"New Zealand","services":[{
            "id":"future-provider","name":"Future Provider","homePage":"https://future.example.com",
            "streamingOptionTypes":{"subscription":true,"rent":true,"free":false,"buy":true,"addon":true},
            "addons":[{"id":"local-channel","name":"Local Channel"}]}]}});
        let result=project_movie_of_the_night_registry(None,&body,100);
        assert_eq!(result.countries[0].code.as_str(),"NZ");
        assert_eq!(result.countries[0].services[0].service.id,"future-provider");
        assert_eq!(result.countries[0].services[0].access_types.len(),4);
        assert_eq!(result.countries[0].services[0].addons[0].id,"local-channel");
    }
    #[test] fn unknown_access_type_is_preserved() {
        let mut body=motn();body["streamingOptions"]["us"][0]["type"]=json!("library-card");
        let result=project_movie_of_the_night(&request(Some("US")),&body,100);
        assert_eq!(result.offers[0].access_type,AccessType::Other);
        assert_eq!(result.offers[0].raw_access_type.as_deref(),Some("library-card"));
    }
    #[test] fn malformed_country_is_rejected_at_deserialization() {
        assert!(serde_json::from_value::<LookupRequest>(json!({"identity":{"kind":"movie","ids":[]},"country":"USA"})).is_err());
    }
    #[test] fn destination_validation_accepts_new_public_domains_and_rejects_local_or_credentials() {
        assert!(safe_https_url("https://future.example.com/movie/238#player").is_some());
        for bad in ["http://future.example.com/movie/238","https://user:password@example.com/movie/238",
            "https://localhost/x","https://10.0.0.1/x","https://[::1]/x","https://[::ffff:192.168.1.1]/x",
            "https://metadata.internal/x","https://example.com:8443/x","javascript:alert(1)"] {
            assert!(safe_https_url(bad).is_none(),"{bad}");
        }
    }
    #[test] fn stable_offer_id_ignores_upstream_order_and_freshness() {
        let mut req=request(None);req.identity.ids.push(CatalogId{source:"imdb".into(),id:"tt0068646".into()});
        let first=project_movie_of_the_night(&req,&motn(),100);
        req.identity.ids.reverse();
        let later=project_movie_of_the_night(&req,&motn(),200);
        assert_eq!(first.offers[0].id,later.offers[0].id);
    }
    #[test] fn provider_home_search_and_catalog_pages_do_not_become_actions() {
        for link in ["https://new-service.example.com/", "https://new-service.example.com/search?q=title",
            "https://www.themoviedb.org/movie/238/watch", "https://www.justwatch.com/us/movie/example"] {
            let mut body = motn();
            body["streamingOptions"]["us"][0]["link"] = json!(link);
            body["streamingOptions"]["us"][0]["videoLink"] = Value::Null;
            let result = project_movie_of_the_night(&request(Some("US")), &body, 100);
            assert_eq!(result.sources[0].status, SourceStatus::Partial);
            assert!(result.offers[0].destinations.is_empty());
        }
    }

}
