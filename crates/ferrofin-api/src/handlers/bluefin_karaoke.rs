//! Authenticated supplied imports and opt-in real server preparation.
use crate::{
    error::ApiError,
    karaoke::{Session, Status, authority::Source},
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use bluefin_karaoke::Admitted;
use serde::Deserialize;
use uuid::Uuid;
static READS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct Selection {
    media_source_id: Option<String>,
}
impl crate::extract::QueryParameters for Selection {
    const COLLECTIONS: &'static [(&'static str, char)] = &[];
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Preparation {
    request_id: Uuid,
    media_source_id: Option<String>,
}
fn valid(item: Uuid, request: Uuid, selection: Option<&str>) -> Result<(), ApiError> {
    if item.is_nil()
        || request.is_nil()
        || selection.is_some_and(|id| id.len() > 64 || Uuid::parse_str(id).is_err())
    {
        return Err(ApiError::BadRequest("invalid karaoke selection".into()));
    }
    Ok(())
}
fn disabled() -> ApiError {
    ApiError::NotFound("karaoke preparation unavailable".into())
}
fn unavailable() -> ApiError {
    ApiError::NotFound("karaoke unavailable".into())
}
fn no_store(value: impl serde::Serialize) -> Response {
    (
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(value),
    )
        .into_response()
}
async fn prepare(
    State(state): State<AppState>,
    session: Session,
    Path(item): Path<Uuid>,
    Json(body): Json<Preparation>,
) -> Result<Response, ApiError> {
    valid(item, body.request_id, body.media_source_id.as_deref())?;
    let service = state.karaoke().ok_or_else(disabled)?.clone();
    let auth = session.authorize(&state).await?;
    super::items::require_visible_item(&state, item, auth.user.as_deref()).await?;
    let sources = state
        .media_sources
        .get_static_media_sources(item, false, Some(session.user))
        .await?;
    let selected = match body.media_source_id.as_deref() {
        Some(id) => sources
            .into_iter()
            .find(|source| source.id.as_deref() == Some(id)),
        None => sources.into_iter().next(),
    }
    .and_then(|source| source.id)
    .ok_or_else(unavailable)?;
    let selected_item = Uuid::parse_str(&selected).map_err(|_| unavailable())?;
    super::items::require_visible_item(&state, selected_item, auth.user.as_deref()).await?;
    let status = service.submit(
        state.clone(),
        session.clone(),
        item,
        body.request_id,
        Some(selected),
    )?;
    Ok(no_store(
        current_status(&state, &session, &service, status).await?,
    ))
}
async fn job(
    State(state): State<AppState>,
    session: Session,
    Path((item, request)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    job_status(state, session, item, request, false).await
}
async fn cancel(
    State(state): State<AppState>,
    session: Session,
    Path((item, request)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    job_status(state, session, item, request, true).await
}
async fn job_status(
    state: AppState,
    session: Session,
    item: Uuid,
    request: Uuid,
    cancel: bool,
) -> Result<Response, ApiError> {
    valid(item, request, None)?;
    let service = state.karaoke().ok_or_else(disabled)?;
    let auth = session.authorize(&state).await?;
    super::items::require_visible_item(&state, item, auth.user.as_deref()).await?;
    let status: Status = service.status(session.owner, item, request, cancel)?;
    Ok(no_store(if cancel {
        status
    } else {
        current_status(&state, &session, service, status).await?
    }))
}
async fn current_status(
    state: &AppState,
    session: &Session,
    service: &crate::karaoke::Service,
    mut status: Status,
) -> Result<Status, ApiError> {
    if status.state == crate::karaoke::State::Ready {
        let source = Source::capture(
            state,
            session,
            status.item_id,
            status.media_source_id.as_deref(),
        )
        .await?;
        match service.prepared(state, session, &source).await {
            Ok(_) => {}
            Err(ApiError::NotFound(_)) => {
                status.state = crate::karaoke::State::Unavailable;
                status.reason = Some(crate::karaoke::Reason::UnsupportedSource);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(status)
}
async fn admitted(
    state: AppState,
    session: Session,
    item: Uuid,
    selection: Option<String>,
) -> Result<Admitted, ApiError> {
    if item.is_nil() {
        return Err(unavailable());
    }
    if selection
        .as_ref()
        .is_some_and(|id| id.len() > 64 || Uuid::parse_str(id).is_err())
    {
        return Err(unavailable());
    }
    let permit = READS
        .try_acquire()
        .map_err(|_| ApiError::ServiceUnavailable("karaoke busy".into()))?;
    let source = Source::capture(&state, &session, item, selection.as_deref()).await?;
    if let Some(service) = state.karaoke() {
        match service.prepared(&state, &session, &source).await {
            Ok(value) => return Ok(value),
            Err(ApiError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }
    }
    let evidence = source.current(&state, &session).await?;
    let root = std::path::PathBuf::from(state.config.application_paths().internal_metadata_path())
        .join("bluefin-karaoke")
        .join(item.to_string());
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let original_hash = crate::karaoke::original_digest(evidence.input)?;
        let bytes = |name: &str, limit| {
            crate::karaoke::storage::read_descriptor(
                bluefin_karaoke::open_regular(&root.join(name)).map_err(|_| unavailable())?,
                limit,
            )
            .map_err(|_| unavailable())
        };
        let raw = bytes("manifest.json", 1024 * 1024)?;
        let import: bluefin_karaoke::Import =
            serde_json::from_slice(&raw).map_err(|_| unavailable())?;
        if !matches!(
            import.processing,
            bluefin_karaoke::Processing::SuppliedStems
        ) {
            return Err(unavailable());
        }
        let mut value = bluefin_karaoke::admit_bytes(
            &raw,
            item,
            Some(&source.selected),
            &original_hash,
            source.duration,
            bytes("accompaniment.wav", bluefin_karaoke::MAX_STEM)?,
            bytes("lead-vocal.wav", bluefin_karaoke::MAX_STEM)?,
        )
        .map_err(|_| unavailable())?;
        let current = handle.block_on(source.current(&state, &session))?;
        if crate::karaoke::original_digest(current.input)? != original_hash {
            return Err(unavailable());
        }
        // Legacy private imports may omit the source ID. The public response
        // carries the exact source just admitted and rechecked, never an
        // item-derived guess or a different edition selected by the client.
        value.package.media_source_id = Some(source.selected.clone());
        Ok(value)
    })
    .await
    .map_err(|_| ApiError::ServiceUnavailable("karaoke read unavailable".into()))?
}
async fn manifest(
    State(state): State<AppState>,
    session: Session,
    Path(item): Path<Uuid>,
    crate::extract::Query(query): crate::extract::Query<Selection>,
) -> Result<Response, ApiError> {
    Ok(no_store(
        admitted(state, session, item, query.media_source_id)
            .await?
            .package,
    ))
}
async fn stem(
    State(state): State<AppState>,
    session: Session,
    Path((item, revision, role)): Path<(Uuid, String, String)>,
    crate::extract::Query(query): crate::extract::Query<Selection>,
) -> Result<Response, ApiError> {
    if !matches!(role.as_str(), "accompaniment" | "lead-vocal")
        || revision.len() != 64
        || !revision.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(ApiError::BadRequest("invalid stem selection".into()));
    }
    let value = admitted(state, session, item, query.media_source_id).await?;
    if revision != value.package.revision {
        return Err(ApiError::Conflict("karaoke changed; prepare again".into()));
    }
    let bytes = if role == "accompaniment" {
        value.accompaniment
    } else {
        value.lead_vocal
    };
    Ok((
        [
            (axum::http::header::CACHE_CONTROL, "no-store"),
            (axum::http::header::CONTENT_TYPE, "audio/wav"),
        ],
        bytes,
    )
        .into_response())
}
/// Add authenticated karaoke routes; disabled preparation feature-detects as 404.
pub fn register(router: Router<AppState>) -> Router<AppState> {
    router
        .route("/Bluefin/Karaoke/{id}", get(manifest))
        .route("/Bluefin/Karaoke/{id}/{revision}/Stems/{role}", get(stem))
        .route(
            "/Bluefin/Karaoke/{id}/Preparation",
            post(prepare).layer(DefaultBodyLimit::max(1024)),
        )
        .route(
            "/Bluefin/Karaoke/{id}/Preparation/{request}",
            get(job).delete(cancel),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt as _;
    #[tokio::test]
    async fn cached_or_fake_authorization_cannot_replace_current_session_authority() {
        for state in [
            crate::test_support::fake_state(),
            crate::test_support::elevated_fake_state(),
        ] {
            let mut request = Request::builder()
                .uri("/Bluefin/Karaoke/00000000-0000-0000-0000-000000000001")
                .body(Body::empty())
                .unwrap();
            request
                .extensions_mut()
                .insert(ferrofin_traits::options::AuthorizationInfo {
                    is_authenticated: true,
                    ..Default::default()
                });
            let response = register(Router::new())
                .with_state(state)
                .oneshot(request)
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }
}
