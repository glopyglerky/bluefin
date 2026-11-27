//! Authenticated account-scoped Bluefin protocol extension.
use crate::{auth::RequireAuth, error::ApiError, extract::Query, state::AppState};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    routing::get,
};
use bluefin_contract::{
    Capabilities, MAX_QUEUE_BODY_BYTES, MAX_QUEUE_ITEMS, MusicQueue, PROTOCOL_VERSION,
    UpdateMusicQueue, ViewerCapabilities,
};
use ferrofin_traits::{error::ServiceError, options::AuthorizationInfo};
use uuid::Uuid;

pub(crate) const QUEUE_CLIENT: &str = "bluefin-music-queue-v1";
const QUEUE_ITEM: Uuid = Uuid::from_u128(0x626c_7565_6669_6e71_7565_7565_0000_0001);
// A single server owns its preferences database. Generic preference routes
// refuse this namespace. Retained across router clones and reconstruction.
static QUEUE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NoUserSelection {}
impl crate::extract::QueryParameters for NoUserSelection {
    const COLLECTIONS: &'static [(&'static str, char)] = &[];
}

fn viewer(auth: &AuthorizationInfo) -> Result<Uuid, ApiError> {
    if auth.is_api_key || !auth.is_authenticated || auth.user_id().is_nil() {
        return Err(ApiError::Forbidden(
            "a current user session is required".into(),
        ));
    }
    Ok(auth.user_id())
}

async fn capabilities(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Query(_): Query<NoUserSelection>,
    request: axum::extract::Request,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user_id = viewer(&auth)?;
    let policy = auth
        .user_policy
        .as_ref()
        .ok_or_else(|| ApiError::Forbidden("user policy required".into()))?;
    let mut capabilities = serde_json::to_value(Capabilities {
        protocol_version: PROTOCOL_VERSION,
        music_queue_sync: true,
        max_queue_items: MAX_QUEUE_ITEMS,
        discovery: match &state.discovery {
            Some(discovery) => {
                discovery
                    .capabilities(
                        bluefin_discovery::Viewer::new(user_id, policy.is_administrator),
                        bluefin_discovery::Audience::Client,
                    )
                    .await
            }
            None => None,
        },
        viewer: ViewerCapabilities {
            user_id,
            media_playback: policy.enable_media_playback,
            audio_transcoding: policy.enable_audio_playback_transcoding,
            content_downloading: policy.enable_content_downloading,
            live_tv_access: policy.enable_live_tv_access,
        },
    })
    .map_err(|_| ApiError::ServiceUnavailable("capabilities unavailable".into()))?;
    capabilities["karaokePreparation"] = serde_json::json!({
        "enabled": state.karaoke().is_some(), "maxDurationSeconds": 300,
        "requiresSuppliedTiming": false,
        "supportsStemOnly": state.karaoke().is_some(),
    });
    let (parts, _) = request.into_parts();
    if let Some(transport) = crate::iroh_media::advertised(&state, &parts).await {
        capabilities["irohMediaTransport"] = serde_json::to_value(transport)
            .map_err(|_| ApiError::ServiceUnavailable("transport unavailable".into()))?;
    }
    Ok(Json(capabilities))
}

async fn load(state: &AppState, user_id: Uuid) -> Result<MusicQueue, ApiError> {
    let prefs = state
        .display_preferences
        .list_custom_item_display_preferences(user_id, QUEUE_ITEM, QUEUE_CLIENT)
        .await?;
    let Some(Some(raw)) = prefs.get("queue") else {
        return Ok(MusicQueue::default());
    };
    if raw.len() > MAX_QUEUE_BODY_BYTES {
        return Err(ServiceError::backend("stored Bluefin queue exceeds limit").into());
    }
    let queue: MusicQueue = serde_json::from_str(raw)
        .map_err(|_| ServiceError::backend("invalid stored Bluefin queue"))?;
    if queue.protocol_version != PROTOCOL_VERSION || !queue.queue.is_valid() {
        return Err(ServiceError::backend("unsupported stored Bluefin queue").into());
    }
    Ok(queue)
}

async fn get_queue(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Query(_): Query<NoUserSelection>,
) -> Result<Json<MusicQueue>, ApiError> {
    let user_id = viewer(&auth)?;
    let mut stored = {
        let _guard = QUEUE_LOCK.lock().await;
        load(&state, user_id).await?
    };
    // Recheck on every read: revoking folder/tag access must hide stale IDs.
    let mut ids = Vec::new();
    let mut cursor = None;
    for (index, id) in stored.queue.item_ids.iter().enumerate() {
        if state
            .library
            .get_item_by_id_for_user(*id, auth.user.as_deref())
            .await?
            .is_some_and(|item| item.type_ == "Audio")
        {
            if stored.queue.cursor == Some(index) {
                cursor = Some(ids.len());
            }
            ids.push(*id);
        }
    }
    stored.queue.item_ids = ids;
    stored.queue.cursor = cursor;
    Ok(Json(stored))
}

async fn put_queue(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Query(_): Query<NoUserSelection>,
    Json(request): Json<UpdateMusicQueue>,
) -> Result<Json<MusicQueue>, ApiError> {
    let user_id = viewer(&auth)?;
    if request.protocol_version != PROTOCOL_VERSION || !request.queue.is_valid() {
        return Err(ApiError::BadRequest(
            "invalid queue or protocol version".into(),
        ));
    }
    // Detached from HTTP cancellation so the serialization lock survives until
    // the manager's atomic SQLite replacement has completed.
    tokio::spawn(async move {
        let _guard = QUEUE_LOCK.lock().await;
        let current = load(&state, user_id).await?;
        if current.revision != request.expected_revision {
            return Err(ApiError::Conflict(
                "queue revision changed; GET and retry".into(),
            ));
        }
        for id in &request.queue.item_ids {
            let item =
                super::items::require_visible_item(&state, *id, auth.user.as_deref()).await?;
            if item.type_ != "Audio" {
                return Err(ApiError::BadRequest("queue requires Audio items".into()));
            }
        }
        let next = MusicQueue {
            protocol_version: PROTOCOL_VERSION,
            revision: current
                .revision
                .checked_add(1)
                .ok_or_else(|| ApiError::Conflict("queue revision exhausted".into()))?,
            queue: request.queue,
        };
        let prefs = std::collections::HashMap::from([(
            "queue".to_owned(),
            Some(serde_json::to_string(&next).map_err(|e| ServiceError::backend(e.to_string()))?),
        )]);
        state
            .display_preferences
            .set_custom_item_display_preferences(user_id, QUEUE_ITEM, QUEUE_CLIENT, &prefs)
            .await?;
        Ok(Json(next))
    })
    .await
    .map_err(|e| ServiceError::backend(e.to_string()))?
}

/// Register Bluefin extensions alongside the existing Jellyfin-compatible API.
pub fn register(router: Router<AppState>) -> Router<AppState> {
    super::bluefin_discovery::register(super::bluefin_karaoke::register(router))
        .route("/Bluefin/Capabilities", get(capabilities))
        .route(
            "/Bluefin/Music/Queue",
            get(get_queue)
                .put(put_queue)
                .layer(DefaultBodyLimit::max(MAX_QUEUE_BODY_BYTES)),
        )
}
