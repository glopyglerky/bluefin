//! Bluefin TV playback capabilities over the existing authenticated planner and
//! stream handlers. Register once per router, before the normal IP/auth layers.
//! Each lease retains a private source-bound state/router and job ownership.
use crate::{
    auth::{auth_request_context, user_authorization},
    ip_access::ResolvedClientAddress,
    state::AppState,
};
use async_trait::async_trait;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderValue, Method, Request, StatusCode, header},
    response::Response,
};
use bluefin_tv_playback::{
    LeaseRequest, MediaKind,
    server::{Admission, Failure, PlaybackBackend},
};
use ferrofin_model::media_info::MediaProtocol;
use ferrofin_traits::options::AuthorizationInfo;
use serde_json::{Value, json};
use std::{net::IpAddr, sync::Arc};
use tower::ServiceExt;
use uuid::Uuid;

/// Captures the current opened input for this viewer and returns a fully
/// source-bound private view. The default application has no factory and does
/// not advertise TV playback. Returning a marker beside ordinary state is invalid.
#[async_trait]
pub trait LocalSourcePins: Send + Sync + 'static {
    /// Compare/open, authorize and rebind every source consumer to the guard.
    async fn pin(
        &self,
        state: &AppState,
        item: Uuid,
        source_id: &str,
        expected_local_path: &str,
        viewer: Uuid,
    ) -> Result<AppState, Failure>;
}
struct Backend {
    state: AppState,
    pins: Arc<dyn LocalSourcePins>,
}
struct PlaybackView {
    state: AppState,
    media: Router,
}
impl PlaybackView {
    fn new(state: AppState) -> Self {
        let media = super::media_info::register(super::hls::register(super::audio::register(
            super::videos::register(Router::new()),
        )))
        .with_state(state.clone());
        Self { state, media }
    }
}
struct PlaybackJobs {
    hls: Arc<dyn ferrofin_traits::media_encoding::HlsStreamManager>,
    viewer: Uuid,
    device: String,
    play_session: String,
    stopped: std::sync::atomic::AtomicBool,
    dispatch: Arc<tokio::sync::RwLock<()>>,
}
impl PlaybackJobs {
    fn stop(&self) -> tokio::task::JoinHandle<()> {
        self.stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let hls = Arc::clone(&self.hls);
        let dispatch = Arc::clone(&self.dispatch);
        let viewer = self.viewer;
        let device = self.device.clone();
        let play_session = self.play_session.clone();
        // Detached on timeout/cancellation. Stop waits for actual dispatch,
        // then retires only these server-generated client/session jobs.
        tokio::spawn(async move {
            // Interrupt an already registered startup too. A second stop after
            // the fence catches a dispatch that had not registered at this point.
            let _ = hls
                .stop_owned_encoding(viewer, &device, &play_session)
                .await;
            let _dispatch = dispatch.write_owned().await;
            let _ = hls
                .stop_owned_encoding(viewer, &device, &play_session)
                .await;
        })
    }
}
impl Drop for PlaybackJobs {
    fn drop(&mut self) {
        if !self.stopped.load(std::sync::atomic::Ordering::SeqCst)
            && tokio::runtime::Handle::try_current().is_ok()
        {
            drop(self.stop());
        }
    }
}
/// Private credentials, state, router and job ownership never enter a DTO.
struct PlaybackContext {
    authorization: HeaderValue,
    viewer: Uuid,
    device: String,
    item: Uuid,
    source: String,
    play_session: String,
    client: IpAddr,
    playback: Arc<PlaybackView>,
    jobs: Arc<PlaybackJobs>,
    operations: RequiredOperations,
}
/// The safe profile encodes each selected stream. Silent video does not need
/// an audio-transcoding grant; pin ownership never supplies either permission.
#[derive(Clone, Copy)]
struct RequiredOperations {
    audio_transcoding: bool,
    video_transcoding: bool,
}
fn failure(error: crate::error::ApiError) -> Failure {
    use axum::response::IntoResponse;
    error.into_response().status()
}
impl Backend {
    async fn identity(
        &self,
        header: &HeaderValue,
        transport: &Request<()>,
    ) -> Result<AuthorizationInfo, Failure> {
        let raw = header.to_str().map_err(|_| StatusCode::UNAUTHORIZED)?;
        if raw.len() > 4096 || !raw.starts_with("MediaBrowser ") {
            return Err(StatusCode::UNAUTHORIZED);
        }
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(header::AUTHORIZATION, header.clone());
        let client = self
            .state
            .client_address_of(transport.extensions(), transport.headers());
        let context = auth_request_context(&headers, None, Some(client.to_string()));
        // Do not reuse an AuthorizationInfo extension captured at admission.
        let mut auth = self
            .state
            .auth_service()
            .authenticate(&context)
            .await
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
        if !auth.is_authenticated
            || auth.is_api_key
            || auth.user_id().is_nil()
            || !auth.has_token()
            || auth
                .device_id
                .as_deref()
                .is_none_or(|d| d.is_empty() || d.len() > 128)
        {
            return Err(StatusCode::UNAUTHORIZED);
        }
        // Ordinary authentication may serve a valid cached projection. This
        // capability also checks the current persisted session binding, so an
        // independent writer's token revoke or reassignment takes effect now.
        let devices = self
            .state
            .devices
            .get_devices(&ferrofin_traits::devices::DeviceQuery {
                device_id: auth.device_id.clone(),
                user_id: Some(auth.user_id()),
                limit: Some(16),
                ..Default::default()
            })
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        if !devices.items.iter().any(|device| {
            auth.token
                .as_ref()
                .is_some_and(|token| token.expose() == device.access_token)
                && Uuid::parse_str(&device.user_id).ok() == Some(auth.user_id())
                && auth.device_id.as_deref() == Some(device.device_id.as_str())
        }) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        // Re-read the user and Permissions projection, even if the auth service
        // served an otherwise valid session from its invalidation-aware cache.
        let user = self
            .state
            .users
            .get_user_by_id(auth.user_id())
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .ok_or(StatusCode::UNAUTHORIZED)?;
        if !self
            .state
            .devices
            .can_access_device(&user, auth.device_id.as_deref().unwrap())
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        {
            return Err(StatusCode::FORBIDDEN);
        }
        auth.user_policy = super::users::user_policy(&self.state, &user)
            .await
            .map_err(failure)?
            .map(Arc::new);
        auth.user = Some(Arc::new(user));
        let policy = user_authorization(&auth, client, &self.state, true)
            .await
            .map_err(failure)?;
        if !policy.enable_media_playback {
            return Err(StatusCode::FORBIDDEN);
        }
        Ok(auth)
    }
    async fn pin_view(
        &self,
        auth: &AuthorizationInfo,
        transport: &Request<()>,
        item: Uuid,
        kind: MediaKind,
    ) -> Result<(Arc<PlaybackView>, String), Failure> {
        let visible = super::items::require_visible_item(&self.state, item, auth.user.as_deref())
            .await
            .map_err(failure)?;
        if (kind == MediaKind::Audio
            && ferrofin_model::data::BaseItemKind::from_stored_type_name(&visible.type_)
                != Some(ferrofin_model::data::BaseItemKind::Audio))
            || (kind == MediaKind::Video && !super::videos::is_video(&visible))
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        let ordinary = PlaybackView::new(self.state.clone());
        ordinary.local_source(auth, transport, item, None).await?;
        let selected = self
            .state
            .media_sources
            .get_stream_media_source(item, None, None)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let selected_id = selected
            .id
            .clone()
            .filter(|id| !id.is_empty())
            .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;
        let local = super::streaming::static_source(&self.state, item, Some(&selected_id), None)
            .await
            .map_err(failure)?;
        // No pre-pin PlaybackInfo/preparation: unsupported input fails before
        // negotiation can allocate a session or start source-dependent work.
        let scoped = self
            .pins
            .pin(&self.state, item, &selected_id, &local.path, auth.user_id())
            .await?;
        if !scoped.has_scoped_media() {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        let playback = Arc::new(PlaybackView::new(scoped));
        if kind == MediaKind::Video {
            let captured = playback
                .state
                .media_sources
                .get_stream_media_source(item, Some(&selected_id), None)
                .await
                .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
            if captured.video_stream().is_none_or(|stream| {
                stream.video_range_type() != ferrofin_model::data::VideoRangeType::Sdr
            }) {
                return Err(StatusCode::UNPROCESSABLE_ENTITY);
            }
        }
        Ok((playback, selected_id))
    }
}
impl PlaybackView {
    async fn require_operations(&self, operations: RequiredOperations) -> Result<(), Failure> {
        let policy = self
            .state
            .media_sources
            .current_pinned_policy()
            .await
            .map_err(|error| failure(error.into()))?;
        if !policy.enable_media_playback
            || (operations.audio_transcoding && !policy.playback.audio_transcoding)
            || (operations.video_transcoding && !policy.playback.video_transcoding)
        {
            return Err(StatusCode::FORBIDDEN);
        }
        Ok(())
    }
    async fn plan(
        &self,
        transport: &Request<()>,
        auth: AuthorizationInfo,
        item: Uuid,
        selected_id: &str,
        request: &LeaseRequest,
    ) -> Result<Value, Failure> {
        // Server-owned formats are hints only; existing planner permissions still apply.
        let profile = json!({"Name":"Bluefin TV safe v1","MaxStreamingBitrate":request.playback.max_bitrate,
            "DirectPlayProfiles":[],"TranscodingProfiles":[
                {"Type":"Video","Container":"ts","Protocol":"hls","VideoCodec":"h264","AudioCodec":"aac","MaxAudioChannels":"2","Context":"Streaming","EnableSubtitlesInManifest":false,"MinSegments":2,"SegmentLength":6},
                {"Type":"Audio","Container":"mp3","Protocol":"http","AudioCodec":"mp3","MaxAudioChannels":"2","Context":"Streaming"}
            ],"SubtitleProfiles":[],"CodecProfiles":[]});
        let body = json!({"DeviceProfile":profile,"MediaSourceId":selected_id,"MaxStreamingBitrate":request.playback.max_bitrate,"MaxAudioChannels":2,
            "StartTimeTicks":request.playback.start_ticks,"AudioStreamIndex":request.playback.audio_stream_index,
            "SubtitleStreamIndex":request.playback.subtitle_stream_index,"EnableDirectPlay":false,"EnableDirectStream":false,
            "EnableTranscoding":true,"AllowVideoStreamCopy":false,"AllowAudioStreamCopy":false,
            "AutoOpenLiveStream":false,"AlwaysBurnInSubtitleWhenTranscoding":true});
        let plan_request = self.request(
            transport,
            auth,
            Method::POST,
            &format!("/Items/{}/PlaybackInfo", item.simple()),
            Body::from(body.to_string()),
        )?;
        let response = self
            .media
            .clone()
            .oneshot(plan_request)
            .await
            .map_err(|_| StatusCode::BAD_GATEWAY)?;
        if !response.status().is_success() {
            return Err(response.status());
        }
        let bytes = to_bytes(response.into_body(), 512 * 1024)
            .await
            .map_err(|_| StatusCode::BAD_GATEWAY)?;
        serde_json::from_slice(&bytes).map_err(|_| StatusCode::BAD_GATEWAY)
    }
    async fn local_source(
        &self,
        auth: &AuthorizationInfo,
        transport: &Request<()>,
        item: Uuid,
        source: Option<&str>,
    ) -> Result<(), Failure> {
        super::items::require_visible_item(&self.state, item, auth.user.as_deref())
            .await
            .map_err(failure)?;
        super::streaming::require_streamable_item(
            &self.state,
            auth,
            transport.extensions(),
            transport.headers(),
            item,
            source,
        )
        .await
        .map_err(failure)?;
        let source = super::streaming::static_source(&self.state, item, source, None)
            .await
            .map_err(failure)?;
        if source.protocol != MediaProtocol::File
            || source.is_live_stream
            || source.is_infinite_stream
            || !std::path::Path::new(&source.path).is_absolute()
        {
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
        Ok(())
    }
    fn request(
        &self,
        transport: &Request<()>,
        auth: AuthorizationInfo,
        method: Method,
        uri: &str,
        body: Body,
    ) -> Result<axum::extract::Request, Failure> {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .body(body)
            .map_err(|_| StatusCode::BAD_GATEWAY)?;
        // Router-private path parameters belong to the capability route, not
        // the media route. Copying all extensions makes axum merge grant and
        // resource parameters into the inner handler's item-id extractor.
        if let Some(peer) = transport
            .extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        {
            request.extensions_mut().insert(*peer);
        }
        request.extensions_mut().insert(auth);
        let client = self
            .state
            .client_address_of(transport.extensions(), transport.headers());
        request
            .extensions_mut()
            .insert(ResolvedClientAddress(client));
        // The address is already resolved using the actual peer and trusted
        // proxy chain. No forwarded credentials or arbitrary upstream headers.
        for name in [header::RANGE, header::IF_RANGE] {
            if let Some(value) = transport.headers().get(&name) {
                request.headers_mut().insert(name, value.clone());
            }
        }
        request.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        Ok(request)
    }
}
#[async_trait]
impl PlaybackBackend for Backend {
    type Session = PlaybackContext;
    async fn admit(
        &self,
        transport: &Request<()>,
        request: LeaseRequest,
    ) -> Result<Admission<PlaybackContext>, Failure> {
        if transport
            .headers()
            .get_all(header::AUTHORIZATION)
            .iter()
            .count()
            != 1
        {
            return Err(StatusCode::UNAUTHORIZED);
        }
        let authorization = transport
            .headers()
            .get(header::AUTHORIZATION)
            .cloned()
            .ok_or(StatusCode::UNAUTHORIZED)?;
        let auth = self.identity(&authorization, transport).await?;
        let item = Uuid::parse_str(&request.item_id).map_err(|_| StatusCode::BAD_REQUEST)?;
        let (playback, selected_id) = self
            .pin_view(&auth, transport, item, request.playback.kind)
            .await?;
        // Pinning refreshed the policy projection against independent writers.
        let auth = self.identity(&authorization, transport).await?;
        // Verify again after acquiring the guard, which must compare-and-pin the
        // exact local generation, not just promise future best-effort checks.
        playback
            .local_source(&auth, transport, item, Some(&selected_id))
            .await?;
        let captured = playback
            .state
            .media_sources
            .get_stream_media_source(item, Some(&selected_id), None)
            .await
            .map_err(|error| failure(error.into()))?;
        let operations = RequiredOperations {
            audio_transcoding: captured.media_streams.iter().any(|stream| {
                stream.stream_type == ferrofin_model::entities::MediaStreamType::Audio
            }),
            video_transcoding: request.playback.kind == MediaKind::Video,
        };
        playback.require_operations(operations).await?;
        let plan = playback
            .plan(transport, auth.clone(), item, &selected_id, &request)
            .await?;
        let play_session = plan["PlaySessionId"]
            .as_str()
            .filter(|s| s.len() == 32 && s.bytes().all(|c| c.is_ascii_hexdigit()))
            .ok_or(StatusCode::BAD_GATEWAY)?
            .to_owned();
        let source = plan["MediaSources"]
            .as_array()
            .and_then(|s| s.iter().find(|s| s["TranscodingUrl"].is_string()))
            .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;
        let source_id = source["Id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 128)
            .ok_or(StatusCode::BAD_GATEWAY)?
            .to_owned();
        playback
            .local_source(&auth, transport, item, Some(&source_id))
            .await?;
        if source_id != selected_id {
            return Err(StatusCode::BAD_GATEWAY);
        }
        let job_device = Uuid::new_v4().simple().to_string();
        let root_uri = canonical_route(
            source["TranscodingUrl"].as_str().unwrap(),
            &request.item_id,
            &play_session,
        )?;
        let root_uri = format!("{root_uri}&deviceid={job_device}");
        let device = auth.device_id.clone().unwrap();
        let viewer = auth.user_id();
        Ok(Admission {
            session: PlaybackContext {
                authorization,
                viewer,
                device: device.clone(),
                item,
                source: source_id,
                play_session: play_session.clone(),
                client: self
                    .state
                    .client_address_of(transport.extensions(), transport.headers()),
                jobs: Arc::new(PlaybackJobs {
                    hls: Arc::clone(&playback.state.hls),
                    viewer,
                    device: job_device,
                    play_session: play_session.clone(),
                    stopped: std::sync::atomic::AtomicBool::new(false),
                    dispatch: Arc::new(tokio::sync::RwLock::new(())),
                }),
                playback,
                operations,
            },
            viewer: viewer.to_string(),
            device,
            item_id: request.item_id,
            play_session_id: play_session,
            root_uri,
        })
    }
    async fn reauthorize(
        &self,
        session: &PlaybackContext,
        transport: &Request<()>,
    ) -> Result<(), Failure> {
        // Scoped resolution refreshes the actual user/library policy owner
        // before ordinary authentication/device policy can consult its cache.
        session
            .playback
            .require_operations(session.operations)
            .await?;
        let auth = self.identity(&session.authorization, transport).await?;
        // The existing HTTPS media URL is a scoped bearer capability. Iroh
        // additionally requires ordinary viewer Authorization on every request;
        // that caller must be the lease's actual token/device/account, rather
        // than borrowing the stored session's authority through the capability.
        if transport
            .extensions()
            .get::<bluefin_iroh_server::DirectPeer>()
            .is_some()
        {
            let header = transport
                .headers()
                .get(header::AUTHORIZATION)
                .ok_or(StatusCode::UNAUTHORIZED)?;
            let caller = self.identity(header, transport).await?;
            if caller.user_id() != session.viewer
                || caller.device_id.as_deref() != Some(&session.device)
                || caller
                    .token
                    .as_ref()
                    .map(ferrofin_model::secret::Secret::expose)
                    != auth
                        .token
                        .as_ref()
                        .map(ferrofin_model::secret::Secret::expose)
            {
                return Err(StatusCode::UNAUTHORIZED);
            }
        }
        if auth.user_id() != session.viewer || auth.device_id.as_deref() != Some(&session.device) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        if self
            .state
            .client_address_of(transport.extensions(), transport.headers())
            != session.client
        {
            return Err(StatusCode::FORBIDDEN);
        }
        session
            .playback
            .local_source(&auth, transport, session.item, Some(&session.source))
            .await
    }
    async fn serve(
        &self,
        session: &PlaybackContext,
        transport: &Request<()>,
        uri: &str,
        method: Method,
    ) -> Result<Response, Failure> {
        bluefin_tv_playback::server::validate_uri(
            uri,
            &session.item.simple().to_string(),
            &session.play_session,
        )?;
        // Authenticate once more at dispatch, so no captured grant becomes an
        // anonymous call to legacy stream routes which permit anonymous inputs.
        let auth = self.identity(&session.authorization, transport).await?;
        if auth.user_id() != session.viewer || auth.device_id.as_deref() != Some(&session.device) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        self.reauthorize(session, transport).await?;
        let request = session
            .playback
            .request(transport, auth, method, uri, Body::empty())?;
        let playback = Arc::clone(&session.playback);
        let jobs = Arc::clone(&session.jobs);
        let culture = ferrofin_util::current_culture::capture();
        let ui = ferrofin_util::current_culture::ambient_ui_culture();
        // Keep queued filesystem opens and planner work owned if the caller
        // disappears or the outer deadline fires. Retirement drains this fence.
        tokio::spawn(ferrofin_util::current_culture::scope_with_ui(
            culture,
            ui,
            async move {
                let _dispatch = Arc::clone(&jobs.dispatch).read_owned().await;
                if jobs.stopped.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err(StatusCode::NOT_FOUND);
                }
                playback
                    .media
                    .clone()
                    .oneshot(request)
                    .await
                    .map_err(|_| StatusCode::BAD_GATEWAY)
            },
        ))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    }
    async fn release(&self, session: &PlaybackContext) {
        let _ = session.jobs.stop().await;
    }
}
fn canonical_route(raw: &str, item: &str, session: &str) -> Result<String, Failure> {
    if !raw.starts_with('/') || raw.starts_with("//") || raw.contains(['\\', '#']) {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let (path, query) = raw.split_once('?').ok_or(StatusCode::BAD_GATEWAY)?;
    if path.contains('%') || path.split('/').any(|p| p == ".." || p == ".") {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let mut fields = std::collections::BTreeMap::new();
    for (name, value) in url::form_urlencoded::parse(query.as_bytes()) {
        let name = name.to_ascii_lowercase();
        if matches!(name.as_str(), "apikey" | "api_key" | "deviceid") {
            continue;
        }
        if !name
            .bytes()
            .chain(value.bytes())
            .all(|b| b.is_ascii_alphanumeric() || b"-_,.".contains(&b))
            || fields.insert(name, value.to_string()).is_some()
        {
            return Err(StatusCode::BAD_GATEWAY);
        }
    }
    if fields.get("playsessionid").map(String::as_str) != Some(session) {
        return Err(StatusCode::BAD_GATEWAY);
    }
    // Suppress auxiliary manifests that could carry account-token URLs or
    // renegotiate a different resource/codec outside this fixed grant.
    for (key, value) in [
        ("enabletrickplay", "false"),
        ("enablesubtitlesinmanifest", "false"),
        ("enableadaptivebitratestreaming", "false"),
        ("allowvideostreamcopy", "false"),
        ("allowaudiostreamcopy", "false"),
        ("maxvideobitdepth", "8"),
        ("h264-rangetype", "SDR"),
    ] {
        fields.insert(key.into(), value.into());
    }
    // StreamInfo emits lowercase controller names; the private router uses
    // their registered spelling and has no outer path-normalization layer.
    let path = if let Some(tail) = path.strip_prefix("/videos/") {
        format!("/Videos/{tail}")
    } else if let Some(tail) = path.strip_prefix("/audio/") {
        format!("/Audio/{tail}")
    } else {
        path.to_owned()
    };
    let path = path.replace(
        &Uuid::parse_str(item)
            .map_err(|_| StatusCode::BAD_GATEWAY)?
            .to_string(),
        item,
    );
    let route = format!(
        "{path}?{}",
        fields
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&")
    );
    bluefin_tv_playback::server::validate_uri(&route, item, session)?;
    Ok(route)
}
/// Registers the capability routes with the configured source binding factory,
/// before the ordinary router's IP/auth/CORS layers.
pub fn register(
    router: Router<AppState>,
    state: AppState,
    pins: Arc<dyn LocalSourcePins>,
) -> Router<AppState> {
    router.merge(bluefin_tv_playback::server::routes(Backend { state, pins }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn planner_urls_are_local_fixed_and_credential_free() {
        let item = "00000000000000000000000000000001";
        let session = "00000000000000000000000000000002";
        let uri = format!(
            "/Videos/{item}/master.m3u8?ApiKey=secret&PlaySessionId={session}&VideoCodec=h264%2Caac"
        );
        let clean = canonical_route(&uri, item, session).unwrap();
        assert!(!clean.contains("secret"));
        assert!(!clean.contains("ApiKey"));
        assert!(clean.contains("videocodec=h264,aac"));
        for path in [
            "https://example.com/stream",
            "//example.com/stream",
            "/Videos/%2e%2e/stream",
            "/Videos/../stream",
        ] {
            assert!(canonical_route(path, item, session).is_err());
        }
    }
}

#[cfg(test)]
mod permission_router_tests {
    //! Real adapter authentication/user_authorization/visibility code through
    //! the real lease router. Synthetic admission avoids invoking ffmpeg or
    //! adding production pins. Installed planning remains a parent gate.
    use super::*;
    use crate::test_support::*;
    use axum::response::IntoResponse;
    use ferrofin_db::entities::users::UserEntity;
    use ferrofin_model::{
        configuration::UserConfiguration,
        dto::{NameIdPair, UserDto},
        users::UserPolicy,
    };
    use ferrofin_traits::{
        error::ServiceError,
        library::UserManager,
        net::{AuthService, RequestContext},
    };
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };
    const ID: Uuid = Uuid::from_u128(1);
    struct MutableUsers(Arc<Mutex<UserPolicy>>);
    #[async_trait]
    impl UserManager for MutableUsers {
        async fn get_user_dto(
            &self,
            user: &UserEntity,
            _server_id: Option<String>,
        ) -> Result<UserDto, ServiceError> {
            Ok(UserDto {
                id: Uuid::parse_str(&user.id).unwrap_or_default(),
                name: Some(user.username.clone()),
                policy: Some(self.0.lock().unwrap().clone()),
                ..UserDto::default()
            })
        }
        async fn get_users(&self) -> Result<Vec<UserEntity>, ServiceError> {
            FakeUsers.get_users().await
        }
        async fn get_user_ids(&self) -> Result<Vec<Uuid>, ServiceError> {
            FakeUsers.get_user_ids().await
        }
        async fn initialize(&self) -> Result<(), ServiceError> {
            FakeUsers.initialize().await
        }
        async fn get_user_by_id(&self, id: Uuid) -> Result<Option<UserEntity>, ServiceError> {
            Ok(Some(fake_user_entity(id, "Synthetic viewer")))
        }
        async fn get_first_user(&self) -> Result<Option<UserEntity>, ServiceError> {
            FakeUsers.get_first_user().await
        }
        async fn get_user_by_name(&self, name: &str) -> Result<Option<UserEntity>, ServiceError> {
            FakeUsers.get_user_by_name(name).await
        }
        async fn rename_user(
            &self,
            user_id: Uuid,
            old_name: &str,
            new_name: &str,
        ) -> Result<(), ServiceError> {
            FakeUsers.rename_user(user_id, old_name, new_name).await
        }
        async fn update_user(&self, user: &UserEntity) -> Result<(), ServiceError> {
            FakeUsers.update_user(user).await
        }
        async fn create_user(&self, name: &str) -> Result<UserEntity, ServiceError> {
            FakeUsers.create_user(name).await
        }
        async fn delete_user(&self, user_id: Uuid) -> Result<(), ServiceError> {
            FakeUsers.delete_user(user_id).await
        }
        async fn reset_password(&self, user_id: Uuid) -> Result<(), ServiceError> {
            FakeUsers.reset_password(user_id).await
        }
        async fn change_password(
            &self,
            user_id: Uuid,
            new_password: &str,
        ) -> Result<(), ServiceError> {
            FakeUsers.change_password(user_id, new_password).await
        }
        async fn authenticate_user(
            &self,
            username: &str,
            password: &str,
            remote_endpoint: &str,
            is_user_session: bool,
        ) -> Result<Option<UserEntity>, ServiceError> {
            FakeUsers
                .authenticate_user(username, password, remote_endpoint, is_user_session)
                .await
        }
        async fn get_authentication_providers(&self) -> Result<Vec<NameIdPair>, ServiceError> {
            FakeUsers.get_authentication_providers().await
        }
        async fn get_password_reset_providers(&self) -> Result<Vec<NameIdPair>, ServiceError> {
            FakeUsers.get_password_reset_providers().await
        }
        async fn update_configuration(
            &self,
            user_id: Uuid,
            config: &UserConfiguration,
        ) -> Result<(), ServiceError> {
            FakeUsers.update_configuration(user_id, config).await
        }
        async fn update_policy(
            &self,
            user_id: Uuid,
            policy: &UserPolicy,
        ) -> Result<(), ServiceError> {
            FakeUsers.update_policy(user_id, policy).await
        }
        async fn clear_profile_image(&self, user: &UserEntity) -> Result<(), ServiceError> {
            FakeUsers.clear_profile_image(user).await
        }
        async fn save_profile_image(
            &self,
            user: &UserEntity,
            content: &[u8],
            mime_type: &str,
            extension: &str,
        ) -> Result<(), ServiceError> {
            FakeUsers
                .save_profile_image(user, content, mime_type, extension)
                .await
        }
        async fn get_profile_image(
            &self,
            user_id: Uuid,
        ) -> Result<Option<ferrofin_traits::options::ItemImageInfo>, ServiceError> {
            FakeUsers.get_profile_image(user_id).await
        }
    }

    struct SessionAuth(Arc<AtomicBool>);
    #[async_trait]
    impl AuthService for SessionAuth {
        async fn authenticate(
            &self,
            context: &RequestContext,
        ) -> Result<AuthorizationInfo, ServiceError> {
            let allowed = self.0.load(Ordering::SeqCst)
                && context
                    .header("Authorization")
                    .is_some_and(|h| h.contains("Token=\"fixture\""));
            Ok(AuthorizationInfo {
                is_authenticated: allowed,
                user: Some(Arc::new(fake_user_entity(ID, "Synthetic viewer"))),
                device_id: Some("fixture-device".into()),
                token: Some("fixture".into()),
                ..Default::default()
            })
        }
    }
    struct HiddenLibrary;
    use ferrofin_db::entities::base_items::{BaseItemEntity, PeopleEntity};
    use ferrofin_model::{
        data::CollectionType,
        dto::ItemCounts,
        querying::{QueryFiltersLegacy, QueryResult},
    };
    use ferrofin_traits::{
        library::LibraryManager,
        options::{DeleteOptions, InternalItemsQuery, InternalPeopleQuery},
        persistence::ItemWithCounts,
    };
    #[async_trait]
    impl LibraryManager for HiddenLibrary {
        async fn get_item_by_id_for_user(
            &self,
            _id: Uuid,
            _user: Option<&UserEntity>,
        ) -> Result<Option<BaseItemEntity>, ServiceError> {
            Ok(None)
        }
        async fn queue_library_scan(&self) -> Result<(), ServiceError> {
            unimplemented!("unused fixture")
        }
        async fn get_item_by_id(&self, _id: Uuid) -> Result<Option<BaseItemEntity>, ServiceError> {
            unimplemented!("fake")
        }
        async fn query_items(
            &self,
            _query: &InternalItemsQuery,
        ) -> Result<QueryResult<BaseItemEntity>, ServiceError> {
            unimplemented!("fake")
        }
        async fn get_item_ids(
            &self,
            _query: &InternalItemsQuery,
        ) -> Result<Vec<Uuid>, ServiceError> {
            unimplemented!("fake")
        }
        async fn get_item_list(
            &self,
            _query: &InternalItemsQuery,
        ) -> Result<Vec<BaseItemEntity>, ServiceError> {
            unimplemented!("fake")
        }
        async fn get_latest_item_list(
            &self,
            _query: &InternalItemsQuery,
            _collection_type: CollectionType,
        ) -> Result<Vec<BaseItemEntity>, ServiceError> {
            unimplemented!("fake")
        }
        async fn create_items(
            &self,
            _items: &[BaseItemEntity],
            _parent_id: Option<Uuid>,
        ) -> Result<(), ServiceError> {
            unimplemented!("fake")
        }
        async fn update_items(
            &self,
            _items: &[BaseItemEntity],
            _parent_id: Option<Uuid>,
        ) -> Result<(), ServiceError> {
            unimplemented!("fake")
        }
        async fn delete_item(
            &self,
            _id: Uuid,
            _options: &DeleteOptions,
        ) -> Result<(), ServiceError> {
            unimplemented!("fake")
        }
        async fn get_people(
            &self,
            _query: &InternalPeopleQuery,
        ) -> Result<Vec<PeopleEntity>, ServiceError> {
            unimplemented!("fake")
        }
        async fn get_people_names(
            &self,
            _query: &InternalPeopleQuery,
        ) -> Result<Vec<String>, ServiceError> {
            unimplemented!("fake")
        }
        async fn get_count(&self, _query: &InternalItemsQuery) -> Result<i32, ServiceError> {
            unimplemented!("fake")
        }
        async fn get_item_counts(
            &self,
            _query: &InternalItemsQuery,
        ) -> Result<ItemCounts, ServiceError> {
            unimplemented!("fake")
        }
        async fn get_genres(
            &self,
            _query: &InternalItemsQuery,
        ) -> Result<QueryResult<ItemWithCounts>, ServiceError> {
            unimplemented!("fake")
        }
        async fn get_studios(
            &self,
            _query: &InternalItemsQuery,
        ) -> Result<QueryResult<ItemWithCounts>, ServiceError> {
            unimplemented!("fake")
        }
        async fn get_artists(
            &self,
            _query: &InternalItemsQuery,
        ) -> Result<QueryResult<ItemWithCounts>, ServiceError> {
            unimplemented!("fake")
        }
        async fn get_music_genres(
            &self,
            _query: &InternalItemsQuery,
        ) -> Result<QueryResult<ItemWithCounts>, ServiceError> {
            unimplemented!("fake")
        }
        async fn get_album_artists(
            &self,
            _query: &InternalItemsQuery,
        ) -> Result<QueryResult<ItemWithCounts>, ServiceError> {
            unimplemented!("fake")
        }
        async fn get_query_filters_legacy(
            &self,
            _query: &InternalItemsQuery,
        ) -> Result<QueryFiltersLegacy, ServiceError> {
            unimplemented!("fake")
        }
        async fn get_media_stream_languages(
            &self,
            _stream_type: ferrofin_model::entities::MediaStreamType,
            _query: &InternalItemsQuery,
        ) -> Result<Vec<String>, ServiceError> {
            unimplemented!("fake")
        }
    }

    use ferrofin_db::entities::security::{DeviceEntity, DeviceOptionsEntity};
    use ferrofin_model::{
        devices::DeviceInfo,
        dto::{ClientCapabilitiesDto, DeviceInfoDto},
        session::ClientCapabilities,
    };
    use ferrofin_traits::devices::{DeviceManager, DeviceQuery};
    struct SessionDevices;
    #[async_trait]
    impl DeviceManager for SessionDevices {
        async fn create_device(
            &self,
            _device: &DeviceEntity,
        ) -> Result<DeviceEntity, ServiceError> {
            unimplemented!("fake")
        }
        async fn save_capabilities(
            &self,
            _device_id: &str,
            _capabilities: &ClientCapabilities,
        ) -> Result<(), ServiceError> {
            unimplemented!("fake")
        }
        async fn get_capabilities(
            &self,
            _device_id: Option<&str>,
        ) -> Result<ClientCapabilities, ServiceError> {
            Ok(ClientCapabilities::default())
        }
        async fn get_device(&self, _id: &str) -> Result<Option<DeviceInfoDto>, ServiceError> {
            unimplemented!("fake")
        }
        async fn get_devices(
            &self,
            _query: &DeviceQuery,
        ) -> Result<QueryResult<DeviceEntity>, ServiceError> {
            let now = chrono::Utc::now();
            Ok(QueryResult::new(
                Some(0),
                Some(1),
                vec![DeviceEntity {
                    id: 1,
                    access_token: "fixture".into(),
                    app_name: "fixture".into(),
                    app_version: "1".into(),
                    date_created: now,
                    date_last_activity: now,
                    date_modified: now,
                    device_id: "fixture-device".into(),
                    device_name: "fixture".into(),
                    is_active: true,
                    user_id: ID.to_string(),
                }],
            ))
        }
        async fn get_device_infos(
            &self,
            _query: &DeviceQuery,
        ) -> Result<QueryResult<DeviceInfo>, ServiceError> {
            unimplemented!("fake")
        }
        async fn get_devices_for_user(
            &self,
            _user_id: Option<Uuid>,
        ) -> Result<QueryResult<DeviceInfoDto>, ServiceError> {
            unimplemented!("fake")
        }
        async fn delete_device(&self, _device: &DeviceEntity) -> Result<(), ServiceError> {
            unimplemented!("fake")
        }
        async fn update_device(&self, _device: &DeviceEntity) -> Result<(), ServiceError> {
            unimplemented!("fake")
        }
        async fn can_access_device(
            &self,
            _user: &UserEntity,
            _device_id: &str,
        ) -> Result<bool, ServiceError> {
            Ok(true)
        }
        async fn update_device_options(
            &self,
            _device_id: &str,
            _device_name: Option<&str>,
        ) -> Result<(), ServiceError> {
            unimplemented!("fake")
        }
        async fn get_device_options(
            &self,
            _device_id: &str,
        ) -> Result<Option<DeviceOptionsEntity>, ServiceError> {
            unimplemented!("fake")
        }
        async fn to_client_capabilities_dto(
            &self,
            _capabilities: &ClientCapabilities,
        ) -> Result<ClientCapabilitiesDto, ServiceError> {
            unimplemented!("fake")
        }
    }
    struct NoPins;
    #[async_trait]
    impl LocalSourcePins for NoPins {
        async fn pin(
            &self,
            _state: &AppState,
            _item: Uuid,
            _source: &str,
            _path: &str,
            _viewer: Uuid,
        ) -> Result<AppState, Failure> {
            Err(StatusCode::NOT_IMPLEMENTED)
        }
    }
    struct PolicyBackend(Backend);
    #[async_trait]
    impl PlaybackBackend for PolicyBackend {
        type Session = PlaybackContext;
        async fn admit(
            &self,
            t: &Request<()>,
            r: LeaseRequest,
        ) -> Result<Admission<PlaybackContext>, Failure> {
            let authorization = t
                .headers()
                .get(header::AUTHORIZATION)
                .cloned()
                .ok_or(StatusCode::UNAUTHORIZED)?;
            let auth = self.0.identity(&authorization, t).await?;
            Ok(Admission {
                session: PlaybackContext {
                    authorization,
                    viewer: auth.user_id(),
                    device: "fixture-device".into(),
                    item: ID,
                    source: "fixture-source".into(),
                    play_session: "fixture-session".into(),
                    operations: RequiredOperations {
                        audio_transcoding: true,
                        video_transcoding: false,
                    },
                    client: self.0.state.client_address_of(t.extensions(), t.headers()),
                    playback: Arc::new(PlaybackView::new(self.0.state.clone())),
                    jobs: Arc::new(PlaybackJobs {
                        hls: self.0.state.hls.clone(),
                        viewer: ID,
                        device: "fixture-device".into(),
                        play_session: "fixture-session".into(),
                        stopped: std::sync::atomic::AtomicBool::new(true),
                        dispatch: Arc::new(tokio::sync::RwLock::new(())),
                    }),
                },
                viewer: ID.to_string(),
                device: "fixture-device".into(),
                item_id: r.item_id,
                play_session_id: "fixture-session".into(),
                root_uri: format!(
                    "/Videos/{}/stream.mp4?playsessionid=fixture-session",
                    ID.simple()
                ),
            })
        }
        async fn reauthorize(&self, s: &PlaybackContext, t: &Request<()>) -> Result<(), Failure> {
            // This fixture isolates the ordinary auth/policy/visibility seam;
            // real opened-input refresh is covered by the server composition tests.
            let auth = self.0.identity(&s.authorization, t).await?;
            s.playback
                .local_source(&auth, t, s.item, Some(&s.source))
                .await
        }
        async fn serve(
            &self,
            _: &PlaybackContext,
            _: &Request<()>,
            _: &str,
            _: Method,
        ) -> Result<Response, Failure> {
            Ok(StatusCode::OK.into_response())
        }
        async fn release(&self, _: &PlaybackContext) {}
    }
    fn state(policy: Arc<Mutex<UserPolicy>>, active: Arc<AtomicBool>) -> AppState {
        AppState::new(
            Arc::new(HiddenLibrary),
            Arc::new(MutableUsers(policy)),
            Arc::new(FakeUserViews),
            Arc::new(FakeUserData),
            Arc::new(FakeMediaSources),
            Arc::new(FakeSessions),
            Arc::new(FakeSystem),
            Arc::new(FakeAppHost),
            Arc::new(FakeConfig),
            Arc::new(FakeProviders),
            Arc::new(FakeMusic),
            Arc::new(FakeSimilarItems),
            Arc::new(FakeSearch),
            Arc::new(FakeDto),
            Arc::new(FakeAuthContext),
            Arc::new(SessionAuth(active)),
            Arc::new(FakeQuickConnect),
            Arc::new(FakePlaylists),
            Arc::new(FakeCollections),
            Arc::new(FakeTvSeries),
            Arc::new(FakeSubtitles),
            Arc::new(FakeLyrics),
            Arc::new(FakeMediaSegments),
            Arc::new(FakeTrickplay),
            Arc::new(SessionDevices),
            Arc::new(FakeClientEventLogger),
            Arc::new(FakeApiKeys),
            Arc::new(FakeLocalization),
            Arc::new(FakeDisplayPreferences),
            Arc::new(FakeActivity),
            Arc::new(FakeFileSystem),
            Arc::new(FakeTasks),
        )
    }
    fn req(method: Method, path: &str, body: Body) -> axum::extract::Request {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(
                header::AUTHORIZATION,
                "MediaBrowser Token=\"fixture\", DeviceId=\"fixture-device\"",
            )
            .body(body)
            .unwrap();
        request.extensions_mut().insert(axum::extract::ConnectInfo(
            "203.0.113.5:4444".parse::<std::net::SocketAddr>().unwrap(),
        ));
        request
    }
    #[tokio::test]
    async fn captured_grant_uses_fresh_actual_auth_policy_and_visibility_for_get_and_head() {
        for method in [Method::GET, Method::HEAD] {
            for mode in 0..6 {
                let policy = Arc::new(Mutex::new(UserPolicy {
                    enable_remote_access: true,
                    enable_media_playback: true,
                    ..Default::default()
                }));
                let active = Arc::new(AtomicBool::new(true));
                let state = state(policy.clone(), active.clone());
                let backend = PolicyBackend(Backend {
                    state,
                    pins: Arc::new(NoPins),
                });
                let app: Router = bluefin_tv_playback::server::routes(backend);
                let response=app.clone().oneshot(req(Method::POST,"/Bluefin/TV/v1/leases",Body::from(json!({"version":1,"itemId":ID.simple().to_string(),"playback":{"kind":"video","maxBitrate":8_000_000}}).to_string()))).await.unwrap();
                assert_eq!(response.status(), StatusCode::CREATED);
                let lease: Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap())
                        .unwrap();
                match mode {
                    0 => active.store(false, Ordering::SeqCst),
                    1 => policy.lock().unwrap().is_disabled = true,
                    2 => policy.lock().unwrap().enable_remote_access = false,
                    3 => policy.lock().unwrap().enable_media_playback = false,
                    4 => {
                        policy.lock().unwrap().access_schedules =
                            vec![ferrofin_model::users::AccessSchedule {
                                day_of_week: ferrofin_model::users::DynamicDayOfWeek::Everyday,
                                start_hour: 25.0,
                                end_hour: 26.0,
                                ..Default::default()
                            }];
                    }
                    _ => (), // Source visibility is now denied by the real library seam.
                }
                let response = app
                    .oneshot(req(
                        method.clone(),
                        lease["mediaUrl"].as_str().unwrap(),
                        Body::empty(),
                    ))
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    match mode {
                        0 | 1 => StatusCode::UNAUTHORIZED,
                        2..=4 => StatusCode::FORBIDDEN,
                        _ => StatusCode::NOT_FOUND,
                    }
                );
            }
        }
    }
}
