//! Public source-backed access offers. No library session or pairing is required.
use crate::{error::ApiError, extract::Query, state::AppState};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    routing::get,
};
use bluefin_discovery::catalog_types::{
    CatalogId, CatalogIdentity, CountryCode, LookupRequest, MediaKind, OfferResponse,
    RegistryResponse,
};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OffersQuery {
    kind: MediaKind,
    source: String,
    id: String,
    country: Option<CountryCode>,
}
impl crate::extract::QueryParameters for OffersQuery {
    const COLLECTIONS: &'static [(&'static str, char)] = &[];
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProvidersQuery {
    country: Option<CountryCode>,
    kind: Option<MediaKind>,
    source: Option<String>,
    id: Option<String>,
}
impl crate::extract::QueryParameters for ProvidersQuery {
    const COLLECTIONS: &'static [(&'static str, char)] = &[];
}
fn unavailable() -> ApiError {
    ApiError::ServiceUnavailable("Public catalog is unavailable".into())
}
fn error(error: bluefin_discovery::Error) -> ApiError {
    match error {
        bluefin_discovery::Error::Invalid => {
            ApiError::BadRequest("Invalid catalog identity or country".into())
        }
        _ => unavailable(),
    }
}
async fn offers(
    State(state): State<AppState>,
    Query(query): Query<OffersQuery>,
) -> Result<Json<OfferResponse>, ApiError> {
    let request = LookupRequest {
        identity: CatalogIdentity {
            kind: query.kind,
            ids: vec![CatalogId {
                source: query.source,
                id: query.id,
            }],
        },
        country: query.country,
    };
    lookup(&state, request).await
}
async fn offers_post(
    State(state): State<AppState>,
    Json(request): Json<LookupRequest>,
) -> Result<Json<OfferResponse>, ApiError> {
    lookup(&state, request).await
}
async fn lookup(state: &AppState, request: LookupRequest) -> Result<Json<OfferResponse>, ApiError> {
    let discovery = state.discovery.as_ref().ok_or_else(unavailable)?;
    Ok(Json(
        discovery.catalog().lookup(request).await.map_err(error)?,
    ))
}
async fn providers(
    State(state): State<AppState>,
    Query(query): Query<ProvidersQuery>,
) -> Result<Json<RegistryResponse>, ApiError> {
    let discovery = state.discovery.as_ref().ok_or_else(unavailable)?;
    let identity = match (query.kind, query.source, query.id) {
        (None, None, None) => None,
        (Some(kind), Some(source), Some(id)) => Some(CatalogIdentity {
            kind,
            ids: vec![CatalogId { source, id }],
        }),
        _ => {
            return Err(ApiError::BadRequest(
                "Catalog context requires kind, source and id".into(),
            ));
        }
    };
    Ok(Json(
        discovery
            .catalog()
            .providers_for(query.country, identity)
            .await
            .map_err(error)?,
    ))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataSearch {
    kind: bluefin_contract::public_catalog::CatalogKind,
    query: String,
    page: Option<u32>,
}
impl crate::extract::QueryParameters for MetadataSearch {
    const COLLECTIONS: &'static [(&'static str, char)] = &[];
}
async fn metadata_search(
    State(state): State<AppState>,
    Query(query): Query<MetadataSearch>,
) -> Result<axum::response::Response, ApiError> {
    let discovery = state.discovery.as_ref().ok_or_else(unavailable)?;
    Ok(metadata_response(
        discovery
            .catalog()
            .metadata_search(query.kind, &query.query, query.page.unwrap_or(1))
            .await,
    ))
}
async fn metadata_details(
    State(state): State<AppState>,
    axum::extract::Path((kind, id)): axum::extract::Path<(
        bluefin_contract::public_catalog::CatalogKind,
        String,
    )>,
) -> Result<axum::response::Response, ApiError> {
    let discovery = state.discovery.as_ref().ok_or_else(unavailable)?;
    Ok(metadata_response(
        discovery.catalog().metadata_details(kind, &id).await,
    ))
}
fn metadata_response<T: serde::Serialize>(
    result: Result<T, bluefin_contract::public_catalog::CatalogFailure>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use bluefin_discovery::catalog_types::SourceStatus;
    match result {
        Ok(value) => Json(value).into_response(),
        Err(failure) => {
            let status = match failure.status() {
                SourceStatus::UnsupportedIdentity | SourceStatus::UnsupportedCountry => {
                    StatusCode::BAD_REQUEST
                }
                SourceStatus::NotFound => StatusCode::NOT_FOUND,
                SourceStatus::RateLimited => StatusCode::TOO_MANY_REQUESTS,
                SourceStatus::IdentityMismatch | SourceStatus::InvalidResponse => {
                    StatusCode::BAD_GATEWAY
                }
                _ => StatusCode::SERVICE_UNAVAILABLE,
            };
            (status, Json(failure)).into_response()
        }
    }
}

/// Adds public read-only metadata, access offers and dynamic regional provider discovery.
pub fn register(router: Router<AppState>) -> Router<AppState> {
    router
        .route("/Bluefin/Catalog/Search", get(metadata_search))
        .route(
            "/Bluefin/Catalog/Details/{kind}/{id}",
            get(metadata_details),
        )
        .route(
            "/Bluefin/Catalog/Offers",
            get(offers)
                .post(offers_post)
                .layer(DefaultBodyLimit::max(16_384)),
        )
        .route("/Bluefin/Catalog/Providers", get(providers))
}
