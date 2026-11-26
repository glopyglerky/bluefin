//! Home recommendations and explicit search signals on the existing account DB.
//! No device-local identity, cloud history, provider call, or cross-server grant.
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    routing::{get, post},
};
use chrono::Utc;
use ferrofin_db::entities::base_items::BaseItemEntity;
use ferrofin_model::{
    data::BaseItemKind,
    dto::{BaseItemDto, RecommendationType},
};
use ferrofin_traits::options::{AuthorizationInfo, DtoOptions, InternalItemsQuery};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::Arc};
#[path = "bluefin_home_repository.rs"]
mod repository;
use crate::{auth::RequireAuth, error::ApiError, extract::Query, state::AppState};
pub(crate) use repository::History;
use repository::{HistoryState, SearchEvent};
use uuid::Uuid;

const MAX_SHELVES: usize = 4;
const MAX_ITEMS: usize = 12;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoUserSelection {}
impl crate::extract::QueryParameters for NoUserSelection {
    const COLLECTIONS: &'static [(&'static str, char)] = &[];
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClearHistory {
    generation: i64,
}

fn viewer(auth: &AuthorizationInfo) -> Result<Uuid, ApiError> {
    if auth.is_api_key || !auth.is_authenticated || auth.user_id().is_nil() || auth.user.is_none() {
        return Err(ApiError::Forbidden(
            "a current user session is required".into(),
        ));
    }
    Ok(auth.user_id())
}
fn history(state: &AppState) -> Result<&History, ApiError> {
    state
        .home_history
        .as_deref()
        .ok_or_else(|| ApiError::NotFound("account search history unavailable".into()))
}
async fn record_search(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Query(_): Query<NoUserSelection>,
    Json(mut event): Json<SearchEvent>,
) -> Result<Json<HistoryState>, ApiError> {
    let user = viewer(&auth)?;
    event.validate(Utc::now().timestamp_millis())?;
    if let Some(id) = event.selected_item_id {
        if state
            .library
            .get_item_by_id_for_user(id, auth.user.as_deref())
            .await?
            .is_none()
        {
            return Err(ApiError::NotFound("selected item unavailable".into()));
        }
    }
    Ok(Json(
        history(&state)?
            .record(user, event, Utc::now().timestamp_millis())
            .await?,
    ))
}
async fn clear_searches(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Query(_): Query<NoUserSelection>,
    Json(request): Json<ClearHistory>,
) -> Result<Json<HistoryState>, ApiError> {
    let user = viewer(&auth)?;
    Ok(Json(
        history(&state)?.clear(user, request.generation).await?,
    ))
}

#[derive(Serialize)]
struct HomeShelf {
    id: String,
    title: String,
    reason: &'static str,
    items: Vec<BaseItemDto>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HomeRecommendations {
    recommendations: Vec<HomeShelf>,
    search_history_supported: bool,
    search_history_generation: Option<i64>,
}
struct ShelfRows {
    id: String,
    title: String,
    reason: &'static str,
    items: Vec<BaseItemEntity>,
}

fn watched_label(kind: RecommendationType, baseline: &str) -> (String, &'static str) {
    let (prefix, reason) = match kind {
        RecommendationType::SimilarToRecentlyPlayed => ("Because you watched", "watched"),
        RecommendationType::SimilarToLikedItem => ("Because you like", "liked"),
        RecommendationType::HasDirectorFromRecentlyPlayed => ("More from", "watched"),
        RecommendationType::HasActorFromRecentlyPlayed => ("More with", "watched"),
        RecommendationType::HasLikedDirector => ("From a director you like:", "liked"),
        RecommendationType::HasLikedActor => ("With an actor you like:", "liked"),
    };
    (
        format!(
            "{prefix} {}",
            baseline.chars().take(128).collect::<String>()
        ),
        reason,
    )
}
fn bound_shelves(rows: Vec<ShelfRows>) -> Vec<ShelfRows> {
    let mut seen = HashSet::new();
    rows.into_iter()
        .take(MAX_SHELVES)
        .filter_map(|mut shelf| {
            shelf.items = shelf
                .items
                .into_iter()
                .filter(|item| Uuid::parse_str(&item.id).is_ok_and(|id| seen.insert(id)))
                .take(MAX_ITEMS)
                .collect();
            (!shelf.items.is_empty()).then_some(shelf)
        })
        .collect()
}

async fn recommendations(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Query(_): Query<NoUserSelection>,
) -> Result<Json<HomeRecommendations>, ApiError> {
    let user_id = viewer(&auth)?;
    let user = auth
        .user
        .as_ref()
        .ok_or_else(|| ApiError::Forbidden("user required".into()))?;
    let options = DtoOptions {
        fields: super::query_parse::parse_csv_enums_lenient(Some(
            "PrimaryImageAspectRatio,Overview",
        )),
        ..DtoOptions::default()
    };
    let (generation, events) = if let Some(history) = &state.home_history {
        let (generation, events) = history
            .snapshot(user_id, Utc::now().timestamp_millis())
            .await?;
        (Some(generation), events)
    } else {
        (None, Vec::new())
    };
    let movie = state
        .similar_items
        .get_movie_recommendations(
            Some(user_id),
            Uuid::nil(),
            MAX_SHELVES as i32,
            MAX_ITEMS as i32,
            &options,
        )
        .await?;
    let mut movie: Vec<_> = movie
        .into_iter()
        .map(|row| {
            let (title, reason) = watched_label(row.recommendation_type, &row.baseline_item_name);
            ShelfRows {
                id: format!("{reason}:{}", row.category_id),
                title,
                reason,
                items: row.items,
            }
        })
        .collect();
    // Include both watched and liked evidence before adding more shelves from
    // one signal. This keeps searches from crowding every liked category out.
    if let Some(index) = movie.iter().position(|row| row.reason == "liked") {
        let liked = movie.remove(index);
        movie.insert(movie.len().min(1), liked);
    }
    let mut searches = Vec::new();
    let mut terms = HashSet::new();
    // Every source is the current user's own event. A revoked/deleted selected
    // item cannot remain a baseline, and each catalogue query reapplies policy.
    for event in events {
        if searches.len() == 2 || terms.len() == 4 {
            break;
        }
        if !terms.insert(event.term.to_lowercase()) {
            continue;
        }
        let selected = event
            .selected_item_id
            .as_deref()
            .and_then(|id| Uuid::parse_str(id).ok());
        if let Some(id) = selected {
            if state
                .library
                .get_item_by_id_for_user(id, Some(user))
                .await?
                .is_none()
            {
                continue;
            }
        }
        let mut query = InternalItemsQuery {
            recursive: true,
            limit: Some(MAX_ITEMS as i32),
            search_term: Some(event.term.clone()),
            include_item_types: vec![
                BaseItemKind::Movie,
                BaseItemKind::Series,
                BaseItemKind::MusicAlbum,
                BaseItemKind::MusicArtist,
                BaseItemKind::Audio,
                BaseItemKind::TvChannel,
            ],
            is_played: Some(false),
            exclude_item_ids: selected.into_iter().collect(),
            ..Default::default()
        };
        query.set_user(Arc::clone(user));
        let items = state.library.get_item_list(&query).await?;
        if !items.is_empty() {
            searches.push(ShelfRows {
                id: format!("search:{}", event.event_id),
                title: format!("From your search: {}", event.term),
                reason: "search",
                items,
            });
        }
    }
    let reserve = searches.len();
    let remainder = movie.split_off(movie.len().min(MAX_SHELVES - reserve));
    movie.extend(searches);
    movie.extend(remainder);
    let rows = bound_shelves(movie);
    // A single batched DTO projection keeps image/user-data lookups bounded.
    let all: Vec<_> = rows
        .iter()
        .flat_map(|row| row.items.iter().cloned())
        .collect();
    let mut dtos = state
        .dto
        .get_base_item_dtos(&all, &options, Some(user), None, true)
        .await?
        .into_iter();
    let recommendations = rows
        .into_iter()
        .map(|row| HomeShelf {
            id: row.id,
            title: row.title,
            reason: row.reason,
            items: dtos.by_ref().take(row.items.len()).collect(),
        })
        .collect();
    Ok(Json(HomeRecommendations {
        recommendations,
        search_history_supported: generation.is_some(),
        search_history_generation: generation,
    }))
}

pub(crate) fn register(router: Router<AppState>) -> Router<AppState> {
    router.merge(
        Router::new()
            .route("/Bluefin/Home/Recommendations", get(recommendations))
            .route(
                "/Bluefin/Home/Searches",
                post(record_search).delete(clear_searches),
            )
            .layer(DefaultBodyLimit::max(4096))
            .layer(axum::middleware::map_response(
                |mut response: axum::response::Response| async move {
                    response.headers_mut().insert(
                        axum::http::header::CACHE_CONTROL,
                        axum::http::HeaderValue::from_static("no-store"),
                    );
                    response
                },
            )),
    )
}

#[cfg(test)]
#[path = "bluefin_home_tests.rs"]
mod tests;
