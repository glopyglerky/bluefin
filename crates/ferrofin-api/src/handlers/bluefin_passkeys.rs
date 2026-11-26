//! WebAuthn uses the configured HTTPS origin and server-side, short-lived ceremonies.

use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{StatusCode, header, request::Parts};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ferrofin_db::Database;
use ferrofin_model::secret::Secret;
use ferrofin_traits::{
    error::ServiceError,
    managed_pairing::{ManagedPairing, PairingProof},
    options::AuthorizationInfo,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use url::Url;
use uuid::Uuid;
use webauthn_rs::prelude::*;

use crate::error::ApiError;
use crate::state::AppState;

const MAX_PENDING: usize = 128;
const LIFETIME: Duration = Duration::from_secs(300);
const MAX_KEYS: usize = 12;

pub(crate) struct Service {
    db: Database,
    webauthn: Webauthn,
    page: Url,
    origin: String,
    pending: Mutex<HashMap<Uuid, Pending>>,
}

struct Pending {
    pairing: PairingProof,
    expires: Instant,
    device_id: String,
    phase: Phase,
}

enum Phase {
    Waiting,
    Checking,
    Enroll(Uuid, PasskeyRegistration),
    SignIn(Uuid, PasskeyAuthentication),
    Approved,
}

impl Service {
    pub(crate) fn new(db: Database, published_url: &str) -> Option<Self> {
        let mut page = Url::parse(published_url).ok()?;
        if page.scheme() != "https"
            || !page.username().is_empty()
            || page.password().is_some()
            || page.query().is_some()
            || page.fragment().is_some()
        {
            return None;
        }
        let host = page.domain()?;
        let origin = page.origin().ascii_serialization();
        let relying_party_origin = Url::parse(&origin).ok()?;
        let webauthn = WebauthnBuilder::new(host, &relying_party_origin)
            .ok()?
            .rp_name("Bluefin")
            .build()
            .ok()?;
        page.path_segments_mut()
            .ok()?
            .pop_if_empty()
            .extend(["Bluefin", "Accounts", "Passkey"]);
        Some(Self {
            db,
            webauthn,
            page,
            origin,
            pending: Mutex::new(HashMap::new()),
        })
    }

    pub(crate) fn origin(&self) -> &str { &self.origin }

    async fn credentials(&self, user: Uuid) -> Result<Vec<Passkey>, ApiError> {
        let credentials: Vec<String> = sqlx::query_scalar(
            "SELECT Credential FROM BluefinPasskeys WHERE UserId = ? ORDER BY CredentialId LIMIT 12"
        )
            .bind(ferrofin_db::store::guid_to_db(user))
            .fetch_all(self.db.pool()).await.map_err(storage_error)?;
        credentials
            .into_iter()
            .map(|credential| serde_json::from_str(&credential).map_err(storage_error))
            .collect()
    }
}

fn storage_error(error: impl std::fmt::Display) -> ApiError {
    ServiceError::backend(format!("Passkey storage: {error}")).into()
}

fn unavailable() -> ApiError {
    ApiError::NotFound("Passkeys are unavailable on this server.".into())
}
fn expired() -> ApiError {
    ApiError::NotFound("Start a new passkey request on your device.".into())
}
fn refused() -> ApiError {
    ApiError::Unauthorized("Unable to verify this account.".into())
}

fn passkeys(state: &AppState) -> Result<&Service, ApiError> {
    state.passkeys.as_deref().ok_or_else(unavailable)
}

fn pairing(state: &AppState) -> Result<&dyn ManagedPairing, ApiError> {
    state
        .quick_connect
        .managed_pairing()
        .ok_or_else(unavailable)
}

fn browser_origin(service: &Service, parts: &Parts) -> Result<(), ApiError> {
    if parts
        .headers
        .get(header::ORIGIN)
        .and_then(|h| h.to_str().ok())
        == Some(service.origin.as_str())
    {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "Open the passkey page on the configured server.".into(),
        ))
    }
}

#[derive(Serialize, Deserialize)]
struct Proof {
    ticket: Uuid,
    pairing: PairingProof,
}

#[derive(Serialize)]
struct Initiation {
    url: String,
    qr_url: String,
    code: String,
    proof: Proof,
}

async fn initiate(
    State(state): State<AppState>,
    parts: Parts,
) -> Result<Json<Initiation>, ApiError> {
    let service = passkeys(&state)?;
    if !state
        .config
        .configuration()
        .await?
        .is_startup_wizard_completed
    {
        return Err(unavailable());
    }
    let mut auth = parts
        .extensions
        .get::<AuthorizationInfo>()
        .cloned()
        .unwrap_or_default();
    auth.remote_endpoint = Some(state.client_address(&parts).to_string());
    let mut pending = service.pending.lock().await;
    let now = Instant::now();
    pending.retain(|_, request| request.expires > now);
    if pending.len() >= MAX_PENDING {
        return Err(ApiError::ServiceUnavailable(
            "Try again in a few minutes.".into(),
        ));
    }
    let ticket = Uuid::new_v4();
    let challenge = pairing(&state)?.initiate(&auth).await?;
    let proof = Proof {
        ticket,
        pairing: challenge.proof.clone(),
    };
    pending.insert(
        ticket,
        Pending {
            pairing: challenge.proof,
            expires: now + LIFETIME,
            device_id: challenge.device_id,
            phase: Phase::Waiting,
        },
    );
    let mut page = service.page.clone();
    page.set_fragment(Some(&ticket.simple().to_string()));
    Ok(Json(Initiation {
        url: page.to_string(),
        qr_url: passkey_qr_url(service, ticket)?.to_string(),
        code: ticket.simple().to_string()[..6].to_uppercase(),
        proof,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    ticket: Uuid,
    username: String,
    // Supplied only when adding a new credential to an existing account.
    password: Option<Secret>,
}

async fn start(
    State(state): State<AppState>,
    parts: Parts,
    Json(input): Json<Start>,
) -> Result<Json<Value>, ApiError> {
    let service = passkeys(&state)?;
    browser_origin(service, &parts)?;
    if input.username.is_empty()
        || input.username.len() > 256
        || input
            .password
            .as_ref()
            .is_some_and(|v| v.expose().len() > 512)
    {
        return Err(refused());
    }
    {
        let mut pending = service.pending.lock().await;
        let request = pending
            .get_mut(&input.ticket)
            .filter(|r| r.expires > Instant::now())
            .ok_or_else(expired)?;
        if !matches!(request.phase, Phase::Waiting) {
            return Err(ApiError::Conflict(
                "Start a new request on your device.".into(),
            ));
        }
        request.phase = Phase::Checking;
    }
    let result = async {
        let user = if let Some(password) = &input.password {
            if password.expose().is_empty() {
                return Err(refused());
            }
            state
                .users
                .authenticate_user(
                    &input.username,
                    password.expose(),
                    &state.client_address(&parts).to_string(),
                    true,
                )
                .await?
        } else {
            state.users.get_user_by_name(&input.username).await?
        }
        .ok_or_else(refused)?;
        let uid = Uuid::parse_str(&user.id).map_err(storage_error)?;
        let credentials = service.credentials(uid).await?;
        if input.password.is_some() {
            if credentials.len() >= MAX_KEYS {
                return Err(ApiError::Conflict(
                    "This account already has the maximum number of passkeys.".into(),
                ));
            }
            let exclusion = credentials.iter().map(|c| c.cred_id().clone()).collect();
            let (public, private) = service
                .webauthn
                .start_passkey_registration(uid, &user.username, &user.username, Some(exclusion))
                .map_err(|_| refused())?;
            Ok((
                serde_json::to_value(public).map_err(storage_error)?,
                Phase::Enroll(uid, private),
            ))
        } else {
            if credentials.is_empty() {
                return Err(refused());
            }
            let (public, private) = service
                .webauthn
                .start_passkey_authentication(&credentials)
                .map_err(|_| refused())?;
            Ok((
                serde_json::to_value(public).map_err(storage_error)?,
                Phase::SignIn(uid, private),
            ))
        }
    }
    .await;
    let mut pending = service.pending.lock().await;
    let request = pending
        .get_mut(&input.ticket)
        .filter(|r| r.expires > Instant::now())
        .ok_or_else(expired)?;
    match result {
        Ok((options, phase)) => {
            request.phase = phase;
            Ok(Json(options))
        }
        Err(error) => {
            request.phase = Phase::Waiting;
            Err(error)
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Finish {
    ticket: Uuid,
    credential: Value,
}

async fn finish(
    State(state): State<AppState>,
    parts: Parts,
    Json(input): Json<Finish>,
) -> Result<StatusCode, ApiError> {
    let service = passkeys(&state)?;
    browser_origin(service, &parts)?;
    // Keep verification and approval linear with respect to exchange and cancellation.
    let mut pending = service.pending.lock().await;
    let request = pending
        .get_mut(&input.ticket)
        .filter(|r| r.expires > Instant::now())
        .ok_or_else(expired)?;
    if !matches!(request.phase, Phase::Enroll(..) | Phase::SignIn(..)) {
        return Err(expired());
    }
    let phase = std::mem::replace(&mut request.phase, Phase::Waiting);
    let user = match phase {
        Phase::Enroll(user, ceremony) => {
            let credential = serde_json::from_value(input.credential).map_err(|_| refused())?;
            let key = service
                .webauthn
                .finish_passkey_registration(&credential, &ceremony)
                .map_err(|_| refused())?;
            let payload = serde_json::to_string(&key).map_err(storage_error)?;
            let id = ferrofin_db::store::guid_to_db(user);
            let result = sqlx::query(
                "INSERT INTO BluefinPasskeys (CredentialId, UserId, Credential)
                 SELECT ?, ?, ? WHERE (SELECT COUNT(*) FROM BluefinPasskeys WHERE UserId = ?) < 12
                 ON CONFLICT(CredentialId) DO NOTHING",
            )
            .bind(key.cred_id().as_ref())
            .bind(&id)
            .bind(payload)
            .bind(&id)
            .execute(service.db.writer())
            .await
            .map_err(storage_error)?;
            if result.rows_affected() != 1 {
                return Err(refused());
            }
            user
        }
        Phase::SignIn(user, ceremony) => {
            let credential = serde_json::from_value(input.credential).map_err(|_| refused())?;
            let result = service
                .webauthn
                .finish_passkey_authentication(&credential, &ceremony)
                .map_err(|_| refused())?;
            let id = ferrofin_db::store::guid_to_db(user);
            let previous: String = sqlx::query_scalar(
                "SELECT Credential FROM BluefinPasskeys WHERE UserId = ? AND CredentialId = ?",
            )
            .bind(&id)
            .bind(result.cred_id().as_ref())
            .fetch_optional(service.db.pool())
            .await
            .map_err(storage_error)?
            .ok_or_else(refused)?;
            let mut key: Passkey = serde_json::from_str(&previous).map_err(storage_error)?;
            if key.update_credential(&result).is_none() {
                return Err(refused());
            }
            let payload = serde_json::to_string(&key).map_err(storage_error)?;
            let changed = sqlx::query("UPDATE BluefinPasskeys SET Credential = ? WHERE UserId = ? AND CredentialId = ? AND Credential = ?")
                .bind(payload).bind(&id).bind(result.cred_id().as_ref()).bind(previous).execute(service.db.writer()).await.map_err(storage_error)?;
            if changed.rows_affected() != 1 {
                return Err(refused());
            }
            user
        }
        _ => return Err(expired()),
    };
    request.phase = Phase::Checking;
    let proof = request.pairing.clone();
    let device_id = request.device_id.clone();
    drop(pending);
    let status = pairing(&state)?.authorize(&proof, &device_id, user).await?;
    if status.state != "authorized" {
        return Err(refused());
    }
    let mut pending = service.pending.lock().await;
    let request = pending.get_mut(&input.ticket).ok_or_else(expired)?;
    request.phase = Phase::Approved;
    Ok(StatusCode::NO_CONTENT)
}

async fn exchange(
    State(state): State<AppState>,
    Json(proof): Json<Proof>,
) -> Result<Response, ApiError> {
    let service = passkeys(&state)?;
    let pending = service.pending.lock().await;
    let request = pending
        .get(&proof.ticket)
        .filter(|r| r.pairing.challenge == proof.pairing.challenge && r.expires > Instant::now())
        .ok_or_else(expired)?;
    // Validate the device's secret through the managed store even while waiting.
    let approved = matches!(request.phase, Phase::Approved);
    drop(pending);
    let manager = pairing(&state)?;
    let status = manager.status(&proof.pairing).await?;
    if !approved || !matches!(status.state.as_str(), "authorized" | "exchanged") {
        return Ok(Json(serde_json::json!({"pending": true})).into_response());
    }
    let session = manager.exchange(&proof.pairing).await?;
    Ok(Json(super::users::authentication_result(&state, session).await).into_response())
}

async fn cancel(
    State(state): State<AppState>,
    Json(proof): Json<Proof>,
) -> Result<StatusCode, ApiError> {
    let service = passkeys(&state)?;
    let status = pairing(&state)?.cancel(&proof.pairing).await?;
    if status.state != "cancelled" {
        return Err(ApiError::Conflict(
            "The request could not be canceled.".into(),
        ));
    }
    let mut pending = service.pending.lock().await;
    if pending
        .get(&proof.ticket)
        .is_some_and(|r| r.pairing.challenge == proof.pairing.challenge)
    {
        pending.remove(&proof.ticket);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn page(State(state): State<AppState>) -> Result<Response, ApiError> {
    passkeys(&state)?;
    Ok(([
        (header::CONTENT_SECURITY_POLICY, "default-src 'none'; script-src 'self'; connect-src 'self'; style-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"),
        (header::REFERRER_POLICY, "no-referrer"),
    ], Html(include_str!("bluefin_account_web/passkey.html"))).into_response())
}

fn passkey_qr_url(service: &Service, ticket: Uuid) -> Result<Url, ApiError> {
    let mut url = service.page.clone();
    url.path_segments_mut()
        .map_err(|()| unavailable())?
        .extend(["QR", &ticket.simple().to_string()]);
    Ok(url)
}

async fn qr(State(state): State<AppState>, Path(ticket): Path<Uuid>) -> Result<Response, ApiError> {
    let service = passkeys(&state)?;
    let pending = service.pending.lock().await;
    if !pending
        .get(&ticket)
        .is_some_and(|request| request.expires > Instant::now())
    {
        return Err(expired());
    }
    drop(pending);
    let mut url = service.page.clone();
    url.set_fragment(Some(&ticket.simple().to_string()));
    let code = qrcode::QrCode::new(url.as_str()).map_err(storage_error)?;
    let image = code
        .render::<image::Luma<u8>>()
        .min_dimensions(256, 256)
        .build();
    let mut png = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(storage_error)?;
    Ok(([(header::CONTENT_TYPE, "image/png")], png.into_inner()).into_response())
}

async fn script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("bluefin_account_web/passkey.js"),
    )
}

async fn style() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("bluefin_account_web/accounts.css"),
    )
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/Bluefin/Accounts/Passkey", get(page))
        .route("/Bluefin/Accounts/Passkey/client.js", get(script))
        .route("/Bluefin/Accounts/Passkey/QR/{ticket}", get(qr))
        .route("/Bluefin/Accounts/Passkey/accounts.css", get(style))
        .route("/Bluefin/Accounts/Passkey/Initiate", post(initiate))
        .route("/Bluefin/Accounts/Passkey/Challenge", post(start))
        .route("/Bluefin/Accounts/Passkey/Verify", post(finish))
        .route("/Bluefin/Accounts/Passkey/Exchange", post(exchange))
        .route("/Bluefin/Accounts/Passkey/Cancel", post(cancel))
        .layer(DefaultBodyLimit::max(24 * 1024))
}
