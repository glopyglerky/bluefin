//! Synthetic backend, real axum routes. No production accounts or media.
use async_trait::async_trait;
use axum::{
    body::{Body, Bytes, to_bytes},
    http::{Method, Request, StatusCode, header},
    response::Response,
};
use bluefin_tv_playback::{
    LeaseRequest,
    server::{Admission, Failure, PlaybackBackend, routes},
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;
const ITEM: &str = "00000000000000000000000000000001";
const SESSION: &str = "00000000000000000000000000000002";
#[derive(Clone)]
struct Backend {
    denied: Arc<AtomicUsize>,
    stops: Arc<AtomicUsize>,
    manifest: Option<String>,
    slow: bool,
}
impl Backend {
    fn new() -> Self {
        Self {
            denied: Arc::new(AtomicUsize::new(0)),
            stops: Arc::new(AtomicUsize::new(0)),
            manifest: None,
            slow: false,
        }
    }
}
#[async_trait]
impl PlaybackBackend for Backend {
    type Session = ();
    async fn admit(&self, t: &Request<()>, r: LeaseRequest) -> Result<Admission<()>, Failure> {
        if t.headers()
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            != Some("MediaBrowser Token=fixture")
        {
            return Err(StatusCode::UNAUTHORIZED);
        }
        self.reauthorize(&(), t).await?;
        Ok(Admission {
            session: (),
            viewer: "viewer".into(),
            device: "device".into(),
            item_id: r.item_id,
            play_session_id: SESSION.into(),
            root_uri: format!(
                "/Videos/{ITEM}/{}?playsessionid={SESSION}",
                if self.manifest.is_some() {
                    "master.m3u8"
                } else {
                    "stream.mp4"
                }
            ),
        })
    }
    async fn reauthorize(&self, _: &(), t: &Request<()>) -> Result<(), Failure> {
        if t.headers()
            .get("x-real-fixture")
            .and_then(|h| h.to_str().ok())
            != Some("remote")
        {
            return Err(StatusCode::FORBIDDEN);
        }
        match self.denied.load(Ordering::SeqCst) {
            0 => Ok(()),
            1 | 2 => Err(StatusCode::UNAUTHORIZED),
            3 | 4 => Err(StatusCode::FORBIDDEN),
            _ => Err(StatusCode::NOT_FOUND),
        }
    }
    async fn serve(
        &self,
        _: &(),
        t: &Request<()>,
        uri: &str,
        method: Method,
    ) -> Result<Response, Failure> {
        if uri.contains("master.m3u8") {
            return Ok(Response::new(Body::from(self.manifest.clone().unwrap())));
        }
        if self.slow {
            let stream = futures_util::stream::unfold(0, |n| async move {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                Some((Ok::<_, std::io::Error>(Bytes::from_static(b"x")), n + 1))
            });
            return Ok(Response::new(Body::from_stream(stream)));
        }
        let mut response = Response::new(if method == Method::HEAD {
            Body::empty()
        } else if t.headers().contains_key(header::RANGE) {
            Body::from("234")
        } else {
            Body::from("0123456789")
        });
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("video/mp4"),
        );
        if t.headers().contains_key(header::RANGE) {
            *response.status_mut() = StatusCode::PARTIAL_CONTENT;
            response.headers_mut().insert(
                header::CONTENT_RANGE,
                header::HeaderValue::from_static("bytes 2-4/10"),
            );
            response.headers_mut().insert(
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("3"),
            );
        } else {
            response.headers_mut().insert(
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("10"),
            );
        }
        Ok(response)
    }
    async fn release(&self, _: &()) {
        self.stops.fetch_add(1, Ordering::SeqCst);
    }
}
fn req(method: &str, path: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("x-real-fixture", "remote")
        .body(body)
        .unwrap()
}
async fn create(app: &axum::Router) -> Value {
    let mut r = req(
        "POST",
        "/Bluefin/TV/v1/leases",
        Body::from(
            json!({"version":1,"itemId":ITEM,"playback":{"kind":"video","maxBitrate":8000000}})
                .to_string(),
        ),
    );
    r.headers_mut().insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_static("MediaBrowser Token=fixture"),
    );
    let response = app.clone().oneshot(r).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap()
}
#[tokio::test]
async fn range_head_and_idempotent_stop_preserve_no_store() {
    let backend = Backend::new();
    let stops = backend.stops.clone();
    let app: axum::Router = routes(backend);
    let lease = create(&app).await;
    let path = lease["mediaUrl"].as_str().unwrap();
    assert!(!path.contains("fixture"));
    for method in ["GET", "HEAD"] {
        let mut r = req(method, path, Body::empty());
        r.headers_mut()
            .insert(header::RANGE, header::HeaderValue::from_static("bytes=2-4"));
        let response = app.clone().oneshot(r).await.unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 2-4/10");
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "no-store, private"
        );
        assert_eq!(response.headers()["referrer-policy"], "no-referrer");
        let body = to_bytes(response.into_body(), 32).await.unwrap();
        assert_eq!(
            body.as_ref(),
            if method == "HEAD" {
                &b""[..]
            } else {
                &b"234"[..]
            }
        );
    }
    for _ in 0..2 {
        assert_eq!(
            app.clone()
                .oneshot(req(
                    "DELETE",
                    lease["releaseUrl"].as_str().unwrap(),
                    Body::empty()
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(stops.load(Ordering::SeqCst), 1);
    assert_eq!(
        app.oneshot(req("GET", path, Body::empty()))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}
#[tokio::test]
async fn every_request_rechecks_revocation_disabled_schedule_remote_and_folder() {
    for denial in 1..=5 {
        for method in ["GET", "HEAD"] {
            let backend = Backend::new();
            let denied = backend.denied.clone();
            let app: axum::Router = routes(backend);
            let lease = create(&app).await;
            denied.store(denial, Ordering::SeqCst);
            let response = app
                .oneshot(req(
                    method,
                    lease["mediaUrl"].as_str().unwrap(),
                    Body::empty(),
                ))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                match denial {
                    1 | 2 => StatusCode::UNAUTHORIZED,
                    3 | 4 => StatusCode::FORBIDDEN,
                    _ => StatusCode::NOT_FOUND,
                }
            );
        }
    }
}
#[tokio::test]
async fn cookies_api_keys_unknown_selectors_and_cross_instance_grants_fail() {
    let app: axum::Router = routes(Backend::new());
    for header in [None, Some("ApiKey fixture"), Some("Bearer fixture")] {
        let mut r = req(
            "POST",
            "/Bluefin/TV/v1/leases",
            Body::from(
                json!({"version":1,"itemId":ITEM,"playback":{"kind":"video","maxBitrate":8000000}})
                    .to_string(),
            ),
        );
        r.headers_mut().insert(
            header::COOKIE,
            header::HeaderValue::from_static("Token=fixture"),
        );
        if let Some(h) = header {
            r.headers_mut().insert(
                header::AUTHORIZATION,
                header::HeaderValue::from_str(h).unwrap(),
            );
        }
        assert_eq!(
            app.clone().oneshot(r).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let r=req("POST","/Bluefin/TV/v1/leases",Body::from(json!({"version":1,"itemId":ITEM,"userId":"administrator","playback":{"kind":"video","maxBitrate":8000000}}).to_string()));
    assert_eq!(
        app.clone().oneshot(r).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    let lease = create(&app).await;
    let other: axum::Router = routes(Backend::new());
    assert_eq!(
        other
            .oneshot(req(
                "GET",
                lease["mediaUrl"].as_str().unwrap(),
                Body::empty()
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}
#[tokio::test]
async fn hls_child_and_init_are_rewritten_then_rechecked() {
    let mut backend = Backend::new();
    backend.manifest =
        Some("#EXTM3U\n#EXT-X-MAP:URI=\"hls1/main/-1.mp4\"\n#EXTINF:6,\nhls1/main/0.ts\n".into());
    let denied = backend.denied.clone();
    let app: axum::Router = routes(backend);
    let lease = create(&app).await;
    let response = app
        .clone()
        .oneshot(req(
            "GET",
            lease["mediaUrl"].as_str().unwrap(),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body =
        String::from_utf8(to_bytes(response.into_body(), 8192).await.unwrap().to_vec()).unwrap();
    assert!(!body.contains("playsessionid"));
    assert!(body.contains("/1\""));
    let child = body.lines().last().unwrap();
    assert_eq!(
        app.clone()
            .oneshot(req("HEAD", child, Body::empty()))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    denied.store(5, Ordering::SeqCst);
    assert_eq!(
        app.oneshot(req("GET", child, Body::empty()))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}
#[tokio::test]
async fn hls_external_traversal_encoded_and_session_escape_fail_closed() {
    for reference in [
        "https://example.com/x",
        "//example.com/x",
        "../main.m3u8",
        "%2e%2e/main.m3u8",
        "hls1/main/0.ts?playsessionid=other",
        "hls1/main/0.ts?api_key=secret",
    ] {
        let mut backend = Backend::new();
        backend.manifest = Some(format!("#EXTM3U\n#EXTINF:6,\n{reference}\n"));
        let app: axum::Router = routes(backend);
        let lease = create(&app).await;
        assert_eq!(
            app.oneshot(req(
                "GET",
                lease["mediaUrl"].as_str().unwrap(),
                Body::empty()
            ))
            .await
            .unwrap()
            .status(),
            StatusCode::BAD_GATEWAY,
            "{reference}"
        );
    }
}
#[tokio::test(start_paused = true)]
async fn abandoned_grants_expire_and_clean_exactly_once() {
    let backend = Backend::new();
    let stops = backend.stops.clone();
    let app: axum::Router = routes(backend);
    let lease = create(&app).await;
    tokio::time::advance(std::time::Duration::from_secs(93)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        app.clone()
            .oneshot(req(
                "GET",
                lease["mediaUrl"].as_str().unwrap(),
                Body::empty()
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    tokio::time::advance(std::time::Duration::from_secs(3)).await;
    tokio::task::yield_now().await;
    assert_eq!(stops.load(Ordering::SeqCst), 1);
}
#[tokio::test(start_paused = true)]
async fn active_stream_cancels_on_policy_revocation() {
    let mut backend = Backend::new();
    backend.slow = true;
    let denied = backend.denied.clone();
    let app: axum::Router = routes(backend);
    let lease = create(&app).await;
    let response = app
        .oneshot(req(
            "GET",
            lease["mediaUrl"].as_str().unwrap(),
            Body::empty(),
        ))
        .await
        .unwrap();
    let collect = tokio::spawn(to_bytes(response.into_body(), 1024));
    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    tokio::task::yield_now().await;
    denied.store(1, Ordering::SeqCst);
    tokio::time::advance(std::time::Duration::from_secs(7)).await;
    tokio::task::yield_now().await;
    assert!(collect.await.unwrap().unwrap().len() < 10);
}
#[tokio::test]
async fn per_device_lease_limit_is_enforced() {
    let app: axum::Router = routes(Backend::new());
    create(&app).await;
    create(&app).await;
    let mut r = req(
        "POST",
        "/Bluefin/TV/v1/leases",
        Body::from(
            json!({"version":1,"itemId":ITEM,"playback":{"kind":"video","maxBitrate":8000000}})
                .to_string(),
        ),
    );
    r.headers_mut().insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_static("MediaBrowser Token=fixture"),
    );
    assert_eq!(
        app.oneshot(r).await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}
#[tokio::test]
async fn dropped_readers_release_connection_slots() {
    let mut backend = Backend::new();
    backend.slow = true;
    let app: axum::Router = routes(backend);
    let lease = create(&app).await;
    let path = lease["mediaUrl"].as_str().unwrap();
    let mut responses = Vec::new();
    for _ in 0..8 {
        let response = app
            .clone()
            .oneshot(req("GET", path, Body::empty()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        responses.push(response);
    }
    assert_eq!(
        app.clone()
            .oneshot(req("GET", path, Body::empty()))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    drop(responses);
    for _ in 0..3 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        app.oneshot(req("GET", path, Body::empty()))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}
#[tokio::test]
async fn hls_key_request_and_encoded_session_ambiguity_are_checked() {
    let mut backend = Backend::new();
    backend.manifest = Some(
        "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"hls1/main/1.key\"\n#EXTINF:6,\nhls1/main/0.ts\n"
            .into(),
    );
    let denied = backend.denied.clone();
    let app: axum::Router = routes(backend);
    let lease = create(&app).await;
    let response = app
        .clone()
        .oneshot(req(
            "GET",
            lease["mediaUrl"].as_str().unwrap(),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text =
        String::from_utf8(to_bytes(response.into_body(), 8192).await.unwrap().to_vec()).unwrap();
    let key = text
        .split("URI=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    denied.store(1, Ordering::SeqCst);
    assert_eq!(
        app.oneshot(req("HEAD", key, Body::empty()))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}
struct RacingBackend {
    inner: Backend,
    started: Arc<tokio::sync::Notify>,
    finish: Arc<tokio::sync::Notify>,
}
#[async_trait]
impl PlaybackBackend for RacingBackend {
    type Session = ();
    async fn admit(&self, t: &Request<()>, r: LeaseRequest) -> Result<Admission<()>, Failure> {
        self.inner.admit(t, r).await
    }
    async fn reauthorize(&self, s: &(), t: &Request<()>) -> Result<(), Failure> {
        self.inner.reauthorize(s, t).await
    }
    async fn serve(
        &self,
        s: &(),
        t: &Request<()>,
        uri: &str,
        method: Method,
    ) -> Result<Response, Failure> {
        self.started.notify_one();
        self.finish.notified().await;
        self.inner.serve(s, t, uri, method).await
    }
    async fn release(&self, s: &()) {
        self.inner.release(s).await
    }
}
#[tokio::test]
async fn release_waits_for_inflight_dispatch_then_stops_and_refuses_late_requests() {
    let inner = Backend::new();
    let stops = inner.stops.clone();
    let started = Arc::new(tokio::sync::Notify::new());
    let finish = Arc::new(tokio::sync::Notify::new());
    let app: axum::Router = routes(RacingBackend {
        inner,
        started: started.clone(),
        finish: finish.clone(),
    });
    let lease = create(&app).await;
    let path = lease["mediaUrl"].as_str().unwrap();
    let media = tokio::spawn(app.clone().oneshot(req("GET", path, Body::empty())));
    started.notified().await;
    let release = tokio::spawn(app.clone().oneshot(req(
        "DELETE",
        lease["releaseUrl"].as_str().unwrap(),
        Body::empty(),
    )));
    for _ in 0..3 {
        tokio::task::yield_now().await;
    }
    assert_eq!(stops.load(Ordering::SeqCst), 0);
    assert_eq!(
        app.clone()
            .oneshot(req("GET", path, Body::empty()))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    finish.notify_one();
    assert_eq!(
        media.await.unwrap().unwrap().status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        release.await.unwrap().unwrap().status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(stops.load(Ordering::SeqCst), 1);
}
#[tokio::test(start_paused = true)]
async fn stalled_readers_release_slots_without_waiting_for_lease_expiry() {
    let mut backend = Backend::new();
    backend.slow = true;
    let app: axum::Router = routes(backend);
    let lease = create(&app).await;
    let path = lease["mediaUrl"].as_str().unwrap();
    let mut responses = Vec::new();
    for _ in 0..8 {
        responses.push(
            app.clone()
                .oneshot(req("GET", path, Body::empty()))
                .await
                .unwrap(),
        );
    }
    for _ in 0..20 {
        tokio::time::advance(std::time::Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
    }
    assert_eq!(
        app.oneshot(req("GET", path, Body::empty()))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    drop(responses);
}
