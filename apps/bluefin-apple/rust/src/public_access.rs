//! Public offers execute on this device before account routing.
use bluefin_catalog::{
    CountryCode, CoverageScope, LookupRequest, MediaKind, OfferResponse, SourceStatus,
    WatchPageClient, music_http::MusicBrainzClient,
};
use serde_json::Value;
use std::sync::OnceLock;
use super::{err, Result};

pub(super) fn music_client() -> Result<&'static MusicBrainzClient> {
    static CLIENT: OnceLock<Result<MusicBrainzClient>> = OnceLock::new();
    CLIENT.get_or_init(|| MusicBrainzClient::new().map_err(|_|err("Music catalog is unavailable")))
        .as_ref().map_err(Clone::clone)
}
fn watch_client() -> Result<&'static WatchPageClient> {
    static CLIENT: OnceLock<Result<WatchPageClient>> = OnceLock::new();
    CLIENT.get_or_init(|| WatchPageClient::new().map_err(|_|err("Video catalog is unavailable")))
        .as_ref().map_err(Clone::clone)
}
pub(super) fn request(input: &Value) -> Result<LookupRequest> {
    let identity = serde_json::from_value(input["identity"].clone())
        .map_err(|_|err("Invalid catalog identity"))?;
    let country = match input.get("country") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(CountryCode::parse(value)
            .map_err(|_|err("Invalid catalog country"))?),
        _ => return Err(err("Invalid catalog country")),
    };
    let request = LookupRequest {identity,country};
    if request.identity.ids.is_empty() || request.identity.ids.len() > 12 {
        return Err(err("Invalid catalog identity"));
    }
    let mut ids = std::collections::HashMap::new();
    for id in &request.identity.ids {
        if id.source.is_empty() || id.source.len()>64 || id.id.is_empty() || id.id.len()>256
            || id.id.chars().any(|ch|ch.is_control() || ch.is_whitespace())
            || !id.source.bytes().all(|b|b.is_ascii_alphanumeric() || b==b'-' || b==b'_')
            || ids.insert(id.source.to_ascii_lowercase(),&id.id)
                .is_some_and(|previous|previous != &id.id) {
            return Err(err("Invalid or conflicting catalog identity"));
        }
    }
    Ok(request)
}
pub(super) fn offers(input: &Value) -> Result<Value> {
    let request = request(input)?;
    let runtime = super::mesh_connection::runtime()?;
    let response = match request.identity.kind {
        MediaKind::Movie | MediaKind::Series | MediaKind::Season | MediaKind::Episode =>
            runtime.block_on(watch_client()?.lookup(&request)),
        MediaKind::Artist | MediaKind::Release | MediaKind::ReleaseGroup |
        MediaKind::Recording | MediaKind::MusicVideo | MediaKind::Work =>
            runtime.block_on(music_client()?.lookup(&request)),
        _ => OfferResponse::with_status(&request,"public-catalog",SourceStatus::UnsupportedIdentity,
            CoverageScope::LinkedAccess,0),
    };
    serde_json::to_value(response).map_err(|_|err("Catalog offers could not be decoded"))
}

pub(super) fn countries(input: &Value) -> Result<Value> {
    let mut request=request(input)?;
    request.country=None;
    let response=super::mesh_connection::runtime()?.block_on(watch_client()?.countries(&request));
    serde_json::to_value(response).map_err(|_|err("Catalog countries could not be decoded"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn public_requests_keep_region_and_entity_types_and_reject_ambiguous_ids() {
        let identity=json!({"kind":"recording","ids":[{"source":"musicbrainz","id":"a74b1b7f-71a5-4011-9441-d0b5e4122711"}]});
        let unspecified=request(&json!({"identity":identity})).unwrap();
        assert_eq!(unspecified.country,None);
        assert_eq!(unspecified.identity.kind,MediaKind::Recording);
        let regional=request(&json!({"identity":identity,"country":"gb"})).unwrap();
        assert_eq!(regional.country.unwrap().as_str(),"GB");
        assert!(request(&json!({"identity":identity,"country":"GB/US"})).is_err());
        assert!(request(&json!({"identity":{"kind":"movie","ids":[
            {"source":"tmdb","id":"278"},{"source":"TMDB","id":"603"}]}})).is_err());
    }
    #[test]
    fn unsupported_public_kind_does_not_require_a_private_account() {
        let response=offers(&json!({"identity":{"kind":"sportsEvent","ids":[
            {"source":"wikidata","id":"Q1"}]}})).unwrap();
        assert_eq!(response["sources"][0]["status"],"unsupportedIdentity");
        assert!(response["offers"].as_array().unwrap().is_empty());
    }
}
