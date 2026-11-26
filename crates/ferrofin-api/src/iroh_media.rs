//! Additive Iroh admission over the ordinary production router/handlers.
//! A direct peer is supplied by the transport, never by an HTTP header.
use crate::{
    ApiError, AppState,
    auth::{auth_request_context, user_authorization},
    handlers::streaming::{require_streamable_item, static_source},
};
use axum::{
    Router,
    body::Body,
    extract::{ConnectInfo, Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use bluefin_iroh_server::{Authority, DirectPeer, ResponseAuthority};
use ferrofin_model::media_info::MediaProtocol;
use ferrofin_traits::{
    library::PlaybackPermissions, net::RequestContext, options::AuthorizationInfo,
};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

/// Wraps the same fully composed API router used by TCP. Standard local media
/// uses a fully composed source-bound view of that router, with the existing
/// production source/encoder factory. Existing TV leases keep their own router.
pub fn router(state: AppState, app: Router) -> Router {
    app.layer(axum::middleware::from_fn_with_state(state, admit))
}

struct Current {
    state: AppState,
    context: RequestContext,
    peer: DirectPeer,
    user: Uuid,
    device: String,
    pinned: Option<AppState>,
    operations: Option<PlaybackPermissions>,
    input: Option<(std::fs::File, [i128; 7])>,
}
impl Current {
    async fn identity(&self) -> Result<AuthorizationInfo, ApiError> {
        require_network(&self.state, self.peer)?;
        let auth = self
            .state
            .auth_service
            .authenticate_current_session(&self.context)
            .await?;
        if auth.is_api_key
            || !auth.is_authenticated
            || auth.user_id().is_nil()
            || !auth.has_token()
            || auth.user_id() != self.user
            || auth.device_id.as_deref() != Some(self.device.as_str())
        {
            return Err(ApiError::Unauthorized(
                "current viewer session required".into(),
            ));
        }
        let ip = crate::state::normalize_ip(self.peer.remote.ip());
        let policy = user_authorization(&auth, ip, &self.state, true).await?;
        if !policy.enable_media_playback {
            return Err(ApiError::Forbidden("media playback refused".into()));
        }
        Ok(auth)
    }
    async fn validate(&self) -> Result<(), ApiError> {
        self.identity().await?;
        if let Some((file, stamp)) = &self.input
            && input_stamp(file)? != *stamp
        {
            return Err(ApiError::NotFound("captured media input changed".into()));
        }
        if let Some(view) = &self.pinned {
            let current = view.media_sources.current_pinned_policy().await?;
            if !current.enable_media_playback
                || self.operations.is_some_and(|required| {
                    (required.audio_transcoding && !current.playback.audio_transcoding)
                        || (required.video_transcoding && !current.playback.video_transcoding)
                        || (required.remuxing && !current.playback.remuxing)
                })
            {
                return Err(ApiError::Forbidden(
                    "current source operation refused".into(),
                ));
            }
        }
        Ok(())
    }
}
fn require_network(state: &AppState, peer: DirectPeer) -> Result<(), ApiError> {
    let remote = crate::state::normalize_ip(peer.remote.ip());
    let local = crate::state::normalize_ip(peer.local.ip());
    if remote.is_loopback()
        || remote == local
        || state.remote_access_policy(remote)
            == ferrofin_networking::RemoteAccessPolicyResult::Allow
    {
        return Ok(());
    }
    Err(ApiError::ServiceUnavailable("remote access refused".into()))
}
fn input_stamp(file: &std::fs::File) -> Result<[i128; 7], ApiError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = file.metadata().map_err(|_| missing())?;
        if !m.is_file() {
            return Err(missing());
        }
        Ok([
            i128::from(m.dev()),
            i128::from(m.ino()),
            i128::from(m.len()),
            i128::from(m.mtime()),
            i128::from(m.mtime_nsec()),
            i128::from(m.ctime()),
            i128::from(m.ctime_nsec()),
        ])
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        Err(ApiError::ServiceUnavailable(
            "input identity observation unsupported".into(),
        ))
    }
}
impl Authority for Current {
    fn check(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow_result::Result> + Send + '_>>
    {
        Box::pin(async move { self.validate().await.map_err(|_| anyhow_result::failure()) })
    }
}
// Keep API errors and private source/credential detail out of the transport error.
mod anyhow_result {
    pub type Result = std::result::Result<(), bluefin_iroh_server::AuthorityError>;
    pub fn failure() -> bluefin_iroh_server::AuthorityError {
        bluefin_iroh_server::authority_refused()
    }
}

#[derive(Debug)]
enum MediaRoute {
    Item(Uuid),
    Lease,
}
fn route(path: &str) -> Result<MediaRoute, ApiError> {
    let p = path.split('/').collect::<Vec<_>>();
    let safe = |s: &str| {
        !s.is_empty()
            && s.len() <= 128
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            && s != "."
            && s != ".."
    };
    if p.len() == 7
        && p[1..5] == ["Bluefin", "TV", "v1", "media"]
        && safe(p[5])
        && p[6].parse::<usize>().is_ok()
    {
        return Ok(MediaRoute::Lease);
    }
    if p.len() >= 4 && matches!(p[1], "Audio" | "Videos") {
        let item = Uuid::parse_str(p[2]).map_err(|_| missing())?;
        if item.is_nil() {
            return Err(missing());
        }
        let valid = match &p[3..] {
            ["stream" | "master.m3u8" | "main.m3u8"] => true,
            ["live.m3u8"] => p[1] == "Videos",
            [stream] => stream.strip_prefix("stream.").is_some_and(|ext| {
                [
                    "mp4", "mkv", "webm", "mov", "mp3", "aac", "flac", "wav", "m4a", "ogg", "ts",
                ]
                .contains(&ext)
            }),
            ["hls1" | "hls", playlist, segment] => safe(playlist) && safe(segment),
            _ => false,
        };
        if valid {
            return Ok(MediaRoute::Item(item));
        }
    }
    Err(missing())
}
fn missing() -> ApiError {
    ApiError::NotFound("media route unavailable".into())
}

async fn admit(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    match prepare(&state, &mut request).await {
        Ok((current, Some(view))) => {
            // Identical production route composition and layers, with only the
            // existing scoped input consumers rebound. No local HTTP hop.
            // The outer matched route's private path parameters must not merge
            // with the source-bound router's parameters. Carry only the actual
            // transport context, method, URI and reviewed headers.
            let mut inner = Request::new(Body::empty());
            *inner.method_mut() = request.method().clone();
            *inner.uri_mut() = request.uri().clone();
            *inner.headers_mut() = request.headers().clone();
            inner
                .extensions_mut()
                .insert(ConnectInfo(current.peer.remote));
            inner.extensions_mut().insert(current.peer);
            inner
                .extensions_mut()
                .insert(crate::ip_access::ConnectionLocalAddr(current.peer.local));
            inner
                .extensions_mut()
                .insert(crate::forwarded_headers::Transport::Tls);
            let response = crate::create_router(view).oneshot(inner).await;
            let mut response = match response {
                Ok(r) => r,
                Err(e) => match e {},
            };
            response
                .extensions_mut()
                .insert(ResponseAuthority(Arc::new(current)));
            response
        }
        Ok((current, None)) => {
            let mut response = next.run(request).await;
            response
                .extensions_mut()
                .insert(ResponseAuthority(Arc::new(current)));
            response
        }
        Err(error) => error.into_response(),
    }
}

fn verified_peer(request: &Request) -> Result<DirectPeer, ApiError> {
    let peer = request
        .extensions()
        .get::<DirectPeer>()
        .copied()
        .ok_or_else(|| ApiError::ServiceUnavailable("direct peer required".into()))?;
    if request
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .is_none_or(|ci| ci.0 != peer.remote)
        || peer.remote.ip().is_unspecified()
        || peer.remote.ip().is_multicast()
    {
        return Err(ApiError::ServiceUnavailable("direct peer mismatch".into()));
    }
    if request.headers().keys().any(|n| {
        !matches!(
            n.as_str(),
            "host"
                | "authorization"
                | "range"
                | "if-range"
                | "if-modified-since"
                | "if-none-match"
                | "accept"
        )
    }) {
        return Err(ApiError::BadRequest("media header refused".into()));
    }
    Ok(peer)
}
const QUERY_FIELDS: &[&str] = &[
    "static",
    "mediasourceid",
    "playsessionid",
    "deviceid",
    "container",
    "videocodec",
    "audiocodec",
    "audiobitrate",
    "videobitrate",
    "maxwidth",
    "maxheight",
    "width",
    "height",
    "maxframerate",
    "starttimeticks",
    "audiostreamindex",
    "subtitlestreamindex",
    "segmentcontainer",
    "minsegments",
    "segmentlength",
    "breakonnonkeyframes",
    "enableautostreamcopy",
    "allowvideostreamcopy",
    "allowaudiostreamcopy",
    "transcodereasons",
    "runtimeticks",
    "actualsegmentlengthticks",
    "enableadaptivebitratestreaming",
    "maxstreamingbitrate",
];
struct MediaQuery {
    source: Option<String>,
    static_read: bool,
    device: Option<String>,
    has_session: bool,
}
impl MediaQuery {
    fn parse(request: &Request) -> Result<Self, ApiError> {
        let mut out = Self {
            source: None,
            static_read: false,
            device: None,
            has_session: false,
        };
        let mut seen = std::collections::HashSet::new();
        for (key, value) in
            form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
        {
            let key = key.to_ascii_lowercase();
            if !seen.insert(key.clone()) || !QUERY_FIELDS.contains(&key.as_str()) {
                return Err(ApiError::BadRequest("media query refused".into()));
            }
            match key.as_str() {
                "mediasourceid" => out.source = Some(value.into_owned()),
                "deviceid" => out.device = Some(value.into_owned()),
                "static" => out.static_read = value.eq_ignore_ascii_case("true"),
                "playsessionid" => {
                    if value.is_empty()
                        || value.len() > 128
                        || !value
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                    {
                        return Err(ApiError::BadRequest(
                            "ordinary playback session required".into(),
                        ));
                    }
                    out.has_session = true;
                }
                _ => {}
            }
        }
        Ok(out)
    }
    fn bind_device(
        &self,
        current: &Current,
        item_media: bool,
        request: &mut Request,
    ) -> Result<(), ApiError> {
        if self.device.as_ref().is_some_and(|d| d != &current.device) {
            return Err(ApiError::Unauthorized("session device mismatch".into()));
        }
        if !item_media || self.static_read {
            return Ok(());
        }
        if !self.has_session {
            return Err(ApiError::BadRequest(
                "ordinary playback session required".into(),
            ));
        }
        if self.device.is_none() {
            if current.device.len() > 128
                || !current
                    .device
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            {
                return Err(ApiError::BadRequest(
                    "media device identifier unavailable".into(),
                ));
            }
            *request.uri_mut() = format!(
                "{}{}DeviceId={}",
                request.uri(),
                if request.uri().query().is_some() {
                    "&"
                } else {
                    "?"
                },
                current.device
            )
            .parse()
            .map_err(|_| missing())?;
        }
        Ok(())
    }
}
async fn authenticate(
    state: &AppState,
    peer: DirectPeer,
    headers: &axum::http::HeaderMap,
) -> Result<(Current, AuthorizationInfo), ApiError> {
    let ip = crate::state::normalize_ip(peer.remote.ip());
    let context = auth_request_context(headers, None, Some(ip.to_string()));
    let auth = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        state.auth_service.authenticate_current_session(&context),
    )
    .await
    .map_err(|_| ApiError::ServiceUnavailable("session unavailable".into()))??;
    let current = Current {
        state: state.clone(),
        context,
        peer,
        user: auth.user_id(),
        device: auth
            .device_id
            .clone()
            .ok_or_else(|| ApiError::Unauthorized("device required".into()))?,
        pinned: None,
        operations: None,
        input: None,
    };
    current.identity().await?;
    Ok((current, auth))
}
async fn capture_item(
    current: &mut Current,
    request: &axum::http::Request<()>,
    item: Uuid,
    auth: &AuthorizationInfo,
    query: &MediaQuery,
) -> Result<AppState, ApiError> {
    let state = &current.state;
    let visible =
        crate::handlers::items::require_visible_item(state, item, auth.user.as_deref()).await?;
    let audio = request.uri().path().starts_with("/Audio/");
    if (audio
        && ferrofin_model::data::BaseItemKind::from_stored_type_name(&visible.type_)
            != Some(ferrofin_model::data::BaseItemKind::Audio))
        || (!audio && !crate::handlers::videos::is_video(&visible))
    {
        return Err(missing());
    }
    require_streamable_item(
        state,
        auth,
        request.extensions(),
        request.headers(),
        item,
        query.source.as_deref(),
    )
    .await?;
    let chosen = state
        .media_sources
        .get_stream_media_source(item, query.source.as_deref(), None)
        .await?;
    let selected = chosen.id.filter(|id| !id.is_empty()).ok_or_else(missing)?;
    let input = static_source(state, item, query.source.as_deref(), None).await?;
    if input.protocol != MediaProtocol::File
        || input.is_live_stream
        || input.is_infinite_stream
        || !std::path::Path::new(&input.path).is_absolute()
    {
        return Err(ApiError::BadRequest("finite local media required".into()));
    }
    let pins = state
        .tv_playback_pins
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("source pin unavailable".into()))?;
    let view = pins
        .pin(state, item, &selected, &input.path, current.user)
        .await
        .map_err(|_| missing())?;
    if !view.has_scoped_media() {
        return Err(ApiError::ServiceUnavailable(
            "source context unavailable".into(),
        ));
    }
    let input = static_source(&view, item, Some(&selected), None).await?;
    let input_owner = view.clone();
    let file = view
        .blocking_file_system(move |_| {
            let _owner = input_owner;
            std::fs::File::open(input.path)
        })
        .await?
        .map_err(|_| missing())?;
    let stamp = input_stamp(&file)?;
    current.input = Some((file, stamp));
    if !query.static_read {
        current.operations = Some(view.media_sources.current_pinned_policy().await?.playback);
    }
    current.pinned = Some(view.clone());
    current.validate().await?;
    Ok(view)
}
async fn prepare(
    state: &AppState,
    request: &mut Request,
) -> Result<(Current, Option<AppState>), ApiError> {
    let peer = verified_peer(request)?;
    require_network(state, peer)?;
    let selected = route(request.uri().path())?;
    let query = MediaQuery::parse(request)?;
    let (mut current, auth) = authenticate(state, peer, request.headers()).await?;
    query.bind_device(&current, matches!(selected, MediaRoute::Item(_)), request)?;
    request
        .extensions_mut()
        .insert(crate::ip_access::ConnectionLocalAddr(peer.local));
    request
        .extensions_mut()
        .insert(crate::forwarded_headers::Transport::Tls);
    let view = match selected {
        MediaRoute::Lease => None,
        MediaRoute::Item(item) => {
            let mut transport = axum::http::Request::new(());
            *transport.uri_mut() = request.uri().clone();
            *transport.headers_mut() = request.headers().clone();
            *transport.extensions_mut() = request.extensions().clone();
            Some(capture_item(&mut current, &transport, item, &auth, &query).await?)
        }
    };
    Ok((current, view))
}

/// Current authenticated disclosure check; a running endpoint never grants media.
pub async fn advertised(
    state: &AppState,
    request: &axum::http::request::Parts,
) -> Option<bluefin_contract::transport::IrohMediaTransport> {
    let transport = state.iroh_media_transport()?;
    let ip = state.client_address_of(&request.extensions, &request.headers);
    let ctx = auth_request_context(&request.headers, None, Some(ip.to_string()));
    let auth = state
        .auth_service
        .authenticate_current_session(&ctx)
        .await
        .ok()?;
    let policy = user_authorization(&auth, ip, state, true).await.ok()?;
    if !policy.enable_media_playback
        || (!ip.is_loopback()
            && state.remote_access_policy(ip)
                != ferrofin_networking::RemoteAccessPolicyResult::Allow)
    {
        return None;
    }
    Some(transport)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn route_scope_has_no_prefix_or_catalog_grant() {
        let id = Uuid::new_v4();
        for p in [
            format!("/Audio/{id}/stream"),
            format!("/Videos/{id}/master.m3u8"),
            format!("/Videos/{id}/hls1/main/0.ts"),
            "/Bluefin/TV/v1/media/opaque/0".into(),
        ] {
            assert!(route(&p).is_ok(), "{p}");
        }
        for p in [
            "/System/Shutdown",
            "/Items",
            "/Videos/ActiveEncodings",
            "/Audio/not-an-item/stream",
            "/Bluefin/TV/v1/leases",
            "/Bluefin/TV/v1/media/a/../b",
        ] {
            assert!(route(p).is_err(), "{p}");
        }
    }
}
