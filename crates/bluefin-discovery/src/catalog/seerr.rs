//! Read-only adapter to an operator's existing Seerr service.
use super::{bounded_bytes, http_status, observation, timestamp};
use super::settings::{Settings, validate_service_url};
use crate::{Error, Result};
use bluefin_catalog::{
    AccessType, Attribution, CountryCode, CountryRecord, Coverage, CoverageScope,
    LookupRequest, MediaKind, OfferResponse, RegistryResponse, RegistryService,
    ServiceIdentity, SourceStatus,
};
use reqwest::{Client, header::HeaderValue};
use serde_json::{Map, Value, json};
use std::time::Duration;
use url::Url;

pub(super) struct Seerr {
    client: Client,
    origin: Url,
    key: HeaderValue,
}
impl Seerr {
    pub fn from_settings(settings: &Settings) -> Result<Option<Self>> {
        let Some(origin) = settings.seerr_url.as_deref() else { return Ok(None); };
        let origin = validate_service_url(origin, settings.allow_local_http)?;
        let mut key = HeaderValue::from_str(settings.seerr_api_key.as_deref().ok_or(Error::Invalid)?)
            .map_err(|_| Error::Invalid)?;
        key.set_sensitive(true);
        let client = Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(12)).connect_timeout(Duration::from_secs(3))
            .user_agent("Bluefin catalog/1").build().map_err(|_| Error::Upstream)?;
        Ok(Some(Self {client, origin, key}))
    }
    async fn get(&self, path: &[&str], country: Option<&CountryCode>) -> std::result::Result<Value, SourceStatus> {
        let mut url = self.origin.clone();
        url.path_segments_mut().map_err(|()| SourceStatus::InvalidResponse)?
            .pop_if_empty().extend(["api", "v1"]).extend(path.iter().copied());
        let mut request = self.client.get(url).header("X-Api-Key", self.key.clone());
        if let Some(country) = country {
            request = request.query(&[("watchRegion", country.as_str())]);
        }
        let response = request.send().await.map_err(|_| SourceStatus::Unavailable)?;
        if !response.status().is_success() { return Err(http_status(response.status())); }
        serde_json::from_slice(&bounded_bytes(response, 4 * 1024 * 1024).await?)
            .map_err(|_| SourceStatus::InvalidResponse)
    }
    pub async fn lookup(&self, request: &LookupRequest) -> OfferResponse {
        let Some((kind, id)) = tmdb_title(request) else {
            return OfferResponse::with_status(request, "seerr", SourceStatus::UnsupportedIdentity,
                CoverageScope::RegionalAvailability, timestamp());
        };
        let payload = match self.get(&[kind, &id], None).await {
            Ok(payload) => payload,
            Err(status) => return OfferResponse::with_status(request, "seerr", status,
                CoverageScope::RegionalAvailability, timestamp()),
        };
        if payload["id"].as_u64().map(|value| value.to_string()).as_deref() != Some(id.as_str()) {
            return OfferResponse::with_status(request, "seerr", SourceStatus::IdentityMismatch,
                CoverageScope::RegionalAvailability, timestamp());
        }
        if let Some(expected) = request.identity.id("imdb") {
            let actual = payload["externalIds"]["imdbId"].as_str().or_else(|| payload["imdbId"].as_str());
            if actual != Some(expected) {
                return OfferResponse::with_status(request, "seerr", SourceStatus::IdentityMismatch,
                    CoverageScope::RegionalAvailability, timestamp());
            }
        }
        project_detail(request, &payload, timestamp())
    }
    pub async fn registry(&self, country: Option<&CountryCode>) -> RegistryResponse {
        let now = timestamp();
        let regions = match self.get(&["watchproviders", "regions"], None).await {
            Ok(regions) => regions,
            Err(status) => return RegistryResponse {
                sources: vec![observation("seerr", status, Coverage::Unknown, now)],
                ..RegistryResponse::default()
            },
        };
        let entries = regions.as_array().or_else(|| regions["results"].as_array());
        let Some(entries) = entries.filter(|entries| entries.len() <= 300) else {
            return RegistryResponse {sources: vec![observation("seerr", SourceStatus::InvalidResponse,
                Coverage::Unknown, now)], ..RegistryResponse::default()};
        };
        let mut result = RegistryResponse::default();
        for entry in entries {
            let Some(code) = entry["iso_3166_1"].as_str().and_then(|code| CountryCode::parse(code).ok()) else { continue; };
            if country.is_some_and(|selected| selected != &code) { continue; }
            let name = entry["english_name"].as_str().or_else(|| entry["name"].as_str())
                .unwrap_or(code.as_str()).to_owned();
            result.countries.push(CountryRecord {code, name, services: Vec::new()});
        }
        let mut status = SourceStatus::Complete;
        if let Some(country) = country {
            let (movies, television) = tokio::join!(
                self.get(&["watchproviders", "movies"], Some(country)),
                self.get(&["watchproviders", "tv"], Some(country)));
            let mut services = Vec::new();
            for payload in [movies, television] {
                match payload {
                    Ok(payload) => {
                        let values = payload.as_array().or_else(|| payload["results"].as_array());
                        if let Some(values) = values.filter(|values| values.len() <= 5000) {
                            services.extend(values.iter().filter_map(registry_service));
                        } else { status = SourceStatus::Partial; }
                    }
                    Err(_) => status = SourceStatus::Partial,
                }
            }
            services.sort_by(|a,b| a.service.id.cmp(&b.service.id));
            services.dedup_by(|a,b| a.service.id == b.service.id);
            if let Some(region) = result.countries.iter_mut().find(|region| &region.code == country) {
                region.services = services;
            } else {
                status = SourceStatus::UnsupportedCountry;
            }
        }
        let mut source = observation("seerr", status, Coverage::SourceCatalog, now);
        source.attributions = attributions();
        result.sources.push(source);
        result
    }
}

pub(super) fn tmdb_title(request: &LookupRequest) -> Option<(&'static str, String)> {
    let kind = match request.identity.kind {
        MediaKind::Movie => "movie",
        MediaKind::Series => "tv",
        _ => return None,
    };
    let raw = request.identity.id("tmdb")?;
    let id = raw.strip_prefix(&format!("{kind}/")).unwrap_or(raw);
    if id.is_empty() || id.len() > 12 || !id.bytes().all(|byte| byte.is_ascii_digit())
        || id.parse::<u64>().ok()? == 0 {
        return None;
    }
    Some((kind, id.to_owned()))
}
fn provider(value: &Value) -> Option<Value> {
    let id = value["id"].as_u64()?;
    let name = value["name"].as_str().filter(|name| !name.is_empty() && name.len() <= 512)?;
    Some(json!({"provider_id":id, "provider_name":name,
        "logo_path":value["logoPath"], "display_priority":value["displayPriority"]}))
}
fn registry_service(value: &Value) -> Option<RegistryService> {
    let value = provider(value)?;
    let id = value["provider_id"].as_u64()?.to_string();
    let name = value["provider_name"].as_str()?.to_owned();
    let logo_url = value["logo_path"].as_str().filter(|path| path.starts_with('/')
        && !path.contains("..") && path.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b,b'/'|b'.'|b'_'|b'-')))
        .map(|path| format!("https://image.tmdb.org/t/p/w185{path}"));
    Some(RegistryService {
        service: ServiceIdentity {source:"tmdb".into(), id, name, home_url:None, logo_url},
        // The registry identifies services; it does not establish which offer types
        // or titles are available. Actual title responses provide those facts.
        access_types: Vec::<AccessType>::new(), addons:Vec::new(),
    })
}
fn attributions() -> Vec<Attribution> {
    vec![Attribution {name:"TMDB".into(),url:"https://www.themoviedb.org/".into()},
        Attribution {name:"JustWatch".into(),url:"https://www.justwatch.com/".into()}]
}
fn project_detail(request: &LookupRequest, payload: &Value, now: u64) -> OfferResponse {
    let Some(regions) = payload["watchProviders"].as_array().filter(|regions| regions.len() <= 300) else {
        return OfferResponse::with_status(request, "seerr", SourceStatus::InvalidResponse,
            CoverageScope::RegionalAvailability, now);
    };
    let mut results = Map::new();
    for region in regions {
        let Some(code) = region["iso_3166_1"].as_str().and_then(|code| CountryCode::parse(code).ok()) else {continue;};
        let mut projected = Map::new();
        projected.insert("link".into(), region["link"].clone());
        for category in ["flatrate","buy","rent","free","ads"] {
            if let Some(values) = region[category].as_array() {
                projected.insert(category.into(), Value::Array(values.iter().take(5000).filter_map(provider).collect()));
            }
        }
        results.insert(code.to_string(), Value::Object(projected));
    }
    let mut response = bluefin_catalog::video::project_tmdb_watch_providers(
        request, &json!({"id":payload["id"],"results":results}), now);
    // Seerr versions may omit rental/free/ad fields when mapping the original API.
    // Therefore this adapter never claims exhaustive source coverage or absence.
    for source in &mut response.sources {
        source.source = "seerr".into();
        source.status = SourceStatus::Partial;
        source.coverage = Coverage::Partial;
        source.attributions = attributions();
    }
    for offer in &mut response.offers { offer.evidence.source = "seerr".into(); }
    response
}
#[cfg(test)]
mod tests {
    use super::*;
    use bluefin_catalog::{CatalogIdentity,CatalogId};
    fn request(country: Option<&str>) -> LookupRequest {
        LookupRequest {identity:CatalogIdentity {kind:MediaKind::Movie,
            ids:vec![CatalogId{source:"tmdb".into(),id:"movie/278".into()}]},
            country:country.map(|value| CountryCode::parse(value).unwrap())}
    }
    #[test]
    fn summary_preserves_regions_and_new_services_without_fabricating_actions() {
        let payload = json!({"id":278,"watchProviders":[
            {"iso_3166_1":"US","link":"https://www.themoviedb.org/movie/278/watch?locale=US",
             "flatrate":[{"id":999999,"name":"A newly added service","logoPath":"/new.png"}]},
            {"iso_3166_1":"GB","buy":[{"id":10,"name":"Amazon Video"}]}]});
        let all = project_detail(&request(None),&payload,17);
        assert_eq!(all.offers.len(),2);
        assert!(all.offers.iter().all(|offer| offer.destinations.is_empty()));
        assert_eq!(all.sources[0].status, SourceStatus::Partial);
        let selected = project_detail(&request(Some("GB")),&payload,17);
        assert_eq!(selected.offers.len(),1);
        assert_eq!(selected.offers[0].access_type,AccessType::Buy);
    }
    #[test]
    fn media_kind_and_external_identity_cannot_be_swapped() {
        let mut request=request(None);
        request.identity.ids[0].id="tv/278".into();
        assert!(tmdb_title(&request).is_none());
        request.identity.ids[0].id="278".into();
        assert_eq!(tmdb_title(&request),Some(("movie","278".into())));
    }
}
