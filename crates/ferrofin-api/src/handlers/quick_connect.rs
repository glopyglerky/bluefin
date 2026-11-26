//! `QuickConnectController` — the Quick Connect pairing flow.
//!
//! Ports Jellyfin's `QuickConnectController`:
//!
//! - `GET  /QuickConnect/Enabled` — whether Quick Connect is active.
//! - `POST /QuickConnect/Initiate` — start a new pairing request.
//! - `GET  /QuickConnect/Connect` — poll a request's status by its secret.
//! - `POST /QuickConnect/Authorize` — authorize a pending request by its code.
//!
//! All four delegate to the [`QuickConnect`](ferrofin_traits::security::QuickConnect)
//! manager. `Initiate` reads the caller's parsed [`AuthorizationInfo`]; `Authorize`
//! is behind `[Authorize]` and targets `userId` (or the caller when omitted).

use axum::extract::State;
use axum::http::request::Parts;
use axum::routing::{get, post};
use axum::{Json, Router};
use ferrofin_model::quick_connect::QuickConnectResult;
use ferrofin_model::secret::Secret;
use ferrofin_traits::options::AuthorizationInfo;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::RequireAuth;
use crate::error::ApiError;
use crate::extract::Query;
use crate::handlers::items::effective_user_id;
use crate::state::AppState;

/// Query parameters for `GET /QuickConnect/Connect` (the request secret).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectQuery {
    /// The secret returned by the initiate endpoint.
    #[serde(default)]
    secret: Secret,
}

/// Query parameters for `POST /QuickConnect/Authorize` (code + optional user).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthorizeQuery {
    /// The user-facing code to authorize.
    #[serde(default)]
    code: String,
    /// The user to authorize as; defaults to the caller when absent.
    #[serde(
        default,
        deserialize_with = "crate::handlers::query_parse::empty_as_none_uuid"
    )]
    user_id: Option<Uuid>,
}

/// Reads the client identity parsed by the auth-context middleware into this
/// request's [`AuthorizationInfo`] extension.
fn auth_info(parts: &Parts) -> AuthorizationInfo {
    parts
        .extensions
        .get::<AuthorizationInfo>()
        .cloned()
        .unwrap_or_default()
}

/// `GET /QuickConnect/Enabled` — whether Quick Connect is active.
///
/// Port of `QuickConnectController.GetQuickConnectEnabled`.
#[utoipa::path(
    get,
    path = "/QuickConnect/Enabled",
    responses((status = 200, description = "Quick Connect enabled state", body = bool)),
    tag = "ferrofin"
)]
async fn get_enabled(State(state): State<AppState>) -> Result<Json<bool>, ApiError> {
    Ok(Json(state.quick_connect.is_enabled().await?))
}

/// `POST /QuickConnect/Initiate` — start a new pairing request.
///
/// Port of `QuickConnectController.InitiateQuickConnect`: reads the caller's
/// parsed authorization info and opens a new request.
#[utoipa::path(
    post,
    path = "/QuickConnect/Initiate",
    responses(
        (status = 200, description = "Quick Connect request created", body = QuickConnectResult),
        (status = 401, description = "Quick Connect is disabled")
    ),
    tag = "ferrofin"
)]
async fn initiate(
    State(state): State<AppState>,
    parts: Parts,
) -> Result<Json<QuickConnectResult>, ApiError> {
    let auth = auth_info(&parts);
    Ok(Json(state.quick_connect.try_connect(&auth).await?))
}

/// `GET /QuickConnect/Connect` — poll a request's status by its secret.
///
/// Port of `QuickConnectController.GetQuickConnectState`.
#[utoipa::path(
    get,
    path = "/QuickConnect/Connect",
    params(("secret" = String, Query, description = "The request secret")),
    responses(
        (status = 200, description = "Quick Connect result returned", body = QuickConnectResult),
        (status = 404, description = "Unknown secret")
    ),
    tag = "ferrofin"
)]
async fn connect(
    State(state): State<AppState>,
    Query(query): Query<ConnectQuery>,
) -> Result<Json<QuickConnectResult>, ApiError> {
    Ok(Json(
        state
            .quick_connect
            .check_request_status(query.secret.expose())
            .await?,
    ))
}

/// `POST /QuickConnect/Authorize` — authorize a pending request by its code.
///
/// Port of `QuickConnectController.AuthorizeQuickConnect`: authorizes the request
/// for `userId` (or the caller when omitted).
#[utoipa::path(
    post,
    path = "/QuickConnect/Authorize",
    params(
        ("code" = String, Query, description = "The code to authorize"),
        ("userId" = Option<String>, Query, description = "The user to authorize as")
    ),
    responses(
        (status = 200, description = "Authorization result", body = bool),
        (status = 403, description = "Unknown user id")
    ),
    tag = "ferrofin"
)]
async fn authorize(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Query(query): Query<AuthorizeQuery>,
) -> Result<Json<bool>, ApiError> {
    // C# `userId = RequestHelpers.GetUserId(User, userId)`. This gate is
    // load-bearing here beyond parity: authorizing a pending request mints a
    // token for the *named* user on the pairing device, so an ungated `userId`
    // let any authenticated account hand out a session as any other account.
    let user_id = effective_user_id(&state, &auth, query.user_id).await?;
    Ok(Json(
        state
            .quick_connect
            .authorize_request(user_id, &query.code)
            .await?,
    ))
}

/// Registers this controller's real routes onto `router`.
pub fn register(router: Router<AppState>) -> Router<AppState> {
    let managed_routes = Router::new()
        .route("/Bluefin/Pairing/Capabilities", get(managed_capability))
        .route(
            "/Bluefin/Pairing/Initiate",
            post(managed_initiate).layer(axum::extract::DefaultBodyLimit::max(1024)),
        )
        .route(
            "/Bluefin/Pairing/Authorize",
            post(managed_authorize).layer(axum::extract::DefaultBodyLimit::max(1024)),
        )
        .route(
            "/Bluefin/Pairing/Status",
            post(managed_status).layer(axum::extract::DefaultBodyLimit::max(1024)),
        )
        .route(
            "/Bluefin/Pairing/Cancel",
            post(managed_cancel).layer(axum::extract::DefaultBodyLimit::max(1024)),
        )
        .route(
            "/Bluefin/Pairing/Exchange",
            post(managed_exchange).layer(axum::extract::DefaultBodyLimit::max(1024)),
        )
        .layer(axum::middleware::from_fn(
            |request: axum::extract::Request, next: axum::middleware::Next| async move {
                let mut response = next.run(request).await;
                response.headers_mut().insert(
                    axum::http::header::CACHE_CONTROL,
                    axum::http::HeaderValue::from_static("no-store"),
                );
                response
            },
        ));
    router
        .merge(managed_routes)
        .route("/QuickConnect/Enabled", get(get_enabled))
        .route("/QuickConnect/Initiate", post(initiate))
        .route("/QuickConnect/Connect", get(connect))
        .route("/QuickConnect/Authorize", post(authorize))
}
crate::query::query_parameters! {
    ConnectQuery {} => [("get", "/QuickConnect/Connect")];
    AuthorizeQuery {} => [("post", "/QuickConnect/Authorize")];
}

// Managed credentials occur only in JSON bodies, never route/query values.
use ferrofin_traits::error::ServiceError;
use ferrofin_traits::managed_pairing::{
    ManagedPairing, PairingChallenge, PairingProof, PairingStatus,
};
fn managed(state: &AppState) -> Result<&dyn ManagedPairing, ApiError> {
    state
        .quick_connect
        .managed_pairing()
        .ok_or_else(|| ServiceError::Unavailable("managed pairing unsupported".into()).into())
}
async fn managed_capability(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"managed_pairing_v1":state.quick_connect.managed_pairing().is_some()}))
}
async fn managed_initiate(
    State(state): State<AppState>,
    parts: Parts,
    body: axum::body::Bytes,
) -> Result<Json<PairingChallenge>, ApiError> {
    if !body.is_empty() {
        return Err(
            ServiceError::invalid_input("managed initiation does not accept a body").into(),
        );
    }
    if !state.quick_connect.is_enabled().await? {
        return Err(ServiceError::unauthorized("pairing disabled").into());
    }
    Ok(Json(managed(&state)?.initiate(&auth_info(&parts)).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedAuthorize {
    proof: PairingProof,
    device_id: String,
}
async fn managed_authorize(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Json(body): Json<ManagedAuthorize>,
) -> Result<Json<PairingStatus>, ApiError> {
    if !state.quick_connect.is_enabled().await? {
        return Err(ServiceError::unauthorized("pairing disabled").into());
    }
    if auth.is_api_key || auth.user_id().is_nil() {
        return Err(ServiceError::unauthorized("actual user required").into());
    }
    Ok(Json(
        managed(&state)?
            .authorize(&body.proof, &body.device_id, auth.user_id())
            .await?,
    ))
}
async fn managed_status(
    State(state): State<AppState>,
    Json(proof): Json<PairingProof>,
) -> Result<Json<PairingStatus>, ApiError> {
    Ok(Json(managed(&state)?.status(&proof).await?))
}
async fn managed_cancel(
    State(state): State<AppState>,
    Json(proof): Json<PairingProof>,
) -> Result<Json<PairingStatus>, ApiError> {
    Ok(Json(managed(&state)?.cancel(&proof).await?))
}
async fn managed_exchange(
    State(state): State<AppState>,
    Json(proof): Json<PairingProof>,
) -> Result<Json<ferrofin_model::session::AuthenticationResult>, ApiError> {
    if !state.quick_connect.is_enabled().await? {
        return Err(ServiceError::unauthorized("pairing disabled").into());
    }
    let result = managed(&state)?.exchange(&proof).await?;
    Ok(Json(
        crate::handlers::users::authentication_result(&state, result).await,
    ))
}
