//! Managed pairing through the real router, database, session manager and cache.
//! All users and credentials belong to a fresh, disposable test database.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ferrofin_server::config::Config;
use ferrofin_server::state::{WiredApp, build_app_state};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const PHONE: &str = "pairing-phone";
const WATCH: &str = "pairing-watch";

async fn wire(db: &ferrofin_db::Database, config: &Config) -> WiredApp {
    let ffmpeg = ferrofin_server::bootstrap::FfmpegPaths {
        encoder_app_path_display: None,
        ffmpeg: "ffmpeg".into(),
        ffprobe: "ffprobe".into(),
        capabilities: ferrofin_mediaencoding::FfmpegCapabilities::default(),
        chromaprint_muxer: false,
    };
    let (shutdown, _receiver) = tokio::sync::oneshot::channel();
    build_app_state(db, config, &ffmpeg, None, shutdown)
        .await
        .expect("wire real managers")
}

async fn retire(wired: WiredApp) {
    wired.task_manager.shutdown().await;
    for task in wired.background {
        task.abort();
        let _ = task.await;
    }
}

async fn request(
    router: &axum::Router,
    method: &str,
    path: &str,
    device: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let identity = format!(
        r#"MediaBrowser Client="Bluefin test", Device="Synthetic", DeviceId="{device}", Version="1""#
    );
    let authorization = token.map_or_else(
        || identity.clone(),
        |token| format!(r#"{identity}, Token="{token}""#),
    );
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::AUTHORIZATION, authorization);
    let body = if let Some(body) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        Body::from(serde_json::to_vec(&body).expect("JSON body"))
    } else {
        Body::empty()
    };
    let response = router
        .clone()
        .oneshot(builder.body(body).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    if path.starts_with("/Bluefin/Pairing/") {
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    let bytes = axum::body::to_bytes(response.into_body(), 2_000_000)
        .await
        .expect("bounded response");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn login(router: &axum::Router, name: &str, password: &str, device: &str) -> String {
    let (status, body) = request(
        router,
        "POST",
        "/Users/AuthenticateByName",
        device,
        None,
        Some(json!({"Username": name, "Pw": password})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body["AccessToken"].as_str().expect("token").to_owned()
}

async fn me(router: &axum::Router, device: &str, token: &str) -> StatusCode {
    request(router, "GET", "/Users/Me", device, Some(token), None)
        .await
        .0
}

struct TestServer {
    _temp: tempfile::TempDir,
    db: ferrofin_db::Database,
    config: Config,
    wired: WiredApp,
    router: axum::Router,
    phone: String,
    viewer: Value,
}

async fn boot() -> TestServer {
    let temp = tempfile::tempdir().expect("test root");
    for name in ["config", "data", "cache"] {
        std::fs::create_dir_all(temp.path().join(name)).expect("test directory");
    }
    let config = Config {
        admin_user: "admin".into(),
        admin_password: "synthetic-admin-password".into(),
        ..Config::test_stub(temp.path())
    };
    let db = ferrofin_db::Database::connect(&config.database_url())
        .await
        .expect("test database");
    db.run_migrations().await.expect("migrations");
    let wired = wire(&db, &config).await;
    ferrofin_server::seed::seed_default_admin(wired.state.users.as_ref(), &config)
        .await
        .expect("seed test administrator");
    let mut settings = (*wired.state.config.configuration().await.expect("settings")).clone();
    settings.quick_connect_available = true;
    wired
        .state
        .config
        .update_configuration(&settings)
        .await
        .expect("enable test pairing");
    let router = ferrofin_api::create_router(wired.state.clone());
    let admin = login(&router, "admin", "synthetic-admin-password", "test-admin").await;
    let (status, viewer) = request(
        &router,
        "POST",
        "/Users/New",
        "test-admin",
        Some(&admin),
        Some(json!({"Name":"viewer", "Password":"synthetic-viewer-password"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let phone = login(&router, "viewer", "synthetic-viewer-password", PHONE).await;
    TestServer {
        _temp: temp,
        db,
        config,
        wired,
        router,
        phone,
        viewer,
    }
}

async fn authorize(router: &axum::Router, phone: &str, viewer: &Value) -> Value {
    let (status, capability) = request(
        router,
        "GET",
        "/Bluefin/Pairing/Capabilities",
        WATCH,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(capability["managed_pairing_v1"], true);
    let (status, start) = request(
        router,
        "POST",
        "/Bluefin/Pairing/Initiate",
        WATCH,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(start["device_id"], WATCH);
    let proof = start["proof"].clone();
    let authorization = json!({"proof":proof, "device_id":WATCH});
    assert_eq!(
        request(
            router,
            "POST",
            "/Bluefin/Pairing/Authorize",
            PHONE,
            None,
            Some(authorization.clone())
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let wrong_device = json!({"proof":proof, "device_id":"another-watch"});
    assert_eq!(
        request(
            router,
            "POST",
            "/Bluefin/Pairing/Authorize",
            PHONE,
            Some(phone),
            Some(wrong_device)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut override_user = authorization.clone();
    override_user["user_id"] = viewer["Id"].clone();
    assert!(
        request(
            router,
            "POST",
            "/Bluefin/Pairing/Authorize",
            PHONE,
            Some(phone),
            Some(override_user)
        )
        .await
        .0
        .is_client_error()
    );
    let (status, authorized) = request(
        router,
        "POST",
        "/Bluefin/Pairing/Authorize",
        PHONE,
        Some(phone),
        Some(authorization),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(authorized["state"], "authorized");
    proof
}

async fn exchange(router: &axum::Router, phone: &str, viewer: &Value, proof: &Value) -> String {
    // Simulate a lost first exchange response: retry must return the same owner.
    let (status, first) = request(
        router,
        "POST",
        "/Bluefin/Pairing/Exchange",
        WATCH,
        None,
        Some(proof.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, repeated) = request(
        router,
        "POST",
        "/Bluefin/Pairing/Exchange",
        WATCH,
        None,
        Some(proof.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["AccessToken"], repeated["AccessToken"]);
    assert_eq!(first["User"]["Id"], viewer["Id"]);
    let watch = first["AccessToken"].as_str().expect("Watch token");
    assert_ne!(watch, phone);
    watch.to_owned()
}

async fn cancel(router: &axum::Router, phone: &str, watch: &str, proof: &Value) {
    // Prime the actual shared authorization cache before cancelling.
    assert_eq!(me(router, WATCH, watch).await, StatusCode::OK);
    let (status, cancelled) = request(
        router,
        "POST",
        "/Bluefin/Pairing/Cancel",
        WATCH,
        None,
        Some(proof.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cancelled["state"], "cancelled");
    assert_eq!(me(router, WATCH, watch).await, StatusCode::UNAUTHORIZED);
    assert_eq!(me(router, PHONE, phone).await, StatusCode::OK);
}

#[tokio::test]
async fn managed_pairing_real_routes_preserve_identity_and_durable_revocation() {
    let TestServer {
        _temp,
        db,
        config,
        wired,
        router,
        phone,
        viewer,
    } = boot().await;
    let proof = authorize(&router, &phone, &viewer).await;
    let watch = exchange(&router, &phone, &viewer, &proof).await;
    cancel(&router, &phone, &watch, &proof).await;
    drop(router);
    retire(wired).await;
    let restarted = wire(&db, &config).await;
    let router = ferrofin_api::create_router(restarted.state.clone());
    let (status, cancelled) = request(
        &router,
        "POST",
        "/Bluefin/Pairing/Cancel",
        WATCH,
        None,
        Some(proof.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cancelled["state"], "cancelled");
    assert!(
        request(
            &router,
            "POST",
            "/Bluefin/Pairing/Exchange",
            WATCH,
            None,
            Some(proof)
        )
        .await
        .0
        .is_client_error()
    );
    assert_eq!(me(&router, WATCH, &watch).await, StatusCode::UNAUTHORIZED);
    assert_eq!(me(&router, PHONE, &phone).await, StatusCode::OK);
    drop(router);
    retire(restarted).await;
}
