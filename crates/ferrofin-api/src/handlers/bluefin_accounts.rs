//! Server-owned account onboarding policy, separate from Jellyfin admin user creation.

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ferrofin_model::secret::Secret;
use ferrofin_traits::error::ServiceError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::auth::RequireAdmin;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Default)]
pub(crate) struct SignupBudget {
    window: Option<(Instant, u8)>,
}

impl SignupBudget {
    fn acquire(&mut self, now: Instant) -> bool {
        let (start, count) = self.window.get_or_insert((now, 0));
        if now.duration_since(*start) >= Duration::from_secs(60) {
            *start = now;
            *count = 0;
        }
        if *count == 8 {
            return false;
        }
        *count += 1;
        true
    }
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Settings {
    allow_signups: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Options {
    allow_signups: bool,
    quick_connect: bool,
    passkeys: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Signup {
    username: String,
    password: Secret,
}

fn settings_path(state: &AppState) -> PathBuf {
    PathBuf::from(
        state
            .config
            .application_paths()
            .configuration_directory_path(),
    )
    .join("bluefin-accounts.json")
}

async fn read_settings(state: &AppState) -> Result<Settings, ApiError> {
    match tokio::fs::read(settings_path(state)).await {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
            ServiceError::backend(format!("Reading Bluefin account settings: {error}")).into()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(error) => {
            Err(ServiceError::backend(format!("Reading Bluefin account settings: {error}")).into())
        }
    }
}

async fn options(State(state): State<AppState>) -> Result<Json<Options>, ApiError> {
    let settings = read_settings(&state).await?;
    let ready = state
        .config
        .configuration()
        .await?
        .is_startup_wizard_completed;
    Ok(Json(Options {
        allow_signups: ready && settings.allow_signups,
        passkeys: ready
            && state.passkeys.is_some()
            && state.quick_connect.managed_pairing().is_some(),
        quick_connect: ready && settings.allow_signups && state.quick_connect.is_enabled().await?,
    }))
}

async fn get_settings(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<Settings>, ApiError> {
    Ok(Json(read_settings(&state).await?))
}

async fn set_settings(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Json(settings): Json<Settings>,
) -> Result<StatusCode, ApiError> {
    let _guard = state.account_gate.lock().await;
    let destination = settings_path(&state);
    let temporary = destination.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let bytes =
        serde_json::to_vec(&settings).map_err(|error| ServiceError::backend(error.to_string()))?;
    let result = async {
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .await?;
        file.write_all(&bytes).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&temporary, &destination).await
    }
    .await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(temporary).await;
        return Err(
            ServiceError::backend(format!("Saving Bluefin account settings: {error}")).into(),
        );
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn signup(
    State(state): State<AppState>,
    Json(input): Json<Signup>,
) -> Result<StatusCode, ApiError> {
    let mut gate = state.account_gate.lock().await;
    if !read_settings(&state).await?.allow_signups
        || !state
            .config
            .configuration()
            .await?
            .is_startup_wizard_completed
    {
        return Err(ApiError::Forbidden(
            "Account creation is turned off on this server.".into(),
        ));
    }
    if !gate.acquire(Instant::now()) {
        return Ok(StatusCode::TOO_MANY_REQUESTS);
    }
    state
        .users
        .create_signup_user(&input.username, input.password.expose())
        .await?;
    Ok(StatusCode::CREATED)
}

async fn settings_page() -> Response {
    ([
        (header::CONTENT_SECURITY_POLICY, "default-src 'none'; script-src 'self'; connect-src 'self'; style-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"),
        (header::REFERRER_POLICY, "no-referrer"),
    ], Html(include_str!("bluefin_account_web/settings.html"))).into_response()
}

async fn settings_script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("bluefin_account_web/settings.js"),
    )
}

pub(crate) fn register(router: Router<AppState>) -> Router<AppState> {
    let routes = super::bluefin_passkeys::routes()
        .route("/Bluefin/Accounts/Options", get(options))
        .route(
            "/Bluefin/Accounts/Settings",
            get(get_settings).put(set_settings),
        )
        .route("/Bluefin/Accounts/Admin", get(settings_page))
        .route("/Bluefin/Accounts/Admin/client.js", get(settings_script))
        .route(
            "/Bluefin/Accounts/Create",
            post(signup).layer(DefaultBodyLimit::max(4096)),
        )
        .layer(axum::middleware::from_fn(
            |request: axum::extract::Request, next: axum::middleware::Next| async move {
                let mut response = next.run(request).await;
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                response
            },
        ));
    router.merge(routes)
}
