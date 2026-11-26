//! Native media capability bridge. Bounded concurrent streaming, item/origin pins,
//! HLS URI rewriting, no account credential reaches the decoder or its URLs.
use crate::{
    playback::{self, Plan},
    protocol::{self, Result, Session},
};
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderValue, Request, Response, StatusCode},
    routing::any,
};
use futures_util::StreamExt;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Semaphore, watch};
use url::Url;
const IDLE: Duration = Duration::from_secs(10);
struct Scope {
    session: Session,
    plan: Plan,
    host: String,
    secret: String,
    resources: Mutex<Resources>,
    http: reqwest::Client,
    stop: watch::Sender<bool>,
    slots: Arc<Semaphore>,
    tunnel: Option<String>,
}
struct Resources {
    urls: HashMap<String, Url>,
    order: VecDeque<String>,
    next: u64,
    root: String,
}
pub struct Bridge {
    pub url: String,
    stop: watch::Sender<bool>,
    stopped: Arc<AtomicBool>,
    receipt: Arc<Mutex<Option<Result<()>>>>,
}
struct TransportHold {
    #[cfg(feature = "direct")]
    id: Option<u64>,
}
impl TransportHold {
    fn close(&mut self) -> Result<()> {
        #[cfg(feature = "direct")]
        if let Some(id) = self.id.take() {
            bluefin_iroh_transport::stop_with_receipt(id)
                .map_err(|_| "Direct transport shutdown was not acknowledged")?;
        }
        Ok(())
    }
}
impl Drop for TransportHold {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
impl Bridge {
    /// Start on a playback worker; pinned transport setup can block. A failed
    /// later setup step still retires the tunnel acquired by this call.
    pub fn start(
        session: Session,
        plan: Plan,
        peer: Option<(String, std::net::SocketAddr)>,
    ) -> Result<Self> {
        #[cfg(feature = "direct")]
        let tunnel = if let Some((peer_id, peer_address)) = peer {
            Some(
                bluefin_iroh_transport::start(bluefin_iroh_transport::ClientConfig {
                    peer_id,
                    peer_address,
                    authorization: protocol::authorization(&session.token)?,
                })
                .map_err(|_| "Pinned Iroh media route could not start")?,
            )
        } else {
            None
        };
        #[cfg(not(feature = "direct"))]
        let tunnel: Option<(u64, String)> = if peer.is_some() {
            return Err("Direct transport not compiled in".into());
        } else {
            None
        };
        // This owner also closes a successfully started tunnel if later setup
        // fails. start() already runs off the UI thread for pinned transport.
        let mut transport = TransportHold {
            #[cfg(feature = "direct")]
            id: tunnel.as_ref().map(|(id, _)| *id),
        };
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(IDLE)
            .read_timeout(IDLE)
            .build()
            .map_err(|_| "Media HTTP client initialization failed")?;
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|_| "Cannot bind local media bridge")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "Cannot configure local media bridge")?;
        let host = listener
            .local_addr()
            .map_err(|_| "Invalid media listener")?
            .to_string();
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| "Random source unavailable")?;
        let secret: String = bytes.iter().map(|n| format!("{n:02x}")).collect();
        let suffix = suffix(&plan.url);
        let key = format!("/{secret}/0{suffix}");
        let url = format!("http://{host}{key}");
        let (stop, _) = watch::channel(false);
        let stopped = Arc::new(AtomicBool::new(false));
        let receipt = Arc::new(Mutex::new(None));
        let scope = Arc::new(Scope {
            session,
            resources: Mutex::new(Resources {
                urls: HashMap::from([(key.clone(), plan.url.clone())]),
                order: VecDeque::new(),
                next: 1,
                root: key,
            }),
            plan,
            host,
            secret,
            http,
            stop: stop.clone(),
            slots: Arc::new(Semaphore::new(4)),
            tunnel: tunnel.as_ref().map(|(_, s)| s.clone()),
        });
        let done = stopped.clone();
        let result = receipt.clone();
        std::thread::Builder::new()
            .name("bluefin-media".into())
            .spawn(move || {
                let bridge_result = run_bridge(listener, scope);
                let transport_result = transport.close();
                if let Ok(mut result) = result.lock() {
                    *result = Some(bridge_result.and(transport_result));
                }
                done.store(true, Ordering::Release);
            })
            .map_err(|_| "Cannot start media bridge worker")?;
        Ok(Self {
            url,
            stop,
            stopped,
            receipt,
        })
    }
    pub fn stop(&self) {
        self.stop.send_replace(true);
    }
    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    /// Confirms the local bridge worker and any selected direct transport
    /// retired. A pending or failed drain never admits automatic queue advance.
    pub fn cleanup_receipt(&self) -> Result<()> {
        if !self.stopped() {
            return Err("Media bridge retirement is still pending".into());
        }
        self.receipt
            .lock()
            .map_err(|_| "Media bridge receipt unavailable")?
            .clone()
            .unwrap_or_else(|| Err("Media bridge cleanup unconfirmed".into()))
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop();
    }
}
async fn canceled(receiver: &mut watch::Receiver<bool>) {
    if !*receiver.borrow_and_update() {
        let _ = receiver.changed().await;
    }
}
fn run_bridge(listener: std::net::TcpListener, scope: Arc<Scope>) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|_| "Media bridge runtime unavailable")?;
    let result = runtime.block_on(async move {
        let listener = tokio::net::TcpListener::from_std(listener)
            .map_err(|_| "Media bridge listener unavailable")?;
        let mut shutdown = scope.stop.subscribe();
        let mut bounded = shutdown.clone();
        let app = Router::new().fallback(any(media)).with_state(scope);
        let serving = std::future::IntoFuture::into_future(
            axum::serve(listener, app)
                .with_graceful_shutdown(async move { canceled(&mut shutdown).await }),
        );
        tokio::pin!(serving);
        tokio::select! {
            result = &mut serving => result.map_err(|_| "Media bridge failed".into()),
            _ = canceled(&mut bounded) => {
                tokio::time::timeout(IDLE, &mut serving).await
                    .map_err(|_| "Media bridge drain timed out")?
                    .map_err(|_| "Media bridge failed".into())
            }
        }
    });
    runtime.shutdown_timeout(Duration::from_secs(1));
    result
}
fn empty(code: StatusCode) -> Response<Body> {
    Response::builder()
        .status(code)
        .header("Cache-Control", "no-store")
        .body(Body::empty())
        .unwrap()
}
fn suffix(url: &Url) -> &'static str {
    match url.path().rsplit('.').next().unwrap_or("") {
        "m3u8" => ".m3u8",
        "ts" => ".ts",
        "mp4" => ".mp4",
        "m4s" => ".m4s",
        "vtt" => ".vtt",
        "aac" => ".aac",
        "key" => ".key",
        _ => ".bin",
    }
}
fn register(scope: &Scope, base: &Url, raw: &str) -> Result<String> {
    let url = playback::pinned_url(base, &scope.plan.prefix, raw)?;
    let mut resources = scope.resources.lock().unwrap();
    if let Some(path) = resources
        .urls
        .iter()
        .find(|(_, u)| **u == url)
        .map(|(p, _)| p.clone())
    {
        if path != resources.root {
            resources.order.retain(|p| p != &path);
            resources.order.push_back(path.clone());
        }
        return Ok(path);
    }
    if resources.urls.len() >= 2048
        && let Some(old) = resources.order.pop_front()
    {
        resources.urls.remove(&old);
    }
    let key = format!("/{}/{}{}", scope.secret, resources.next, suffix(&url));
    resources.next = resources
        .next
        .checked_add(1)
        .ok_or("Media resource counter exhausted")?;
    resources.urls.insert(key.clone(), url);
    resources.order.push_back(key.clone());
    Ok(key)
}
fn manifest(scope: &Scope, base: &Url, text: &str) -> Result<String> {
    if !text.starts_with("#EXTM3U") || text.len() > 1024 * 1024 {
        return Err("Invalid HLS playlist".into());
    }
    let mut out = String::new();
    for line in text.lines() {
        if line.starts_with('#') {
            let mut rest = line;
            while let Some(index) = rest.find("URI=\"") {
                out.push_str(&rest[..index + 5]);
                rest = &rest[index + 5..];
                let end = rest.find('"').ok_or("Invalid HLS URI")?;
                out.push_str(&register(scope, base, &rest[..end])?);
                out.push('"');
                rest = &rest[end + 1..];
            }
            out.push_str(rest);
        } else if !line.trim().is_empty() {
            out.push_str(&register(scope, base, line.trim())?);
        }
        out.push('\n');
    }
    Ok(out)
}
fn tunnel_url(base: &str, canonical: &Url) -> Result<Url> {
    let mut target = Url::parse(base).map_err(|_| "Invalid direct media bridge")?;
    target.set_path(&format!(
        "{}{}",
        target.path().trim_end_matches('/'),
        canonical.path()
    ));
    target.set_query(canonical.query());
    Ok(target)
}
async fn media(State(scope): State<Arc<Scope>>, request: Request<Body>) -> Response<Body> {
    let mut cancel = scope.stop.subscribe();
    if *cancel.borrow() {
        return empty(StatusCode::GONE);
    }
    if request.headers().get("Host").and_then(|v| v.to_str().ok()) != Some(&scope.host)
        || request
            .headers()
            .get("Origin")
            .is_some_and(|v| v.to_str().ok() != Some(&format!("http://{}", scope.host)))
        || !matches!(request.method().as_str(), "GET" | "HEAD")
        || request.uri().query().is_some()
    {
        return empty(StatusCode::FORBIDDEN);
    }
    let Some(url) = scope
        .resources
        .lock()
        .unwrap()
        .urls
        .get(request.uri().path())
        .cloned()
    else {
        return empty(StatusCode::NOT_FOUND);
    };
    let Ok(permit) = scope.slots.clone().try_acquire_owned() else {
        return empty(StatusCode::TOO_MANY_REQUESTS);
    };
    let mut fetch = url.clone();
    if let Some(tunnel) = &scope.tunnel {
        let Ok(target) = tunnel_url(tunnel, &url) else {
            return empty(StatusCode::BAD_GATEWAY);
        };
        fetch = target;
    }
    let Ok(auth) = protocol::authorization(&scope.session.token) else {
        return empty(StatusCode::FORBIDDEN);
    };
    let mut builder = scope.http.request(request.method().clone(), fetch);
    if scope.tunnel.is_none() {
        builder = builder.header("Authorization", auth);
    }
    if let Some(range) = request.headers().get("Range") {
        if range.len() > 128 {
            return empty(StatusCode::RANGE_NOT_SATISFIABLE);
        }
        builder = builder.header("Range", range);
    }
    let response = tokio::select! { _=cancel.changed()=>return empty(StatusCode::GONE),r=tokio::time::timeout(IDLE,builder.send())=>match r{Ok(Ok(r))=>r,_=>return empty(StatusCode::BAD_GATEWAY)}};
    let code = response.status();
    if !(code.is_success() || code == StatusCode::RANGE_NOT_SATISFIABLE) {
        return empty(code);
    }
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("Cache-Control", HeaderValue::from_static("no-store"));
    headers.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff"),
    );
    for name in [
        "content-type",
        "content-length",
        "content-range",
        "accept-ranges",
    ] {
        if let Some(v) = response.headers().get(name) {
            headers.insert(name, v.clone());
        }
    }
    if request.method() == "HEAD" {
        return Response::builder()
            .status(code)
            .body(Body::empty())
            .unwrap();
    }
    let is_hls = url.path().ends_with(".m3u8")
        || response
            .headers()
            .get("Content-Type")
            .is_some_and(|v| v.to_str().unwrap_or("").contains("mpegurl"));
    let body = if is_hls {
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        loop {
            let chunk = tokio::select! {_=cancel.changed()=>return empty(StatusCode::GONE),r=tokio::time::timeout(IDLE,stream.next())=>match r{Ok(r)=>r,Err(_)=>return empty(StatusCode::GATEWAY_TIMEOUT)}};
            match chunk {
                Some(Ok(chunk)) if bytes.len() + chunk.len() <= 1024 * 1024 => {
                    bytes.extend_from_slice(&chunk)
                }
                None => break,
                _ => return empty(StatusCode::BAD_GATEWAY),
            }
        }
        let Ok(text) = String::from_utf8(bytes) else {
            return empty(StatusCode::BAD_GATEWAY);
        };
        let Ok(rewritten) = manifest(&scope, &url, &text) else {
            return empty(StatusCode::BAD_GATEWAY);
        };
        headers.remove("content-length");
        headers.insert(
            "content-type",
            HeaderValue::from_static("application/vnd.apple.mpegurl"),
        );
        Body::from(rewritten)
    } else {
        let stream = futures_util::stream::unfold(
            (response.bytes_stream(), cancel, permit),
            |(mut stream, mut cancel, permit)| async move {
                if *cancel.borrow() {
                    return None;
                }
                let chunk = tokio::select! {_=cancel.changed()=>return None,r=tokio::time::timeout(IDLE,stream.next())=>match r{Ok(Some(Ok(chunk)))=>Ok(chunk),Ok(None)=>return None,_=>Err(std::io::Error::other("Media read interrupted or idle"))}};
                Some((chunk, (stream, cancel, permit)))
            },
        );
        Body::from_stream(stream)
    };
    let mut result = Response::new(body);
    *result.status_mut() = code;
    *result.headers_mut() = headers;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cleanup_requires_completed_successful_receipt() {
        for (done, receipt, accepted) in [
            (false, None, false),
            (true, None, false),
            (true, Some(Err("synthetic drain failure".into())), false),
            (true, Some(Ok(())), true),
        ] {
            let bridge = Bridge {
                url: String::new(),
                stop: watch::channel(false).0,
                stopped: Arc::new(AtomicBool::new(done)),
                receipt: Arc::new(Mutex::new(receipt)),
            };
            assert_eq!(bridge.cleanup_receipt().is_ok(), accepted);
        }
    }
    #[test]
    #[cfg(feature = "direct")]
    fn direct_bridge_stop_waits_for_transport_retirement() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let peer = runtime
            .block_on(bluefin_iroh_transport::endpoint())
            .unwrap();
        let session = Session {
            base: Url::parse("https://example.invalid/").unwrap(),
            token: "synthetic-token".into(),
            user: uuid::Uuid::from_u128(1),
        };
        let plan = Plan {
            item: protocol::Item {
                id: uuid::Uuid::from_u128(1),
                name: "Original test".into(),
                kind: "Audio".into(),
                overview: String::new(),
                start_date: None,
                end_date: None,
                channel_name: None,
                channel_id: None,
                        ..Default::default()
            },
            url: session.base.join("Audio/1/stream.wav").unwrap(),
            prefix: "/Audio/1/".into(),
            play_session: "synthetic".into(),
            media_source: "synthetic".into(),
            live_stream: None,
            method: "DirectPlay".into(),
            hls: false,
            live: false,
            offset: 0.0,
            selection: Default::default(),
            tracks: vec![],
        };
        let bridge = Bridge::start(
            session,
            plan,
            Some((peer.id().to_string(), "127.0.0.1:9".parse().unwrap())),
        )
        .unwrap();
        assert!(bridge.cleanup_receipt().is_err());
        bridge.stop();
        let started = std::time::Instant::now();
        while !bridge.stopped() {
            assert!(started.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
        bridge.cleanup_receipt().unwrap();
        bridge.stop();
        bridge.cleanup_receipt().unwrap();
        let local = Url::parse(&bridge.url).unwrap();
        assert!(std::net::TcpStream::connect(("127.0.0.1", local.port().unwrap())).is_err());
        runtime.block_on(peer.close());
    }
    #[test]
    fn direct_route_preserves_opaque_adapter_capability() {
        let canonical =
            url::Url::parse("https://server.example/root/Videos/abc/stream?Static=true").unwrap();
        let route =
            super::tunnel_url("http://127.0.0.1:1234/opaque-capability", &canonical).unwrap();
        assert_eq!(route.path(), "/opaque-capability/root/Videos/abc/stream");
        assert_eq!(route.query(), Some("Static=true"));
    }
}
