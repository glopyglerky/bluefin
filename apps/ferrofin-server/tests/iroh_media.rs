//! Built-in Iroh against real production composition and synthetic local media.
//! No installed service, real account, external provider or client source edits.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use bluefin_iroh_server::{ALPN, Config as IrohConfig, Server, StopReceipt};
use ferrofin_db::Database;
use ferrofin_server::{
    config::Config,
    state::{WiredApp, build_app_state},
};
use ferrofin_traits::library::ScanTrigger;
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::{
    io::Write,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    os::unix::fs::PermissionsExt,
    path::Path,
    time::Duration,
};
use tokio::{io::AsyncReadExt, sync::watch};
use tower::ServiceExt;

#[path = "iroh_media/permissions.rs"]
mod permissions;

#[path = "iroh_media/migration.rs"]
mod migration;
#[cfg(target_os = "linux")]
#[path = "iroh_media/process.rs"]
mod process;

const CLIENT: &str =
    r#"MediaBrowser Client="iroh-fixture", Device="fixture", DeviceId="iroh-fixture", Version="1""#;
fn auth(token: &str) -> String {
    format!("{CLIENT}, Token=\"{token}\"")
}
fn ffmpeg_binary() -> std::ffi::OsString {
    std::env::var_os("BLUEFIN_TEST_FFMPEG").unwrap_or_else(|| "ffmpeg".into())
}
async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    token: &str,
    value: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header(header::AUTHORIZATION, auth(token))
                .header(header::CONTENT_TYPE, "application/json")
                .body(if value.is_null() {
                    Body::empty()
                } else {
                    Body::from(value.to_string())
                })
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn login(app: &Router, name: &str, password: &str) -> (String, String) {
    let (status, value) = call(
        app,
        "POST",
        "/Users/AuthenticateByName",
        "",
        json!({"Username":name,"Pw":password}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "login {value}");
    (
        value["AccessToken"].as_str().unwrap().into(),
        value["User"]["Id"].as_str().unwrap().into(),
    )
}
fn wav(path: &Path, size: usize) -> [u8; 32] {
    let mut f = std::fs::File::create(path).unwrap();
    let size = u32::try_from(size).unwrap();
    let mut header = Vec::from(&b"RIFF"[..]);
    header.extend((size - 8).to_le_bytes());
    header.extend(b"WAVEfmt ");
    header.extend(16u32.to_le_bytes());
    header.extend(1u16.to_le_bytes());
    header.extend(1u16.to_le_bytes());
    header.extend(48000u32.to_le_bytes());
    header.extend(96000u32.to_le_bytes());
    header.extend(2u16.to_le_bytes());
    header.extend(16u16.to_le_bytes());
    header.extend(b"data");
    header.extend((size - 44).to_le_bytes());
    let mut hash = Sha256::new();
    f.write_all(&header).unwrap();
    hash.update(&header);
    let block = (0..32768)
        .map(|i| u8::try_from((i * 31 + 7) % 251).unwrap())
        .collect::<Vec<_>>();
    let mut left = (size - 44) as usize;
    while left > 0 {
        let n = left.min(block.len());
        f.write_all(&block[..n]).unwrap();
        hash.update(&block[..n]);
        left -= n;
    }
    hash.finalize().into()
}
struct Fixture {
    app: Router,
    wired: WiredApp,
    db: Database,
    token: String,
    admin: String,
    viewer: String,
    item: String,
    hash: [u8; 32],
    size: usize,
    temp: tempfile::TempDir,
}
impl Fixture {
    async fn new(size: usize) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700)).unwrap();
        for dir in ["config", "data", "cache", "music", "identity"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::set_permissions(
            root.join("identity"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let hash = wav(&root.join("music/Original.wav"), size);
        let config = Config {
            admin_user: "iroh-admin".into(),
            admin_password: "synthetic-password".into(),
            ffmpeg_path: std::env::var_os("BLUEFIN_TEST_FFMPEG").map(Into::into),
            ffprobe_path: std::env::var_os("BLUEFIN_TEST_FFPROBE").map(Into::into),
            ..Config::test_stub(root)
        };
        let db = Database::connect(&config.database_url()).await.unwrap();
        db.run_migrations().await.unwrap();
        let ffmpeg = ferrofin_server::bootstrap::discover_ffmpeg(&config, None)
            .await
            .unwrap();
        let (shutdown, _) = tokio::sync::oneshot::channel();
        let wired = build_app_state(&db, &config, &ffmpeg, None, shutdown)
            .await
            .unwrap();
        ferrofin_server::seed::seed_default_admin(wired.state.users.as_ref(), &config)
            .await
            .unwrap();
        let app = ferrofin_api::create_router(wired.state.clone());
        let (admin, _) = login(&app, "iroh-admin", "synthetic-password").await;
        let uri = format!(
            "/Library/VirtualFolders?name=Music&collectionType=music&paths={}&refreshLibrary=false",
            root.join("music").display()
        );
        assert_eq!(
            call(&app, "POST", &uri, &admin, Value::Null).await.0,
            StatusCode::NO_CONTENT
        );
        assert!(
            wired
                .state
                .library
                .run_library_scan(ScanTrigger::Api, None)
                .await
                .unwrap()
        );
        assert_eq!(
            call(
                &app,
                "POST",
                "/Users/New",
                &admin,
                json!({"Name":"iroh-viewer","Password":"viewer-password"})
            )
            .await
            .0,
            StatusCode::OK
        );
        let (token, viewer) = login(&app, "iroh-viewer", "viewer-password").await;
        let (status, items) = call(
            &app,
            "GET",
            "/Items?recursive=true&includeItemTypes=Audio",
            &token,
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let item = items["Items"][0]["Id"].as_str().unwrap().to_owned();
        Self {
            app,
            wired,
            db,
            token,
            admin,
            viewer,
            item,
            hash,
            size,
            temp,
        }
    }
    fn config(&self) -> IrohConfig {
        IrohConfig {
            secret_key_file: self.temp.path().join("identity/key"),
            bind_address: "127.0.0.1:0".parse().unwrap(),
        }
    }
    async fn start(&self) -> Running {
        let config = self.config();
        let server = Server::bind(&config).await.unwrap();
        let (id, address) = server.address();
        self.wired.state.set_iroh_media_transport(Some(
            bluefin_contract::transport::IrohMediaTransport {
                version: 1,
                endpoint_id: id.clone(),
                direct_addresses: vec![address.to_string()],
                alpn: "bluefin/http/1".into(),
                direct_only: true,
            },
        ));
        let active = server.active_connections();
        let (stop, rx) = watch::channel(false);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let capture = seen.clone();
        let app = ferrofin_api::iroh_media::router(self.wired.state.clone(), self.app.clone())
            .layer(axum::middleware::from_fn(
                move |request: axum::extract::Request, next: axum::middleware::Next| {
                    let capture = capture.clone();
                    async move {
                        if let Some(peer) = request
                            .extensions()
                            .get::<bluefin_iroh_server::DirectPeer>()
                        {
                            capture.lock().unwrap().push(*peer);
                        }
                        next.run(request).await
                    }
                },
            ));
        let state = self.wired.state.clone();
        Running {
            id: id.parse().unwrap(),
            address,
            stop,
            active,
            seen,
            job: tokio::spawn(async move {
                let r = server.run(app, rx).await;
                state.set_iroh_media_transport(None);
                r
            }),
        }
    }
    fn path(&self) -> String {
        format!("/Audio/{}/stream?Static=true", self.item)
    }
    async fn policy(&self, changes: Value) {
        let uri = format!("/Users/{}/Policy", self.viewer);
        let (_, me) = call(
            &self.app,
            "GET",
            &format!("/Users/{}", self.viewer),
            &self.admin,
            Value::Null,
        )
        .await;
        let mut policy = me["Policy"].clone();
        policy
            .as_object_mut()
            .unwrap()
            .extend(changes.as_object().unwrap().clone());
        let (status, body) = call(&self.app, "POST", &uri, &self.admin, policy).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    }
    async fn video(&self) -> String {
        let root = self.temp.path();
        std::fs::create_dir(root.join("videos")).unwrap();
        let output = root.join("videos/Original.mp4");
        let status = std::process::Command::new(ffmpeg_binary())
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:size=160x90:rate=12",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "3",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-g",
                "24",
                "-c:a",
                "aac",
                "-y",
            ])
            .arg(&output)
            .status()
            .unwrap();
        assert!(status.success());
        let uri = format!(
            "/Library/VirtualFolders?name=Videos&collectionType=homevideos&paths={}&refreshLibrary=false",
            root.join("videos").display()
        );
        assert_eq!(
            call(&self.app, "POST", &uri, &self.admin, Value::Null)
                .await
                .0,
            StatusCode::NO_CONTENT
        );
        assert!(
            self.wired
                .state
                .library
                .run_library_scan(ScanTrigger::Api, None)
                .await
                .unwrap()
        );
        let (_, items) = call(
            &self.app,
            "GET",
            "/Items?recursive=true&includeItemTypes=Video",
            &self.token,
            Value::Null,
        )
        .await;
        items["Items"][0]["Id"].as_str().unwrap().into()
    }
    async fn close(mut self) {
        self.wired.state.library.shutdown_scans().await;
        self.wired.task_manager.shutdown().await;
        for task in &self.wired.background {
            task.abort();
        }
        for task in self.wired.background.drain(..) {
            let _ = task.await;
        }
        drop(self.app);
        drop(self.wired);
        self.db.close().await;
    }
}

async fn check_video(f: &Fixture, ep: &Endpoint, server: &Running) {
    let video = f.video().await;
    let (status, _, bytes) = response(
        ep,
        server,
        &raw(
            "GET",
            &format!("/Videos/{video}/stream?Static=true"),
            &auth(&f.token),
            "",
        ),
    )
    .await
    .unwrap();
    assert_eq!(status, 200);
    assert_eq!(
        bytes,
        std::fs::read(f.temp.path().join("videos/Original.mp4")).unwrap()
    );
    for path in [
        format!("/Audio/{video}/stream?Static=true"),
        format!("/Videos/{}/stream?Static=true", f.item),
    ] {
        assert_eq!(
            response(ep, server, &raw("GET", &path, &auth(&f.token), ""))
                .await
                .unwrap()
                .0,
            404,
            "actual source kind cannot be reclassified by a route prefix"
        );
    }
    let path = format!(
        "/Videos/{video}/main.m3u8?VideoCodec=h264&AudioCodec=aac&SegmentContainer=ts&PlaySessionId=iroh-video&DeviceId=iroh-fixture"
    );
    let (status, _, bytes) = response(ep, server, &raw("GET", &path, &auth(&f.token), ""))
        .await
        .unwrap();
    assert_eq!(status, 200);
    assert!(bytes.starts_with(b"#EXTM3U"));
    assert_eq!(
        call(
            &f.app,
            "DELETE",
            "/Videos/ActiveEncodings?DeviceId=iroh-fixture&PlaySessionId=iroh-video",
            &f.token,
            Value::Null
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn ordinary_hls_and_source_bound_tv_use_production_handlers_and_explicit_stop() {
    let f = Fixture::new(512 * 1024).await;
    let server = f.start().await;
    let ep = endpoint(IpAddr::V4(Ipv4Addr::LOCALHOST)).await;
    let query = "AudioCodec=aac&SegmentContainer=ts&PlaySessionId=iroh-hls&DeviceId=iroh-fixture";
    let main = format!("/Audio/{}/main.m3u8?{query}", f.item);
    let (status, _, playlist) = response(&ep, &server, &raw("GET", &main, &auth(&f.token), ""))
        .await
        .unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&playlist));
    let playlist = String::from_utf8(playlist).unwrap();
    assert!(playlist.starts_with("#EXTM3U"));
    let segment = playlist
        .lines()
        .find(|l| !l.starts_with('#') && !l.is_empty())
        .unwrap();
    let segment = format!("/Audio/{}/{segment}", f.item);
    let (status, _, bytes) = response(&ep, &server, &raw("GET", &segment, &auth(&f.token), ""))
        .await
        .unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
    assert!(bytes.len() > 4096);
    let path = f.temp.path().join("transport-segment.ts");
    std::fs::write(&path, &bytes).unwrap();
    let decoded = std::process::Command::new(ffmpeg_binary())
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(&path)
        .args(["-f", "md5", "-"])
        .output()
        .unwrap();
    assert!(
        decoded.status.success(),
        "actual HLS segment decodes: {}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    assert_eq!(
        call(
            &f.app,
            "DELETE",
            "/Videos/ActiveEncodings?DeviceId=iroh-fixture&PlaySessionId=iroh-hls",
            &f.token,
            Value::Null
        )
        .await
        .0,
        StatusCode::NO_CONTENT,
        "ordinary explicit stop receipt"
    );
    check_video(&f, &ep, &server).await;
    // Admission remains the existing HTTPS/control-plane API. Iroh gets only
    // the already source-bound lease media resource, with ordinary viewer auth.
    let (status,lease)=call(&f.app,"POST","/Bluefin/TV/v1/leases",&f.token,json!({"version":1,"itemId":f.item,"playback":{"kind":"audio","maxBitrate":192_000,"startTicks":0}})).await;
    assert_eq!(status, StatusCode::CREATED, "{lease}");
    let media = lease["mediaUrl"].as_str().unwrap();
    let (status, _, body) = response(&ep, &server, &raw("GET", media, &auth(&f.token), ""))
        .await
        .unwrap();
    assert_eq!(status, 200);
    assert!(body.len() > 4096);
    assert_eq!(
        response(&ep, &server, &raw("GET", media, &auth(&f.admin), ""))
            .await
            .unwrap()
            .0,
        401,
        "incoming Iroh session must match the source-bound lease viewer/token/device"
    );
    assert_eq!(
        call(
            &f.app,
            "DELETE",
            lease["releaseUrl"].as_str().unwrap(),
            &f.token,
            Value::Null
        )
        .await
        .0,
        StatusCode::NO_CONTENT,
        "source-bound lease joined stop receipt"
    );
    assert_eq!(
        response(&ep, &server, &raw("GET", media, &auth(&f.token), ""))
            .await
            .unwrap()
            .0,
        404
    );
    server.stop().await;
    ep.close().await;
    f.close().await;
}

struct Https {
    base: String,
    client: reqwest::Client,
    stop: tokio::sync::oneshot::Sender<()>,
    job: tokio::task::JoinHandle<()>,
}
fn https(app: Router, client_ip: Option<IpAddr>) -> Https {
    let cert = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    let der = cert.cert.der().clone();
    let config = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![der.clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()).into(),
    )
    .unwrap();
    let listener = ferrofin_server::tls::TlsListener::new(
        ferrofin_server::listen::bind("127.0.0.1:0".parse().unwrap()).unwrap(),
        std::sync::Arc::new(config),
    )
    .unwrap();
    let address = listener.local_addr();
    let (stop, rx) = tokio::sync::oneshot::channel();
    let local = address;
    let app = app.layer(axum::middleware::from_fn(
        move |mut request: axum::extract::Request, next: axum::middleware::Next| async move {
            request
                .extensions_mut()
                .insert(ferrofin_api::ip_access::ConnectionLocalAddr(local));
            request
                .extensions_mut()
                .insert(ferrofin_api::forwarded_headers::Transport::Tls);
            next.run(request).await
        },
    ));
    let make = tower::service_fn(
        move |stream: axum::serve::IncomingStream<'_, ferrofin_server::listen::ServedListener>| {
            let peer = *stream.remote_addr();
            let app = app.clone();
            async move {
                Ok::<_, std::convert::Infallible>(
                    tower::ServiceBuilder::new()
                        .map_request(move |mut req: axum::extract::Request| {
                            req.extensions_mut()
                                .insert(axum::extract::ConnectInfo(peer));
                            req
                        })
                        .service(app),
                )
            }
        },
    );
    let job = tokio::spawn(async move {
        axum::serve(ferrofin_server::listen::ServedListener::Tls(listener), make)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await
            .unwrap();
    });
    let mut client = reqwest::Client::builder()
        .tls_certs_only([reqwest::Certificate::from_der(&der).unwrap()])
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30));
    if let Some(ip) = client_ip {
        client = client.local_address(ip);
    }
    Https {
        base: format!("https://{address}"),
        client: client.build().unwrap(),
        stop,
        job,
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn exact_https_and_iroh_bytes_network_and_classification_parity() {
    let f = Fixture::new(512 * 1024).await;
    let server = f.start().await;
    let ip = nonloopback();
    let ep = endpoint(ip).await;
    let https = https(f.app.clone(), Some(ip));
    let url = format!("{}{}", https.base, f.path());
    let r = https
        .client
        .get(&url)
        .header(header::AUTHORIZATION, auth(&f.token))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let expected = r.bytes().await.unwrap();
    let (status, _, bytes) = response(&ep, &server, &raw("GET", &f.path(), &auth(&f.token), ""))
        .await
        .unwrap();
    assert_eq!(status, 200);
    assert_eq!(bytes, expected);
    let r = https
        .client
        .get(&url)
        .header(header::AUTHORIZATION, auth(&f.token))
        .header(header::RANGE, "bytes=1024-2047")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 206);
    let expected = r.bytes().await.unwrap();
    let (status, _, bytes) = response(
        &ep,
        &server,
        &raw(
            "GET",
            &f.path(),
            &auth(&f.token),
            "Range: bytes=1024-2047\r\n",
        ),
    )
    .await
    .unwrap();
    assert_eq!(status, 206);
    assert_eq!(bytes, expected);
    f.policy(json!({"EnableAllFolders":false,"EnabledFolders":[]}))
        .await;
    let status = https
        .client
        .get(&url)
        .header(header::AUTHORIZATION, auth(&f.token))
        .send()
        .await
        .unwrap()
        .status();
    let result = response(&ep, &server, &raw("GET", &f.path(), &auth(&f.token), ""))
        .await
        .unwrap()
        .0;
    assert_eq!(status, 404);
    assert_eq!(result, 404);
    https.stop.send(()).unwrap();
    https.job.await.unwrap();
    server.stop().await;
    ep.close().await;
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn peer_pin_alpn_context_and_capability_disclosure_fail_closed() {
    let f = Fixture::new(512 * 1024).await;
    let (_, before) = call(
        &f.app,
        "GET",
        "/Bluefin/Capabilities",
        &f.token,
        Value::Null,
    )
    .await;
    assert!(before.get("irohMediaTransport").is_none());
    let mut server = f.start().await;
    let ep = endpoint(IpAddr::V4(Ipv4Addr::LOCALHOST)).await;
    let (_, during) = call(
        &f.app,
        "GET",
        "/Bluefin/Capabilities",
        &f.token,
        Value::Null,
    )
    .await;
    assert_eq!(
        during["irohMediaTransport"]["endpointId"],
        server.id.to_string()
    );
    assert_eq!(during["irohMediaTransport"]["directOnly"], true);
    let wrong = iroh::SecretKey::generate().public();
    let pin = tokio::time::timeout(
        Duration::from_secs(5),
        ep.connect(EndpointAddr::new(wrong).with_ip_addr(server.address), ALPN),
    )
    .await
    .unwrap();
    assert!(pin.is_err(), "TLS pin rejection");
    let alpn = tokio::time::timeout(
        Duration::from_secs(5),
        ep.connect(
            EndpointAddr::new(server.id).with_ip_addr(server.address),
            b"other/protocol",
        ),
    )
    .await
    .unwrap();
    assert!(alpn.is_err(), "ALPN rejection");
    assert!(
        server.seen.lock().unwrap().is_empty(),
        "failed handshakes never reached media router"
    );
    let app = ferrofin_api::iroh_media::router(f.wired.state.clone(), f.app.clone());
    for mismatch in [false, true] {
        let mut req = Request::builder()
            .uri(f.path())
            .header(header::AUTHORIZATION, auth(&f.token))
            .body(Body::empty())
            .unwrap();
        if mismatch {
            req.extensions_mut()
                .insert(bluefin_iroh_server::DirectPeer {
                    remote: "127.0.0.1:1".parse().unwrap(),
                    local: server.address,
                });
            req.extensions_mut().insert(axum::extract::ConnectInfo(
                "127.0.0.1:2".parse::<SocketAddr>().unwrap(),
            ));
        }
        assert_eq!(
            app.clone().oneshot(req).await.unwrap().status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "missing/mismatched peer cannot default to loopback"
        );
    }
    tokio::time::timeout(Duration::from_secs(5), server.active.wait_for(|n| *n == 0))
        .await
        .unwrap()
        .unwrap();
    f.policy(json!({"EnableMediaPlayback":false})).await;
    let (_, denied) = call(
        &f.app,
        "GET",
        "/Bluefin/Capabilities",
        &f.token,
        Value::Null,
    )
    .await;
    assert!(denied.get("irohMediaTransport").is_none());
    server.stop().await;
    let (_, after) = call(
        &f.app,
        "GET",
        "/Bluefin/Capabilities",
        &f.token,
        Value::Null,
    )
    .await;
    assert!(after.get("irohMediaTransport").is_none());
    ep.close().await;
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn stalled_body_is_closed_on_current_permission_and_source_revocation() {
    let mut f = Fixture::new(64 * 1024 * 1024).await;
    let mut server = f.start().await;
    let ep = endpoint(IpAddr::V4(Ipv4Addr::LOCALHOST)).await;
    let (conn, mut recv) = open(&ep, &server, &raw("GET", &f.path(), &auth(&f.token), ""))
        .await
        .unwrap();
    assert!(read_head(&mut recv).await.starts_with("HTTP/1.1 200 "));
    f.policy(json!({"EnableMediaPlayback":false})).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(5), conn.closed())
            .await
            .is_ok(),
        "revocation closes even without polling the backpressured body"
    );
    tokio::time::timeout(Duration::from_secs(5), server.active.wait_for(|n| *n == 0))
        .await
        .unwrap()
        .unwrap();
    drop(recv);
    drop(conn);
    f.policy(json!({"EnableMediaPlayback":true})).await;
    let (conn, mut recv) = open(&ep, &server, &raw("GET", &f.path(), &auth(&f.token), ""))
        .await
        .unwrap();
    assert!(read_head(&mut recv).await.starts_with("HTTP/1.1 200 "));
    f.policy(json!({"EnableAllDevices":false,"EnabledDevices":[]}))
        .await;
    assert!(
        tokio::time::timeout(Duration::from_secs(5), conn.closed())
            .await
            .is_ok(),
        "current persisted device policy revokes stalled output"
    );
    tokio::time::timeout(Duration::from_secs(5), server.active.wait_for(|n| *n == 0))
        .await
        .unwrap()
        .unwrap();
    drop(recv);
    drop(conn);
    f.policy(json!({"EnableAllDevices":true})).await;
    // Device-policy revocation retires the persisted session. Re-enabling
    // devices does not resurrect its old token; use an ordinary fresh login.
    f.token = login(&f.app, "iroh-viewer", "viewer-password").await.0;
    let (conn, mut recv) = open(&ep, &server, &raw("GET", &f.path(), &auth(&f.token), ""))
        .await
        .unwrap();
    assert!(read_head(&mut recv).await.starts_with("HTTP/1.1 200 "));
    assert_eq!(
        call(&f.app, "POST", "/Sessions/Logout", &f.token, Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(5), conn.closed())
            .await
            .is_ok(),
        "ordinary logout revokes the actual persisted token during backpressure"
    );
    tokio::time::timeout(Duration::from_secs(5), server.active.wait_for(|n| *n == 0))
        .await
        .unwrap()
        .unwrap();
    drop(recv);
    drop(conn);
    f.token = login(&f.app, "iroh-viewer", "viewer-password").await.0;
    let (conn, mut recv) = open(&ep, &server, &raw("GET", &f.path(), &auth(&f.token), ""))
        .await
        .unwrap();
    assert!(read_head(&mut recv).await.starts_with("HTTP/1.1 200 "));
    // Change the actual opened file. Iroh's additional file stamp detects this;
    // the production pin separately retains classification/policy authority.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(f.temp.path().join("music/Original.wav"))
        .unwrap();
    file.set_len((f.size - 4096) as u64).unwrap();
    drop(file);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), conn.closed())
            .await
            .is_ok(),
        "source generation revocation closes stalled output"
    );
    tokio::time::timeout(Duration::from_secs(5), server.active.wait_for(|n| *n == 0))
        .await
        .unwrap()
        .unwrap();
    drop(recv);
    drop(conn);
    server.stop().await;
    ep.close().await;
    f.close().await;
}
struct Running {
    id: EndpointId,
    address: SocketAddr,
    stop: watch::Sender<bool>,
    active: watch::Receiver<usize>,
    seen: std::sync::Arc<std::sync::Mutex<Vec<bluefin_iroh_server::DirectPeer>>>,
    job: tokio::task::JoinHandle<anyhow::Result<StopReceipt>>,
}
impl Running {
    async fn stop(self) -> StopReceipt {
        self.stop.send(true).unwrap();
        let r = self.job.await.unwrap().unwrap();
        assert!(r.endpoint_closed);
        r
    }
}
async fn endpoint(ip: IpAddr) -> Endpoint {
    let transport = iroh::endpoint::QuicTransportConfig::builder()
        .max_concurrent_bidi_streams(1u32.into())
        .max_concurrent_uni_streams(0u32.into())
        .stream_receive_window((8u32 * 1024 * 1024).into())
        .receive_window((16u32 * 1024 * 1024).into())
        .send_window(16 * 1024 * 1024)
        .keep_alive_interval(Duration::from_millis(200))
        .build();
    Endpoint::builder(iroh::endpoint::presets::Minimal)
        .clear_ip_transports()
        .clear_relay_transports()
        .clear_address_lookup()
        .bind_addr(SocketAddr::new(ip, 0))
        .unwrap()
        .relay_mode(RelayMode::Disabled)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .transport_config(transport)
        .bind()
        .await
        .unwrap()
}
async fn open(
    ep: &Endpoint,
    server: &Running,
    raw: &str,
) -> anyhow::Result<(iroh::endpoint::Connection, iroh::endpoint::RecvStream)> {
    // Include an unreachable advertised hint. Authorization must use the
    // actual selected path, independently of advertised address ordering.
    let addr = EndpointAddr::new(server.id)
        .with_ip_addr("192.0.2.1:9".parse().unwrap())
        .with_ip_addr(server.address);
    let conn = tokio::time::timeout(Duration::from_secs(5), ep.connect(addr, ALPN)).await??;
    let (mut send, recv) = conn.open_bi().await?;
    send.write_all(raw.as_bytes()).await?;
    send.finish()?;
    Ok((conn, recv))
}
fn raw(method: &str, path: &str, authorization: &str, extra: &str) -> String {
    format!(
        "{method} {path} HTTP/1.1\r\nHost: bluefin\r\nAuthorization: {authorization}\r\n{extra}\r\n"
    )
}
async fn response(
    ep: &Endpoint,
    server: &Running,
    raw: &str,
) -> anyhow::Result<(u16, String, Vec<u8>)> {
    let (conn, mut recv) = open(ep, server, raw).await?;
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        anyhow::ensure!(head.len() < 16 * 1024, "header limit");
        head.push(recv.read_u8().await?);
    }
    let head = String::from_utf8(head)?;
    let status = head.split_whitespace().nth(1).unwrap().parse()?;
    let mut bytes = Vec::new();
    recv.read_to_end(1024 * 1024).await.map(|v| bytes = v)?;
    conn.close(0u32.into(), b"application drained");
    Ok((status, head, bytes))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn real_authenticated_exact_bytes_ranges_head_and_current_authority() {
    let f = Fixture::new(512 * 1024).await;
    let server = f.start().await;
    let id = server.id;
    let ep = endpoint(IpAddr::V4(Ipv4Addr::LOCALHOST)).await;
    let (status, _, bytes) = response(&ep, &server, &raw("GET", &f.path(), &auth(&f.token), ""))
        .await
        .unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
    assert_eq!(bytes.len(), f.size);
    assert_eq!(<[u8; 32]>::from(Sha256::digest(&bytes)), f.hash);
    let (status, head, bytes) = response(
        &ep,
        &server,
        &raw("GET", &f.path(), &auth(&f.token), "Range: bytes=7-1026\r\n"),
    )
    .await
    .unwrap();
    assert_eq!(status, 206);
    assert!(head.contains("content-range: bytes 7-1026/"));
    assert_eq!(
        bytes,
        std::fs::read(f.temp.path().join("music/Original.wav")).unwrap()[7..1027]
    );
    let (status, head, bytes) =
        response(&ep, &server, &raw("HEAD", &f.path(), &auth(&f.token), ""))
            .await
            .unwrap();
    assert_eq!(status, 200);
    assert_eq!(bytes, Vec::<u8>::new());
    assert!(head.contains(&format!("content-length: {}", f.size)));
    for authorization in [
        auth("invalid"),
        auth(&f.token).replace("iroh-fixture\", Version", "wrong-device\", Version"),
    ] {
        assert_eq!(
            response(&ep, &server, &raw("GET", &f.path(), &authorization, ""))
                .await
                .unwrap()
                .0,
            401
        );
    }
    let unauth = format!("GET {} HTTP/1.1\r\nHost: bluefin\r\n\r\n", f.path());
    assert_eq!(response(&ep, &server, &unauth).await.unwrap().0, 401);
    assert_eq!(
        response(
            &ep,
            &server,
            &raw("GET", "/System/Info", &auth(&f.token), "")
        )
        .await
        .unwrap()
        .0,
        404
    );
    assert!(
        response(
            &ep,
            &server,
            &raw(
                "GET",
                &f.path(),
                &auth(&f.token),
                "X-Forwarded-For: 127.0.0.1\r\n"
            )
        )
        .await
        .is_err()
    );
    denied_playback_admissions(&f, &ep, &server).await;
    assert!(server.stop().await.joined_connections >= 9);
    let restarted = f.start().await;
    assert_eq!(id, restarted.id);
    restarted.stop().await;
    ep.close().await;
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn ordinary_static_body_rechecks_fresh_playback_boolean_after_admission() {
    use futures_util::StreamExt as _;
    let f = Fixture::new(512 * 1024).await;
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(f.path())
                .header(header::AUTHORIZATION, auth(&f.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream();
    assert!(!body.next().await.unwrap().unwrap().is_empty());
    // Independent owning-DB write deliberately leaves request/DTO caches warm.
    let changed = sqlx::query("UPDATE Permissions SET Value=0 WHERE UserId=? AND Kind=?")
        .bind(ferrofin_db::store::guid_to_db(
            uuid::Uuid::parse_str(&f.viewer).unwrap(),
        ))
        .bind(i32::from(
            ferrofin_db::enums::PermissionKind::EnableMediaPlayback,
        ))
        .execute(f.db.pool())
        .await
        .unwrap();
    assert_eq!(
        changed.rows_affected(),
        1,
        "the independent writer changed the actual owning permission row"
    );
    assert!(
        body.next().await.unwrap().is_err(),
        "retained HTTP body must refuse its next frame"
    );
    drop(body);
    // This boolean is a playback admission, not a login or catalog policy.
    assert_eq!(
        call(&f.app, "GET", "/Users/Me", &f.token, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &f.app,
            "GET",
            "/Items?recursive=true",
            &f.token,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    f.close().await;
}

fn nonloopback() -> IpAddr {
    let udp = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
    udp.connect("192.0.2.1:9").unwrap();
    let ip = udp.local_addr().unwrap().ip();
    assert!(
        !ip.is_loopback() && !ip.is_unspecified(),
        "fixture requires an existing nonloopback interface"
    );
    ip
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn actual_nonloopback_path_refuses_remote_ip_and_spoofing() {
    let f = Fixture::new(512 * 1024).await;
    let server = f.start().await;
    let ep = endpoint(nonloopback()).await;
    let request = raw("GET", &f.path(), &auth(&f.token), "");
    assert_eq!(response(&ep, &server, &request).await.unwrap().0, 200);
    let peer = *server.seen.lock().unwrap().last().unwrap();
    assert_eq!(peer.remote, ep.bound_sockets()[0]);
    assert_eq!(peer.local, server.address);
    assert!(!peer.remote.ip().is_loopback());
    let network = f.wired.state.network.as_ref().unwrap();
    let saved = network.read().unwrap().configuration().clone();
    let mut refused = saved.clone();
    refused.remote_ip_filter = vec![peer.remote.ip().to_string()];
    refused.is_remote_ip_filter_blacklist = true;
    refused.local_network_subnets = vec!["127.0.0.0/8".into(), format!("!{}/32", peer.remote.ip())];
    network.write().unwrap().update_settings(&refused);
    assert_ne!(
        f.wired.state.remote_access_policy(peer.remote.ip()),
        ferrofin_networking::RemoteAccessPolicyResult::Allow,
        "fixture explicitly classifies its actual socket IP as remote"
    );
    assert_eq!(response(&ep, &server, &request).await.unwrap().0, 503);
    assert!(
        response(
            &ep,
            &server,
            &raw(
                "GET",
                &f.path(),
                &auth(&f.token),
                "X-Forwarded-For: 127.0.0.1\r\n"
            )
        )
        .await
        .is_err()
    );
    // Ordinary production IP gate parity with the real observed socket context.
    let mut req = Request::builder()
        .uri(f.path())
        .header(header::AUTHORIZATION, auth(&f.token))
        .body(Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(axum::extract::ConnectInfo(peer.remote));
    req.extensions_mut()
        .insert(ferrofin_api::ip_access::ConnectionLocalAddr(peer.local));
    assert_eq!(
        f.app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    network.write().unwrap().update_settings(&saved);
    server.stop().await;
    ep.close().await;
    f.close().await;
}

async fn read_head(recv: &mut iroh::endpoint::RecvStream) -> String {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        assert!(head.len() < 16 * 1024);
        head.push(recv.read_u8().await.unwrap());
    }
    String::from_utf8(head).unwrap()
}
fn positions(path: &Path) -> Vec<u64> {
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .flatten()
        .filter_map(|fd| {
            if std::fs::read_link(fd.path()).ok().as_deref() != Some(path) {
                return None;
            }
            let info = std::fs::read_to_string(Path::new("/proc/self/fdinfo").join(fd.file_name()))
                .ok()?;
            info.lines()
                .find_map(|line| line.strip_prefix("pos:\t").and_then(|v| v.parse().ok()))
        })
        .collect()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn large_slow_reader_retains_capacity_drains_and_cancels_with_joined_shutdown() {
    let f = Fixture::new(64 * 1024 * 1024).await;
    let mut server = f.start().await;
    let address = server.address;
    let ep = endpoint(IpAddr::V4(Ipv4Addr::LOCALHOST)).await;
    let (conn, mut recv) = open(&ep, &server, &raw("GET", &f.path(), &auth(&f.token), ""))
        .await
        .unwrap();
    assert!(read_head(&mut recv).await.starts_with("HTTP/1.1 200 "));
    tokio::time::sleep(Duration::from_millis(300)).await;
    let offsets = positions(&f.temp.path().join("music/Original.wav"));
    assert!(
        offsets.iter().any(|p| *p > 0),
        "actual production file read"
    );
    assert!(
        offsets.iter().all(|p| *p < (f.size as u64)),
        "backpressure prevented reading all 64 MiB: {offsets:?}"
    );
    eprintln!("64MiB stalled production-file offsets: {offsets:?}");
    let mut hash = Sha256::new();
    let mut count = 0usize;
    let mut buffer = vec![0u8; 32768];
    while let Some(n) = recv.read(&mut buffer).await.unwrap() {
        hash.update(&buffer[..n]);
        count += n;
        if count % (1024 * 1024) < 32768 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }
    assert_eq!(count, f.size);
    let digest = hash.finalize();
    let mut hex = String::with_capacity(64);
    for byte in &digest {
        write!(&mut hex, "{byte:02x}").unwrap();
    }
    eprintln!("synthetic media bytes={count}, SHA256={hex}");
    assert_eq!(<[u8; 32]>::from(digest), f.hash);
    assert!(
        conn.close_reason().is_none(),
        "FIN/ACK did not close before application drain"
    );
    assert_eq!(
        *server.active.borrow(),
        1,
        "connection retained until application closes"
    );
    conn.close(0u32.into(), b"application drained");
    tokio::time::timeout(Duration::from_secs(5), server.active.wait_for(|n| *n == 0))
        .await
        .unwrap()
        .unwrap();
    // Four stalled requests consume all four real connection permits.
    let mut held = Vec::new();
    for _ in 0..4 {
        let (c, mut r) = open(&ep, &server, &raw("GET", &f.path(), &auth(&f.token), ""))
            .await
            .unwrap();
        read_head(&mut r).await;
        held.push((c, r));
    }
    assert_eq!(*server.active.borrow(), 4);
    let fifth = tokio::time::timeout(
        Duration::from_secs(5),
        ep.connect(
            EndpointAddr::new(server.id).with_ip_addr(server.address),
            ALPN,
        ),
    )
    .await
    .unwrap();
    assert!(
        fifth.is_err(),
        "fifth connection refused, no unbounded waiter"
    );
    for (c, _) in &held {
        c.close(42u32.into(), b"cancelled");
    }
    drop(held);
    tokio::time::timeout(Duration::from_secs(5), server.active.wait_for(|n| *n == 0))
        .await
        .unwrap()
        .unwrap();
    let (active, mut pending) = open(&ep, &server, &raw("GET", &f.path(), &auth(&f.token), ""))
        .await
        .unwrap();
    read_head(&mut pending).await;
    let receipt = server.stop().await;
    assert!(receipt.joined_connections >= 6);
    let closed = tokio::time::timeout(Duration::from_secs(5), active.closed()).await;
    assert!(
        closed.is_ok(),
        "actual closure, timeout is not closure proof"
    );
    drop(pending);
    drop(active);
    let socket =
        std::net::UdpSocket::bind(address).expect("joined endpoint released actual UDP socket");
    drop(socket);
    ep.close().await;
    f.close().await;
}

async fn denied_playback_admissions(f: &Fixture, ep: &Endpoint, server: &Running) {
    f.policy(json!({"EnableMediaPlayback":false})).await;
    assert_eq!(
        response(ep, server, &raw("GET", &f.path(), &auth(&f.token), ""))
            .await
            .unwrap()
            .0,
        403
    );
    let ordinary = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(f.path())
                .header(header::AUTHORIZATION, auth(&f.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    eprintln!(
        "EnableMediaPlayback=false: real Iroh=403, ordinary HTTP={}",
        ordinary.status()
    );
    assert_eq!(ordinary.status(), StatusCode::FORBIDDEN);
    for method in ["HEAD", "GET"] {
        let ordinary = f
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(f.path())
                    .header(header::AUTHORIZATION, auth(&f.token))
                    .header(header::RANGE, "bytes=0-1023")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ordinary.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response(
                ep,
                server,
                &raw(
                    method,
                    &f.path(),
                    &auth(&f.token),
                    "Range: bytes=0-1023\r\n"
                )
            )
            .await
            .unwrap()
            .0,
            403
        );
    }
}
