use crate::{ApiError, AppState, extract::Query};
use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use bluefin_contract::public_catalog::{CatalogKind, CatalogOffers, CatalogPage, CatalogRecord};

/// A public upstream reader. Implementations carry no library or account authority.
#[async_trait]
pub trait PublicCatalog: Send + Sync {
    async fn search(&self, kind: CatalogKind, query: &str, page: u32) -> Option<CatalogPage>;
    async fn details(&self, kind: CatalogKind, id: &str) -> Option<CatalogRecord>;
    async fn offers(&self, kind: CatalogKind, id: u64, country: &str) -> Option<CatalogOffers>;
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Region {
    country: String,
}
impl crate::extract::QueryParameters for Region {
    const COLLECTIONS: &'static [(&'static str, char)] = &[];
}

async fn offers(
    State(state): State<AppState>,
    Path((kind, id)): Path<(CatalogKind, u64)>,
    Query(region): Query<Region>,
) -> Result<Json<CatalogOffers>, ApiError> {
    let country = region.country.to_ascii_lowercase();
    if !matches!(kind, CatalogKind::Movie | CatalogKind::Series)
        || id == 0
        || country.len() != 2
        || !country.bytes().all(|value| value.is_ascii_lowercase())
    {
        return Err(ApiError::BadRequest("Invalid title or country".into()));
    }
    let catalog = state.public_catalog.as_ref().ok_or_else(unavailable)?;
    catalog
        .offers(kind, id, &country)
        .await
        .map(Json)
        .ok_or_else(unavailable)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    kind: CatalogKind,
    query: String,
    page: Option<u32>,
}

impl crate::extract::QueryParameters for Search {
    const COLLECTIONS: &'static [(&'static str, char)] = &[];
}

fn unavailable() -> ApiError {
    ApiError::ServiceUnavailable("The public catalog provider is unavailable".into())
}

async fn search(
    State(state): State<AppState>,
    Query(input): Query<Search>,
) -> Result<Json<CatalogPage>, ApiError> {
    let page = input.page.unwrap_or(1);
    let query = input.query.trim();
    if !(2..=160).contains(&query.chars().count())
        || query.chars().any(char::is_control)
        || !(1..=100).contains(&page)
    {
        return Err(ApiError::BadRequest("Invalid public catalog search".into()));
    }
    let catalog = state.public_catalog.as_ref().ok_or_else(unavailable)?;
    catalog
        .search(input.kind, query, page)
        .await
        .map(Json)
        .ok_or_else(unavailable)
}

async fn details(
    State(state): State<AppState>,
    Path((kind, id)): Path<(CatalogKind, String)>,
) -> Result<Json<CatalogRecord>, ApiError> {
    if id.is_empty()
        || id.len() > 36
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        return Err(ApiError::BadRequest(
            "Invalid public catalog identity".into(),
        ));
    }
    let catalog = state.public_catalog.as_ref().ok_or_else(unavailable)?;
    catalog
        .details(kind, &id)
        .await
        .map(Json)
        .ok_or_else(unavailable)
}

pub(crate) fn register(router: Router<AppState>) -> Router<AppState> {
    router
        .route("/Bluefin/Catalog/Search", get(search))
        .route("/Bluefin/Catalog/Details/{kind}/{id}", get(details))
        .route("/Bluefin/Catalog/Offers/{kind}/{id}", get(offers))
}
