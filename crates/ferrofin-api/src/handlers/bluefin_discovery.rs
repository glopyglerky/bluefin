//! Generic discovery routes plus server/web-only administrator controls.
use crate::{
    auth::{RequireAdmin, RequireAuth},
    error::ApiError,
    extract::Query,
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    routing::{get, post},
};
use bluefin_contract::discovery::{ActionForm, ActionReceipt, DiscoveryPage, SubmitAction};
use bluefin_contract::search::SearchScope;
use bluefin_discovery::{
    Audience, DiscoveryService, Error, Viewer,
    config::{SaveSettings, SettingsSnapshot},
};
use ferrofin_traits::options::AuthorizationInfo;
use std::sync::Arc;
use uuid::Uuid;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Search {
    query: String,
    page: Option<u32>,
    #[serde(default)]
    scope: SearchScope,
}
impl crate::extract::QueryParameters for Search {
    const COLLECTIONS: &'static [(&'static str, char)] = &[];
}

pub(super) fn viewer(auth: &AuthorizationInfo) -> Result<Viewer, ApiError> {
    if auth.is_api_key || !auth.is_authenticated || auth.user_id().is_nil() {
        return Err(ApiError::Forbidden(
            "a current user session is required".into(),
        ));
    }
    let policy = auth
        .user_policy
        .as_ref()
        .ok_or_else(|| ApiError::Forbidden("user policy required".into()))?;
    let viewer = Viewer::new(auth.user_id(), policy.is_administrator);
    if auth
        .device_id
        .as_deref()
        .is_some_and(|id| id.starts_with("bluefin-mesh:"))
    {
        return Ok(viewer);
    }
    Ok(match &auth.token {
        Some(token) => viewer.with_session_token(token.expose()),
        None => viewer,
    })
}
fn service(state: &AppState) -> Result<Arc<DiscoveryService>, ApiError> {
    state
        .discovery
        .clone()
        .ok_or_else(|| ApiError::NotFound("Discovery is unavailable".into()))
}
fn error(error: Error) -> ApiError {
    let message = error.to_string();
    match error {
        Error::Unavailable | Error::Expired => ApiError::NotFound(message),
        Error::Forbidden => ApiError::Forbidden(message),
        Error::Invalid => ApiError::BadRequest(message),
        Error::Conflict | Error::AlreadyRequested => ApiError::Conflict(message),
        _ => ApiError::ServiceUnavailable(message),
    }
}

macro_rules! search_handler {
    ($name:ident,$audience:expr) => {
        async fn $name(
            State(state): State<AppState>,
            RequireAuth(auth): RequireAuth,
            Query(query): Query<Search>,
        ) -> Result<Json<DiscoveryPage>, ApiError> {
            let viewer = viewer(&auth)?;
            Ok(Json(
                service(&state)?
                    .search_scope(
                        viewer,
                        $audience,
                        &query.query,
                        query.page.unwrap_or(1),
                        query.scope,
                    )
                    .await
                    .map_err(error)?,
            ))
        }
    };
}
macro_rules! action_handler {
    ($name:ident,$audience:expr) => {
        async fn $name(
            State(state): State<AppState>,
            RequireAuth(auth): RequireAuth,
            Path(id): Path<Uuid>,
        ) -> Result<Json<ActionForm>, ApiError> {
            Ok(Json(
                service(&state)?
                    .action(viewer(&auth)?, $audience, id)
                    .await
                    .map_err(error)?,
            ))
        }
    };
}
macro_rules! submit_handler {
    ($name:ident,$audience:expr) => {
        async fn $name(
            State(state): State<AppState>,
            RequireAuth(auth): RequireAuth,
            Json(input): Json<SubmitAction>,
        ) -> Result<Json<ActionReceipt>, ApiError> {
            Ok(Json(
                service(&state)?
                    .submit(viewer(&auth)?, $audience, input)
                    .await
                    .map_err(error)?,
            ))
        }
    };
}
search_handler!(client_search, Audience::Client);
search_handler!(web_search, Audience::Web);
action_handler!(client_action, Audience::Client);
action_handler!(web_action, Audience::Web);
submit_handler!(client_submit, Audience::Client);
submit_handler!(web_submit, Audience::Web);

async fn settings(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<SettingsSnapshot>, ApiError> {
    Ok(Json(service(&state)?.settings().await))
}
async fn save(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Json(input): Json<SaveSettings>,
) -> Result<Json<SettingsSnapshot>, ApiError> {
    Ok(Json(service(&state)?.save(input).await.map_err(error)?))
}

/// Registers current-viewer discovery and administrator-only settings.
pub fn register(router: Router<AppState>) -> Router<AppState> {
    router
        .route("/Bluefin/Discovery/Search", get(client_search))
        .route("/Bluefin/Discovery/Actions/{id}", get(client_action))
        .route(
            "/Bluefin/Discovery/Requests",
            post(client_submit).layer(DefaultBodyLimit::max(16_384)),
        )
        .route("/Bluefin/Web/Discovery/Search", get(web_search))
        .route("/Bluefin/Web/Discovery/Actions/{id}", get(web_action))
        .route(
            "/Bluefin/Web/Discovery/Requests",
            post(web_submit).layer(DefaultBodyLimit::max(16_384)),
        )
        .route(
            "/Bluefin/Admin/Discovery",
            get(settings)
                .put(save)
                .layer(DefaultBodyLimit::max(131_072)),
        )
}
