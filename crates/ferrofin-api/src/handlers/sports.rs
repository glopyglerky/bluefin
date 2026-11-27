//! Account-scoped Sports presentation, using existing persistence and authority.
use crate::{
    auth::{RequireAuth, RequireLiveTvAccess},
    error::ApiError,
    extract::Query,
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use bluefin_contract::sports::{
    Home, LocalBroadcast, MAX_BODY_BYTES, MAX_SELECTIONS, Preferences, Saved,
};
use chrono::{DateTime, Utc};
use ferrofin_traits::{
    error::ServiceError,
    options::{AuthorizationInfo, DtoOptions, InternalItemsQuery},
};
use std::collections::HashMap;
use uuid::Uuid;

#[path = "sports_feed.rs"]
mod feed;

pub(crate) const CLIENT: &str = "bluefin-sports-v1";
const ITEM: Uuid = Uuid::from_u128(0x626c_7565_6669_6e73_706f_7274_7300_0001);
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NoSelection {}
impl crate::extract::QueryParameters for NoSelection {
    const COLLECTIONS: &'static [(&'static str, char)] = &[];
}
fn viewer(auth: &AuthorizationInfo) -> Result<Uuid, ApiError> {
    if auth.is_api_key || !auth.is_authenticated || auth.user_id().is_nil() || auth.user.is_none() {
        return Err(ApiError::Forbidden(
            "a current user session is required".into(),
        ));
    }
    Ok(auth.user_id())
}
fn valid(preferences: &Preferences) -> bool {
    preferences.is_valid()
        && serde_json::to_vec(preferences).is_ok_and(|bytes| bytes.len() <= MAX_BODY_BYTES)
}
fn valid_counts(value: &serde_json::Value) -> bool {
    ["sports", "leagues", "conferences", "teams", "players"]
        .iter()
        .all(|field| {
            value
                .get(field)
                .and_then(serde_json::Value::as_array)
                .is_some_and(|ids| ids.len() <= MAX_SELECTIONS)
        })
}
fn corrupt() -> ApiError {
    ServiceError::backend("invalid stored Sports preferences").into()
}
fn decode(raw: &HashMap<String, Option<String>>) -> Result<Saved, ApiError> {
    if raw.is_empty() {
        return Ok(Saved::default());
    }
    let Some(Some(value)) = raw.get("record") else {
        return Err(corrupt());
    };
    if raw.len() != 1 || value.len() > MAX_BODY_BYTES + 256 {
        return Err(corrupt());
    }
    let wire: serde_json::Value = serde_json::from_str(value).map_err(|_| corrupt())?;
    if !wire.get("preferences").is_some_and(valid_counts) {
        return Err(corrupt());
    }
    let saved: Saved = serde_json::from_str(value).map_err(|_| corrupt())?;
    if !valid(&saved.preferences) {
        return Err(corrupt());
    }
    Ok(saved)
}
async fn load(state: &AppState, user: Uuid) -> Result<HashMap<String, Option<String>>, ApiError> {
    Ok(state
        .display_preferences
        .list_custom_item_display_preferences(user, ITEM, CLIENT)
        .await?)
}
fn saved_response(saved: Saved) -> Response {
    (
        [
            (header::ETAG, format!("\"{}\"", saved.version)),
            (header::CACHE_CONTROL, "no-store".to_owned()),
        ],
        Json(saved),
    )
        .into_response()
}
fn revision(headers: &axum::http::HeaderMap) -> Result<u64, StatusCode> {
    let mut values = headers.get_all(header::IF_MATCH).iter();
    let value = values.next().ok_or(StatusCode::PRECONDITION_REQUIRED)?;
    if values.next().is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let text = value.to_str().map_err(|_| StatusCode::BAD_REQUEST)?;
    let digits = text
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or(StatusCode::BAD_REQUEST)?;
    let number: u64 = digits.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    if number.to_string() != digits {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(number)
}
async fn preferences(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Query(_): Query<NoSelection>,
) -> Result<Response, ApiError> {
    let user = viewer(&auth)?;
    Ok(saved_response(decode(&load(&state, user).await?)?))
}
async fn save(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Query(_): Query<NoSelection>,
    request: Request,
) -> Result<Response, ApiError> {
    // Actual viewer and normal auth policy are checked before reading a byte.
    let user = viewer(&auth)?;
    let expected = match revision(request.headers()) {
        Ok(n) => n,
        Err(status) => return Ok(status.into_response()),
    };
    let Ok(bytes) = axum::body::to_bytes(request.into_body(), MAX_BODY_BYTES).await else {
        return Ok(StatusCode::PAYLOAD_TOO_LARGE.into_response());
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::BadRequest("invalid preferences JSON".into()))?;
    // Count the wire entries before set deserialization removes duplicates.
    if !valid_counts(&value) {
        return Err(ApiError::BadRequest(
            "invalid selection arrays or counts".into(),
        ));
    }
    let preferences: Preferences = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::BadRequest("invalid preferences fields".into()))?;
    if !valid(&preferences) {
        return Err(ApiError::BadRequest(
            "invalid preferences limits or schema".into(),
        ));
    }
    // Keep ownership through commit after HTTP cancellation. No timers/actions.
    tokio::spawn(async move {
        let raw = load(&state, user).await?;
        let current = decode(&raw)?;
        if expected != current.version {
            return Ok(StatusCode::PRECONDITION_FAILED.into_response());
        }
        let next = Saved {
            version: current
                .version
                .checked_add(1)
                .ok_or_else(|| ServiceError::backend("Sports revision exhausted"))?,
            preferences,
        };
        let replacement = HashMap::from([(
            "record".into(),
            Some(serde_json::to_string(&next).map_err(|_| corrupt())?),
        )]);
        if !state
            .display_preferences
            .compare_exchange_custom_item_display_preferences(
                user,
                ITEM,
                CLIENT,
                &raw,
                &replacement,
            )
            .await?
        {
            return Ok(StatusCode::PRECONDITION_FAILED.into_response());
        }
        Ok(saved_response(next))
    })
    .await
    .map_err(|_| ServiceError::backend("Sports write task failed"))?
}
fn listing_kind(
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> &'static str {
    match (start, end) {
        (Some(start), Some(end)) if end > start && start <= now && now < end => "ON_NOW",
        (Some(start), Some(end)) if end > start && start > now => "UPCOMING",
        _ => "UNKNOWN",
    }
}
async fn home(
    State(state): State<AppState>,
    RequireLiveTvAccess(auth): RequireLiveTvAccess,
    Query(_): Query<NoSelection>,
) -> Result<Response, ApiError> {
    let user = viewer(&auth)?;
    let preferences = decode(&load(&state, user).await?)?.preferences;
    let now = Utc::now();
    let mut feed = Home {
        schema: 1,
        sports_only: preferences.sports_only,
        games: vec![],
        local_broadcasts: vec![],
        presentation_only: true,
        providers: vec![],
    };
    if let Some(manager) = &state.live_tv {
        // One page only: bounded coverage, no count query. Opaque favorites
        // cannot be matched to verified events using guide metadata alone.
        let query = InternalItemsQuery {
            user: auth.user.clone(),
            is_sports: Some(true),
            min_start_date: Some(now - chrono::Duration::hours(24)),
            max_start_date: Some(now + chrono::Duration::days(7)),
            start_index: Some(0),
            limit: Some(256),
            enable_total_record_count: false,
            order_by: vec![(
                ferrofin_model::live_tv::ItemSortBy::StartDate,
                ferrofin_model::dto::SortOrder::Ascending,
            )],
            ..Default::default()
        };
        for program in manager
            .get_programs(&query, &DtoOptions::default())
            .await?
            .items
        {
            if program.is_sports != Some(true) || program.id.is_nil() {
                continue;
            }
            let Some(channel) = program.channel_id.filter(|id| !id.is_nil()) else {
                continue;
            };
            if state
                .library
                .get_item_by_id_for_user(program.id, auth.user.as_deref())
                .await?
                .is_none()
                || state
                    .library
                    .get_item_by_id_for_user(channel, auth.user.as_deref())
                    .await?
                    .is_none()
            {
                continue;
            }
            if program.end_date.is_some_and(|end| end <= now) {
                continue;
            }
            feed.local_broadcasts.push(LocalBroadcast {
                kind: listing_kind(program.start_date, program.end_date, now),
                item_id: program.id.to_string(),
                channel_id: channel.to_string(),
                channel_name: program.channel_name,
                native_title: program.name,
                native_start: program.start_date.map(|date| date.to_rfc3339()),
                native_end: program.end_date.map(|date| date.to_rfc3339()),
                provenance: "local_guide",
                playback_verified: false,
            });
        }
    }
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(feed)).into_response())
}
/// Mount Sports alongside the normal authenticated API.
pub fn register(router: Router<AppState>) -> Router<AppState> {
    feed::register(
        router
            .route("/sports/api/v1/preferences", get(preferences).put(save))
            .route("/sports/api/v1/home", get(home)),
    )
}
