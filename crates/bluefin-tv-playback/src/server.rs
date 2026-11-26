//! In-process streaming adapter. No sockets, redirects, URL credentials, or
//! independent authorization. Each router owns its own bounded lease store.
use crate::{LeaseRequest, LeaseResponse, VERSION};
use async_trait::async_trait;
use axum::{
    Extension, Json, Router,
    body::{Body, Bytes, to_bytes},
    extract::{DefaultBodyLimit, Path, Request},
    http::{Method, Request as HttpRequest, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use futures_util::{StreamExt, stream};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, watch},
    time::{Instant, timeout},
};

pub type Failure = StatusCode;
pub const ABSOLUTE_SECONDS: u64 = 7200;
pub const IDLE_SECONDS: u64 = 90;
const OP_TIMEOUT: Duration = Duration::from_secs(15);
const RECHECK: Duration = Duration::from_secs(3);
const MAX_LEASES: usize = 32;
const MAX_RESOURCES: usize = 4096;
const MAX_RESOURCE_BYTES: usize = 1024 * 1024;
const MAX_MANIFEST: usize = 1024 * 1024;

/// A backend must admit only ordinary viewers authenticated by the standard
/// Authorization header. Session must retain viewer/device/item/session binding
/// and its credential privately. Never implement this with a loopback HTTP client.
#[async_trait]
pub trait PlaybackBackend: Send + Sync + 'static {
    type Session: Send + Sync + 'static;
    async fn admit(
        &self,
        transport: &HttpRequest<()>,
        request: LeaseRequest,
    ) -> Result<Admission<Self::Session>, Failure>;
    /// Fresh existing server authentication AND policy, including current request
    /// client address/proxy trust, disabled/revoked/schedule and folder/item access.
    async fn reauthorize(
        &self,
        session: &Self::Session,
        transport: &HttpRequest<()>,
    ) -> Result<(), Failure>;
    async fn serve(
        &self,
        session: &Self::Session,
        transport: &HttpRequest<()>,
        uri: &str,
        method: Method,
    ) -> Result<Response, Failure>;
    /// Idempotently cancel this exact server-selected session's encodings/resources.
    async fn release(&self, session: &Self::Session);
}
pub struct Admission<S> {
    pub session: S,
    pub viewer: String,
    pub device: String,
    pub item_id: String,
    pub play_session_id: String,
    /// Server-produced route, including a fixed negotiated query. No credentials.
    pub root_uri: String,
}
struct Lease<S> {
    session: S,
    viewer: String,
    device: String,
    item: String,
    play_session_id: String,
    born: Instant,
    last: Mutex<Instant>,
    resources: Mutex<Book>,
    dispatch: tokio::sync::RwLock<()>,
    cancelled: watch::Sender<bool>,
    connections: Arc<Semaphore>,
}
struct Book {
    uris: Vec<String>,
    index: HashMap<String, usize>,
    bytes: usize,
}
impl Book {
    fn add(&mut self, uri: String) -> Result<usize, Failure> {
        if let Some(n) = self.index.get(&uri) {
            return Ok(*n);
        }
        if self.uris.len() >= MAX_RESOURCES || self.bytes + uri.len() > MAX_RESOURCE_BYTES {
            return Err(StatusCode::INSUFFICIENT_STORAGE);
        }
        self.bytes += uri.len();
        self.index.insert(uri.clone(), self.uris.len());
        self.uris.push(uri);
        Ok(self.uris.len() - 1)
    }
}
struct Engine<B: PlaybackBackend> {
    backend: Arc<B>,
    leases: Mutex<HashMap<String, Arc<Lease<B::Session>>>>,
    connections: Arc<Semaphore>,
    admissions: Arc<Semaphore>,
}
impl<B: PlaybackBackend> Drop for Engine<B> {
    fn drop(&mut self) {
        let leases: Vec<_> = self
            .leases
            .get_mut()
            .unwrap()
            .drain()
            .map(|(_, lease)| lease)
            .collect();
        for lease in &leases {
            lease.cancelled.send_replace(true);
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let backend = self.backend.clone();
            runtime.spawn(async move {
                for lease in leases {
                    let _ = timeout(OP_TIMEOUT, backend.release(&lease.session)).await;
                }
            });
        }
    }
}
fn protected(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store, private"),
    );
    response.headers_mut().insert(
        "referrer-policy",
        header::HeaderValue::from_static("no-referrer"),
    );
    response.headers_mut().insert(
        "x-content-type-options",
        header::HeaderValue::from_static("nosniff"),
    );
    response
}
async fn controls(response: Response) -> Response {
    protected(response)
}
fn fail(code: Failure) -> Response {
    protected(code.into_response())
}
fn transport(req: &Request) -> HttpRequest<()> {
    let mut out = HttpRequest::new(());
    *out.method_mut() = req.method().clone();
    *out.uri_mut() = req.uri().clone();
    *out.headers_mut() = req.headers().clone();
    *out.extensions_mut() = req.extensions().clone();
    out
}
impl<S> Lease<S> {
    fn expired(&self) -> bool {
        self.born.elapsed().as_secs() >= ABSOLUTE_SECONDS
            || self.last.lock().unwrap().elapsed().as_secs() >= IDLE_SECONDS
            || *self.cancelled.borrow()
    }
}
impl<B: PlaybackBackend> Engine<B> {
    async fn retire(&self, grant: &str) {
        let lease = self.leases.lock().unwrap().remove(grant);
        if let Some(lease) = lease {
            lease.cancelled.send_replace(true);
            let _dispatch = lease.dispatch.write().await;
            let _ = timeout(OP_TIMEOUT, self.backend.release(&lease.session)).await;
        }
    }
    fn get(&self, grant: &str) -> Result<Arc<Lease<B::Session>>, Failure> {
        if grant.len() != 64 || !grant.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(StatusCode::NOT_FOUND);
        }
        self.leases
            .lock()
            .unwrap()
            .get(grant)
            .filter(|l| !l.expired())
            .cloned()
            .ok_or(StatusCode::NOT_FOUND)
    }
}
/// Merge this router before the existing IP/auth/CORS layers. Reaper is weakly
/// owned and exits after the router/store is dropped. No cross-instance globals.
pub fn routes<S: Clone + Send + Sync + 'static, B: PlaybackBackend>(backend: B) -> Router<S> {
    let engine = Arc::new(Engine {
        backend: Arc::new(backend),
        leases: Mutex::new(HashMap::new()),
        connections: Arc::new(Semaphore::new(64)),
        admissions: Arc::new(Semaphore::new(4)),
    });
    let weak: Weak<Engine<B>> = Arc::downgrade(&engine);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let Some(engine) = weak.upgrade() else { break };
            let expired: Vec<_> = engine
                .leases
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, l)| l.expired())
                .map(|(k, _)| k.clone())
                .collect();
            for grant in expired {
                engine.retire(&grant).await;
            }
        }
    });
    Router::new()
        .route("/Bluefin/TV/v1", get(|| async { protected(Json(serde_json::json!({"version":VERSION,"mediaLease":true,"absoluteSeconds":ABSOLUTE_SECONDS,"idleSeconds":IDLE_SECONDS})).into_response()) }))
        .route("/Bluefin/TV/v1/leases", post(create::<B>).layer(DefaultBodyLimit::max(4096)))
        .route("/Bluefin/TV/v1/leases/{grant}", delete(release::<B>))
        .route("/Bluefin/TV/v1/media/{grant}/{resource}", get(media::<B>).head(media::<B>))
        .layer(Extension(engine))
        .layer(axum::middleware::map_response(controls))
}
async fn create<B: PlaybackBackend>(
    Extension(engine): Extension<Arc<Engine<B>>>,
    req: Request,
) -> Response {
    if req.uri().query().is_some() {
        return fail(StatusCode::BAD_REQUEST);
    }
    let context = transport(&req);
    let Ok(_permit) = engine.admissions.clone().try_acquire_owned() else {
        return fail(StatusCode::TOO_MANY_REQUESTS);
    };
    if engine.leases.lock().unwrap().len() >= MAX_LEASES {
        return fail(StatusCode::TOO_MANY_REQUESTS);
    }
    let Ok(Ok(bytes)) = timeout(OP_TIMEOUT, to_bytes(req.into_body(), 4096)).await else {
        return fail(StatusCode::PAYLOAD_TOO_LARGE);
    };
    let Ok(request) = serde_json::from_slice::<LeaseRequest>(&bytes) else {
        return fail(StatusCode::BAD_REQUEST);
    };
    if !request.valid() {
        return fail(StatusCode::BAD_REQUEST);
    }
    let admitted = match timeout(OP_TIMEOUT, engine.backend.admit(&context, request)).await {
        Ok(Ok(a)) => a,
        Ok(Err(e)) => return fail(e),
        Err(_) => return fail(StatusCode::GATEWAY_TIMEOUT),
    };
    let valid = validate_uri(
        &admitted.root_uri,
        &admitted.item_id,
        &admitted.play_session_id,
    )
    .is_ok();
    let grant = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let mut book = Book {
        uris: Vec::new(),
        index: HashMap::new(),
        bytes: 0,
    };
    let valid = valid && book.add(admitted.root_uri.clone()).is_ok();
    let accepted = {
        let leases = engine.leases.lock().unwrap();
        valid
            && leases.len() < MAX_LEASES
            && leases
                .values()
                .filter(|l| l.viewer == admitted.viewer)
                .count()
                < 4
            && leases
                .values()
                .filter(|l| l.viewer == admitted.viewer && l.device == admitted.device)
                .count()
                < 2
    };
    if !accepted {
        let _ = timeout(OP_TIMEOUT, engine.backend.release(&admitted.session)).await;
        return fail(if valid {
            StatusCode::TOO_MANY_REQUESTS
        } else {
            StatusCode::BAD_GATEWAY
        });
    }
    let response = LeaseResponse {
        version: VERSION,
        media_url: format!("/Bluefin/TV/v1/media/{grant}/0"),
        release_url: format!("/Bluefin/TV/v1/leases/{grant}"),
        play_session_id: admitted.play_session_id.clone(),
        expires_in_seconds: ABSOLUTE_SECONDS,
        idle_seconds: IDLE_SECONDS,
    };
    let lease = Arc::new(Lease {
        session: admitted.session,
        viewer: admitted.viewer,
        device: admitted.device,
        item: admitted.item_id,
        play_session_id: admitted.play_session_id,
        born: Instant::now(),
        last: Mutex::new(Instant::now()),
        resources: Mutex::new(book),
        dispatch: tokio::sync::RwLock::new(()),
        cancelled: watch::channel(false).0,
        connections: Arc::new(Semaphore::new(8)),
    });
    // Admission concurrency is bounded; recheck under insertion lock.
    {
        let mut leases = engine.leases.lock().unwrap();
        if leases.len() < MAX_LEASES
            && leases.values().filter(|l| l.viewer == lease.viewer).count() < 4
            && leases
                .values()
                .filter(|l| l.viewer == lease.viewer && l.device == lease.device)
                .count()
                < 2
        {
            leases.insert(grant.clone(), lease.clone());
        }
    }
    if !engine.leases.lock().unwrap().contains_key(&grant) {
        let _ = timeout(OP_TIMEOUT, engine.backend.release(&lease.session)).await;
        return fail(StatusCode::TOO_MANY_REQUESTS);
    }
    protected((StatusCode::CREATED, Json(response)).into_response())
}
async fn release<B: PlaybackBackend>(
    Extension(engine): Extension<Arc<Engine<B>>>,
    Path(grant): Path<String>,
    req: Request,
) -> Response {
    if req.uri().query().is_some() {
        return fail(StatusCode::BAD_REQUEST);
    }
    // The opaque grant can only destroy its own stream. No account/user selector.
    engine.retire(&grant).await;
    protected(StatusCode::NO_CONTENT.into_response())
}
async fn media<B: PlaybackBackend>(
    Extension(engine): Extension<Arc<Engine<B>>>,
    Path((grant, resource)): Path<(String, usize)>,
    req: Request,
) -> Response {
    if req.uri().query().is_some() {
        return fail(StatusCode::BAD_REQUEST);
    }
    let lease = match engine.get(&grant) {
        Ok(l) => l,
        Err(e) => return fail(e),
    };
    let Ok(global) = engine.connections.clone().try_acquire_owned() else {
        return fail(StatusCode::TOO_MANY_REQUESTS);
    };
    let Ok(local) = lease.connections.clone().try_acquire_owned() else {
        return fail(StatusCode::TOO_MANY_REQUESTS);
    };
    let context = Arc::new(transport(&req));
    match timeout(
        OP_TIMEOUT,
        engine.backend.reauthorize(&lease.session, &context),
    )
    .await
    {
        Ok(Ok(())) => (),
        Ok(Err(e)) => {
            engine.retire(&grant).await;
            return fail(e);
        }
        Err(_) => return fail(StatusCode::GATEWAY_TIMEOUT),
    }
    let uri = match lease.resources.lock().unwrap().uris.get(resource).cloned() {
        Some(u) => u,
        None => return fail(StatusCode::NOT_FOUND),
    };
    let playlist = uri.split('?').next().unwrap_or("").ends_with(".m3u8");
    if playlist && req.headers().contains_key(header::RANGE) {
        return fail(StatusCode::RANGE_NOT_SATISFIABLE);
    }
    let method = if playlist {
        Method::GET
    } else {
        req.method().clone()
    };
    // Retirement cancels first, waits for already-started dispatches, then
    // stops exact server encodings. A late request cannot start a job after stop.
    let dispatch = lease.dispatch.read().await;
    if lease.expired() {
        return fail(StatusCode::NOT_FOUND);
    }
    let upstream = match timeout(
        OP_TIMEOUT,
        engine.backend.serve(&lease.session, &context, &uri, method),
    )
    .await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return fail(e),
        Err(_) => return fail(StatusCode::GATEWAY_TIMEOUT),
    };
    drop(dispatch);
    if lease.expired() {
        return fail(StatusCode::NOT_FOUND);
    }
    if upstream.status().is_redirection() {
        return fail(StatusCode::BAD_GATEWAY);
    }
    if !upstream.status().is_success() {
        return fail(upstream.status());
    }
    *lease.last.lock().unwrap() = Instant::now();
    if playlist {
        let Ok(Ok(bytes)) = timeout(OP_TIMEOUT, to_bytes(upstream.into_body(), MAX_MANIFEST)).await
        else {
            return fail(StatusCode::BAD_GATEWAY);
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return fail(StatusCode::BAD_GATEWAY);
        };
        let rewritten = match rewrite_manifest(text, &uri, &grant, &lease) {
            Ok(t) => t,
            Err(e) => return fail(e),
        };
        let mut response = Response::new(if req.method() == Method::HEAD {
            Body::empty()
        } else {
            Body::from(rewritten.clone())
        });
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/vnd.apple.mpegurl"),
        );
        response.headers_mut().insert(
            header::CONTENT_LENGTH,
            header::HeaderValue::from_str(&rewritten.len().to_string()).unwrap(),
        );
        return protected(response);
    }
    let (parts, body) = upstream.into_parts();
    let status = parts.status;
    let mut headers = axum::http::HeaderMap::new();
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::ACCEPT_RANGES,
        header::LAST_MODIFIED,
    ] {
        if let Some(value) = parts.headers.get(&name) {
            headers.insert(name, value.clone());
        }
    }
    let body = if req.method() == Method::HEAD {
        Body::empty()
    } else {
        bounded_body(engine, lease, context, body, global, local)
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    protected(response)
}
fn bounded_body<B: PlaybackBackend>(
    engine: Arc<Engine<B>>,
    lease: Arc<Lease<B::Session>>,
    context: Arc<HttpRequest<()>>,
    body: Body,
    global: OwnedSemaphorePermit,
    local: OwnedSemaphorePermit,
) -> Body {
    let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(1);
    let mut cancelled = lease.cancelled.subscribe();
    // One queued frame plus one pending frame. A stalled reader has 15 seconds
    // before this task drops the upstream body, its file/job observer and permits.
    tokio::spawn(async move {
        let (_global, _local) = (global, local);
        let mut stream = Box::pin(body.into_data_stream());
        let mut last_data = Instant::now();
        let mut last_check = Instant::now();
        loop {
            if lease.expired() || last_data.elapsed() >= OP_TIMEOUT {
                break;
            }
            if last_check.elapsed() >= RECHECK {
                if !matches!(
                    timeout(
                        OP_TIMEOUT,
                        engine.backend.reauthorize(&lease.session, &context)
                    )
                    .await,
                    Ok(Ok(()))
                ) {
                    lease.cancelled.send_replace(true);
                    break;
                }
                last_check = Instant::now();
            }
            let data = tokio::select! {
                biased;
                _ = sender.closed() => break,
                _ = cancelled.changed() => break,
                _ = tokio::time::sleep(RECHECK) => continue,
                data = stream.next() => match data { Some(Ok(data)) => data, _ => break }
            };
            // Backend frame sizes must be bounded too. Existing file/transcode
            // handlers use bounded read chunks; refuse an oversized injected frame.
            if data.len() > 1024 * 1024 {
                break;
            }
            let deadline = Instant::now() + OP_TIMEOUT;
            let mut pending = Box::pin(sender.send(Ok(data)));
            let mut delivered = false;
            loop {
                tokio::select! {
                    biased;
                    _ = sender.closed() => break,
                    _ = cancelled.changed() => break,
                    _ = tokio::time::sleep_until(deadline) => break,
                    _ = tokio::time::sleep(RECHECK) => {
                        if lease.expired() || !matches!(timeout(OP_TIMEOUT, engine.backend.reauthorize(&lease.session, &context)).await, Ok(Ok(()))) { lease.cancelled.send_replace(true); break; }
                        last_check=Instant::now();
                    },
                    result = &mut pending => { delivered=result.is_ok(); break; }
                }
            }
            if !delivered {
                break;
            }
            last_data = Instant::now();
            *lease.last.lock().unwrap() = Instant::now();
        }
    });
    Body::from_stream(stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|item| (item, receiver))
    }))
}

/// Reject URL normalisation tricks before parsing. This deliberately accepts
/// only canonical ASCII local routes generated by the existing planner.
fn safe_text(uri: &str) -> bool {
    uri.len() <= 4096
        && !uri.is_empty()
        && uri.bytes().all(|c| (0x21..=0x7e).contains(&c))
        && !uri.contains(['\\', '#', '"', '\''])
        && !uri.split('?').next().unwrap_or("").contains('%')
        && !uri.starts_with("//")
        && !uri.contains("://")
        && !uri
            .split('?')
            .next()
            .unwrap_or("")
            .split('/')
            .any(|s| s == "." || s == "..")
}
fn query(uri: &str) -> Result<std::collections::BTreeMap<String, String>, Failure> {
    let mut out = std::collections::BTreeMap::new();
    if let Some((_, q)) = uri.split_once('?') {
        for pair in q.split('&') {
            let Some((key, value)) = pair.split_once('=') else {
                return Err(StatusCode::BAD_GATEWAY);
            };
            if !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            {
                return Err(StatusCode::BAD_GATEWAY);
            }
            let value = value.replace("%2C", ",").replace("%2c", ",");
            if value.contains(['%', '+']) {
                return Err(StatusCode::BAD_GATEWAY);
            }
            let key = key.to_ascii_lowercase();
            if matches!(
                key.as_str(),
                "apikey" | "api_key" | "token" | "authorization" | "userid" | "url" | "path"
            ) || out.insert(key, value).is_some()
            {
                return Err(StatusCode::BAD_GATEWAY);
            }
        }
    }
    Ok(out)
}
pub fn validate_uri(uri: &str, item: &str, session: &str) -> Result<(), Failure> {
    if !safe_text(uri) || !uri.starts_with('/') {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let path = uri.split('?').next().unwrap();
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() < 4 || !matches!(parts[1], "Videos" | "Audio") || parts[2] != item {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let valid = (parts.len() == 4
        && matches!(
            parts[3],
            "master.m3u8" | "main.m3u8" | "stream.mp3" | "stream.aac" | "stream.mp4"
        ))
        || (parts.len() == 6 && parts[3] == "hls1" && parts[4] == "main" && {
            let (n, ext) = parts[5].split_once('.').unwrap_or(("", ""));
            matches!(ext, "ts" | "mp4" | "aac" | "key")
                && n.parse::<i32>().is_ok_and(|n| (-1..=100000).contains(&n))
        });
    let q = query(uri)?;
    if !valid || q.get("playsessionid").map(String::as_str) != Some(session) {
        return Err(StatusCode::BAD_GATEWAY);
    }
    Ok(())
}
fn child_uri(reference: &str, parent: &str, item: &str, session: &str) -> Result<String, Failure> {
    if !safe_text(reference) {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let base = url::Url::parse(&format!("https://lease.invalid{parent}"))
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let target = base.join(reference).map_err(|_| StatusCode::BAD_GATEWAY)?;
    if target.origin() != base.origin() {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let mut fixed = query(parent)?;
    for (key, value) in query(reference)? {
        if let Some(existing) = fixed.get(&key) {
            if existing != &value {
                return Err(StatusCode::BAD_GATEWAY);
            }
        } else if matches!(key.as_str(), "runtimeticks" | "actualsegmentlengthticks")
            && value.parse::<u64>().is_ok()
        {
            fixed.insert(key, value);
        } else {
            return Err(StatusCode::BAD_GATEWAY);
        }
    }
    let q = fixed
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let uri = format!("{}?{q}", target.path());
    validate_uri(&uri, item, session)?;
    Ok(uri)
}
fn rewrite_manifest<S>(
    text: &str,
    parent: &str,
    grant: &str,
    lease: &Lease<S>,
) -> Result<String, Failure> {
    if !text.starts_with("#EXTM3U\n") && !text.starts_with("#EXTM3U\r\n") {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let mut output = String::with_capacity(text.len());
    let mut book = lease.resources.lock().unwrap();
    let mut rewrite = |reference: &str| -> Result<String, Failure> {
        let uri = child_uri(reference, parent, &lease.item, &lease.play_session_id)?;
        let n = book.add(uri)?;
        Ok(format!("/Bluefin/TV/v1/media/{grant}/{n}"))
    };
    for line in text.lines() {
        if line.len() > 8192 {
            return Err(StatusCode::BAD_GATEWAY);
        }
        if line.starts_with('#') {
            // Encrypted HLS needs a local, admitted key route. No external DRM,
            // data URLs, content steering, variables or session-data extraction.
            if line.contains("{$")
                || line.starts_with("#EXT-X-DEFINE")
                || line.starts_with("#EXT-X-CONTENT-STEERING")
                || line.starts_with("#EXT-X-SESSION-DATA")
            {
                return Err(StatusCode::BAD_GATEWAY);
            }
            if let Some((prefix, tail)) = line.split_once("URI=\"") {
                let Some((reference, suffix)) = tail.split_once('"') else {
                    return Err(StatusCode::BAD_GATEWAY);
                };
                if suffix.contains("URI=") || prefix.contains("URI=") {
                    return Err(StatusCode::BAD_GATEWAY);
                }
                output.push_str(prefix);
                output.push_str("URI=\"");
                output.push_str(&rewrite(reference)?);
                output.push('"');
                output.push_str(suffix);
            } else {
                if line.contains("URI=") {
                    return Err(StatusCode::BAD_GATEWAY);
                }
                output.push_str(line);
            }
        } else if !line.is_empty() {
            output.push_str(&rewrite(line)?);
        }
        output.push('\n');
        if output.len() > MAX_MANIFEST {
            return Err(StatusCode::BAD_GATEWAY);
        }
    }
    Ok(output)
}
