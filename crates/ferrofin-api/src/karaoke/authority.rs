use crate::{ApiError, AppState};
use axum::{extract::FromRequestParts, http::request::Parts};
use ferrofin_model::data::BaseItemKind;
use ferrofin_traits::{
    library::{LocalAudioEvidence, MediaSourceManager},
    net::RequestContext,
    options::AuthorizationInfo,
};
use sha2::{Digest, Sha256};
use std::{net::IpAddr, sync::Arc};
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct Session {
    request: RequestContext,
    address: IpAddr,
    pub owner: [u8; 32],
    pub user: Uuid,
}
impl Session {
    fn scope(info: &AuthorizationInfo) -> Result<[u8; 32], ApiError> {
        if info.is_api_key
            || !info.is_authenticated
            || info.user_id().is_nil()
            || !info.has_token()
            || info.device_id.as_deref().is_none_or(str::is_empty)
        {
            return Err(ApiError::Forbidden("current user session required".into()));
        }
        let mut h = Sha256::new();
        h.update(info.user_id().as_bytes());
        for value in [
            info.device_id.as_deref().unwrap_or_default(),
            info.token.as_ref().map_or("", |s| s.expose()),
        ] {
            h.update((value.len() as u64).to_le_bytes());
            h.update(value.as_bytes());
        }
        Ok(h.finalize().into())
    }
    pub async fn authorize(&self, state: &AppState) -> Result<AuthorizationInfo, ApiError> {
        let info = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            state
                .auth_service
                .authenticate_current_session(&self.request),
        )
        .await
        .map_err(|_| ApiError::ServiceUnavailable("session checks unavailable".into()))??;
        if Self::scope(&info)? != self.owner || info.user_id() != self.user {
            return Err(ApiError::Unauthorized("session changed".into()));
        }
        let policy = crate::auth::user_authorization(&info, self.address, state, true).await?;
        if !policy.enable_media_playback
            || !policy.enable_content_downloading
            || !policy.enable_audio_playback_transcoding
        {
            return Err(ApiError::Forbidden(
                "audio processing permission required".into(),
            ));
        }
        if !self.address.is_loopback()
            && state.remote_access_policy(self.address)
                != ferrofin_networking::RemoteAccessPolicyResult::Allow
        {
            return Err(ApiError::Forbidden("current remote access denied".into()));
        }
        Ok(info)
    }
}
impl FromRequestParts<AppState> for Session {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let address = state.client_address(parts);
        let request = crate::auth::auth_request_context(
            &parts.headers,
            parts.uri.query(),
            Some(address.to_string()),
        );
        let info = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            state.auth_service.authenticate_current_session(&request),
        )
        .await
        .map_err(|_| ApiError::ServiceUnavailable("session checks unavailable".into()))??;
        let session = Self {
            owner: Self::scope(&info)?,
            user: info.user_id(),
            request,
            address,
        };
        session.authorize(state).await?;
        Ok(session)
    }
}

#[derive(Clone)]
pub(crate) struct Source {
    pub item: Uuid,
    pub selected: String,
    pub generation: String,
    pub duration: f64,
    pin: Arc<dyn MediaSourceManager>,
}
impl Source {
    pub async fn capture(
        state: &AppState,
        session: &Session,
        item: Uuid,
        selection: Option<&str>,
    ) -> Result<Self, ApiError> {
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            Self::capture_inner(state, session, item, selection),
        )
        .await
        .map_err(|_| ApiError::ServiceUnavailable("source checks unavailable".into()))?
    }
    async fn capture_inner(
        state: &AppState,
        session: &Session,
        item: Uuid,
        selection: Option<&str>,
    ) -> Result<Self, ApiError> {
        let info = session.authorize(state).await?;
        let row =
            crate::handlers::items::require_visible_item(state, item, info.user.as_deref()).await?;
        if BaseItemKind::from_stored_type_name(&row.type_) != Some(BaseItemKind::Audio) {
            return Err(super::missing());
        }
        let sources = state
            .media_sources
            .get_static_media_sources(item, false, Some(session.user))
            .await?;
        let source = match selection {
            Some(selected) => sources
                .into_iter()
                .find(|s| s.id.as_deref() == Some(selected)),
            None => sources.into_iter().next(),
        }
        .ok_or_else(super::missing)?;
        let selected = source.id.ok_or_else(super::missing)?;
        let path = source.path.ok_or_else(super::missing)?;
        let pin = state
            .media_sources
            .pin_local_source(item, &selected, &path, session.user)
            .await?;
        let evidence = pin.current_local_audio(item, session.user).await?;
        Ok(Self {
            item,
            selected: evidence.media_source_id,
            generation: evidence.generation,
            duration: evidence.duration_ticks as f64 / 10_000_000.,
            pin,
        })
    }
    pub async fn current(
        &self,
        state: &AppState,
        session: &Session,
    ) -> Result<LocalAudioEvidence, ApiError> {
        session.authorize(state).await?;
        let evidence = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            self.pin.current_local_audio(self.item, session.user),
        )
        .await
        .map_err(|_| ApiError::ServiceUnavailable("source checks unavailable".into()))??;
        if evidence.generation != self.generation || evidence.media_source_id != self.selected {
            return Err(super::missing());
        }
        Ok(evidence)
    }
    pub fn cache_key(&self, original: &str, timing: &str) -> String {
        super::digest(format!("{}|{}|{}|{}|{}|{}|{}|{}", self.item, self.selected, self.generation,
            original, super::MODEL_HASH, env!("CARGO_PKG_VERSION"),
            "bluefin-karaoke-engine-0.1.0/candle-core-0.11.1/rustfft-6.4.1/symphonia-0.5.5/rubato-0.16.2", timing).as_bytes())
    }
}
