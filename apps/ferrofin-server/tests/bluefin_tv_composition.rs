//! Real production composition over disposable libraries, media and accounts.
//! These are synthetic router/HTTP checks, never installed-server acceptance.
use std::path::Path;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use ferrofin_db::Database;
use ferrofin_server::{
    config::Config,
    state::{WiredApp, build_app_state},
};
use ferrofin_traits::library::ScanTrigger;
use serde_json::{Value, json};
use tower::ServiceExt as _;

struct Fixture {
    app: Router,
    wired: WiredApp,
    db: Database,
    token: String,
    viewer: String,
    item: String,
    video: String,
    database_url: String,
    temp: tempfile::TempDir,
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(request(method, uri, token, body))
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn request(method: &str, uri: &str, token: &str, body: Value) -> Request<Body> {
    Request::builder().method(method).uri(uri)
        .header(header::AUTHORIZATION, format!(r#"MediaBrowser Client="fixture", Device="fixture", DeviceId="composition-fixture", Version="1", Token="{token}""#))
        .header(header::CONTENT_TYPE, "application/json")
        .body(if body.is_null() { Body::empty() } else { axum::response::IntoResponse::into_response(axum::Json(body)).into_body() }).unwrap()
}

fn generate(path: &Path) {
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "18",
            "-c:a",
            "pcm_s16le",
            "-y",
        ])
        .arg(path)
        .status()
        .expect("real ffmpeg");
    assert!(status.success());
}

async fn fixture() -> Fixture {
    fixture_with_encoder(false).await
}

fn delayed_encoder(root: &Path) -> std::path::PathBuf {
    // A test-only Rust exec wrapper stops before the actual encoder opens an
    // input. It forwards startup probes and normal scan work unchanged.
    let source = root.join("encoder.rs");
    let output = root.join("delayed-encoder");
    let directory = serde_json::to_string(&root.to_string_lossy()).unwrap();
    let code = r#"
use std::{path::Path, process::Command, os::unix::process::CommandExt};
fn main() {
    let root = Path::new(DIRECTORY);
    let mut args: Vec<_> = std::env::args().skip(1).collect();
    let playback_input = args.iter().any(|a| a.contains("/proc/") || a.contains(root.join("music").to_str().unwrap()) || a.contains(root.join("videos").to_str().unwrap()));
    if root.join("realtime-enabled").exists() && playback_input && args.iter().any(|a| a.ends_with(".mp3") || a.ends_with(".m3u8")) {
        if let Some(index) = args.iter().position(|a| a == "-i") { args.insert(index, "-re".into()); }
        std::fs::write(root.join(format!("realtime-{}", std::process::id())), b"encoder").unwrap();
    }
    if root.join("delay-enabled").exists() && args.iter().any(|a| a.ends_with(".mp3") || a.ends_with(".m3u8")) {
        std::fs::write(root.join(format!("pending-{}", std::process::id())), b"pending").unwrap();
        let stopping = root.join(format!("stopping-{}", std::process::id()));
        std::thread::spawn(move || {
            use std::io::Read;
            let mut quit = [0; 2];
            if std::io::stdin().read_exact(&mut quit).is_ok() { std::fs::write(stopping, b"stopping").unwrap(); }
        });
        while !root.join("open-gate").exists() { std::thread::sleep(std::time::Duration::from_millis(10)); }
    }
    let error = Command::new("ffmpeg").args(args).exec();
    eprintln!("fixture encoder exec failed: {error}");
    std::process::exit(1);
}
"#.replace("DIRECTORY", &directory);
    std::fs::write(&source, code).unwrap();
    assert!(
        std::process::Command::new("rustc")
            .args(["--edition=2024", "-C", "debuginfo=0"])
            .arg(&source)
            .arg("-o")
            .arg(&output)
            .status()
            .unwrap()
            .success()
    );
    output
}

async fn fixture_with_encoder(delayed: bool) -> Fixture {
    fixture_with_limits(delayed, 2, 2).await
}

fn create_media(root: &Path) -> std::path::PathBuf {
    for dir in ["config", "data", "cache", "music", "videos"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    let media = root.join("music/Original.wav");
    generate(&media);
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:size=160x90:rate=12",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "42",
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
        .arg(root.join("videos/Original.mp4"))
        .status()
        .unwrap();
    assert!(status.success());
    media
}

async fn create_and_scan_libraries(app: &Router, wired: &WiredApp, root: &Path, admin: &str) {
    // Ordinary supported library creation persists PathInfos and scan ancestry.
    for (name, kind, dir) in [
        ("Music", "music", "music"),
        ("Videos", "homevideos", "videos"),
    ] {
        let uri = format!(
            "/Library/VirtualFolders?name={name}&collectionType={kind}&paths={}&refreshLibrary=false",
            root.join(dir).display()
        );
        let (status, _) = call(app, "POST", &uri, admin, Value::Null).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    assert!(
        wired
            .state
            .library
            .run_library_scan(ScanTrigger::Api, None)
            .await
            .unwrap()
    );
}

async fn sign_in(app: &Router, name: &str, password: &str) -> (String, String) {
    let (status, login) = call(
        app,
        "POST",
        "/Users/AuthenticateByName",
        "",
        json!({"Username":name,"Pw":password}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "fixture login");
    (
        login["AccessToken"].as_str().unwrap().to_owned(),
        login["User"]["Id"].as_str().unwrap().to_owned(),
    )
}

async fn scanned_item(app: &Router, token: &str, kind: &str, path: &Path) -> String {
    let uri = format!("/Items?recursive=true&includeItemTypes={kind}&fields=Path");
    let (status, items) = call(app, "GET", &uri, token, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "normal scanned catalog");
    items["Items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["Path"].as_str() == path.to_str())
        .expect("normally scanned media")["Id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn fixture_with_limits(delayed: bool, server_limit: u32, user_limit: u32) -> Fixture {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn,ferrofin_api::error=debug,ferrofin_mediaencoding=debug")
        .with_test_writer()
        .try_init();
    let temp = tempfile::tempdir().unwrap();
    let media = create_media(temp.path());
    let encoder = if delayed {
        delayed_encoder(temp.path())
    } else {
        std::env::var_os("BLUEFIN_TEST_FFMPEG").map_or_else(
            || std::path::PathBuf::from("ffmpeg"),
            std::path::PathBuf::from,
        )
    };
    let probe = std::env::var_os("BLUEFIN_TEST_FFPROBE").map_or_else(
        || std::path::PathBuf::from("ffprobe"),
        std::path::PathBuf::from,
    );
    let config = Config {
        admin_user: "fixture".into(),
        admin_password: "fixture-password".into(),
        ffmpeg_path: Some(encoder),
        ffprobe_path: Some(probe),
        max_transcodes: server_limit,
        max_transcodes_per_user: user_limit,
        ..Config::test_stub(temp.path())
    };
    let database_url = config.database_url();
    let db = Database::connect(&database_url).await.unwrap();
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
    let (admin, _) = sign_in(&app, "fixture", "fixture-password").await;
    create_and_scan_libraries(&app, &wired, temp.path(), &admin).await;
    let (status, created) = call(
        &app,
        "POST",
        "/Users/New",
        &admin,
        json!({"Name":"viewer", "Password":"viewer-password"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let (token, viewer) = sign_in(&app, "viewer", "viewer-password").await;
    let item = scanned_item(&app, &token, "Audio", &media).await;
    let video = scanned_item(
        &app,
        &token,
        "Video",
        &temp.path().join("videos/Original.mp4"),
    )
    .await;
    Fixture {
        app,
        wired,
        db,
        token,
        viewer,
        item,
        video,
        database_url,
        temp,
    }
}

impl Fixture {
    async fn close(self) {
        drop(self.app);
        for task in &self.wired.background {
            task.abort();
        }
        self.wired.task_manager.shutdown().await;
        drop(self.wired);
        for entry in std::fs::read_dir(self.temp.path().join("data/log"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
            assert!(
                !text.contains("/proc/"),
                "private descriptor escaped an encoder log"
            );
            assert!(
                !text.contains(&self.token),
                "account token escaped an encoder log"
            );
        }
        self.db.close().await;
    }

    async fn lease(&self, item: &str, kind: &str) -> Value {
        let began = std::time::Instant::now();
        let (status, lease) = call(
            &self.app,
            "POST",
            "/Bluefin/TV/v1/leases",
            &self.token,
            json!({"version":1,"itemId":item,"playback":{"kind":kind,"maxBitrate":8_000_000}}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "lease {kind}: {lease}");
        eprintln!(
            "synthetic TV {kind} admission_us={}",
            began.elapsed().as_micros()
        );
        assert!(!lease.to_string().contains(&self.token));
        assert!(!lease.to_string().contains("/proc/"));
        lease
    }

    async fn scoped(&self, item: &str) -> ferrofin_api::AppState {
        let id = uuid::Uuid::parse_str(item).unwrap();
        let source = self
            .wired
            .state
            .media_sources
            .get_stream_media_source(id, None, None)
            .await
            .unwrap();
        self.wired
            .state
            .tv_playback_pins
            .as_ref()
            .unwrap()
            .pin(
                &self.wired.state,
                id,
                source.id.as_deref().unwrap(),
                source.path.as_deref().unwrap(),
                uuid::Uuid::parse_str(&self.viewer).unwrap(),
            )
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn normal_library_pin_and_registered_production_tv_route() {
    let f = fixture().await;
    let item = uuid::Uuid::parse_str(&f.item).unwrap();
    let viewer = uuid::Uuid::parse_str(&f.viewer).unwrap();
    let source = f
        .wired
        .state
        .media_sources
        .get_stream_media_source(item, None, None)
        .await
        .unwrap();
    let pin = f
        .wired
        .state
        .media_sources
        .pin_local_source(
            item,
            source.id.as_deref().unwrap(),
            source.path.as_deref().unwrap(),
            viewer,
        )
        .await
        .expect("normal PathInfos/physical ancestry must admit");
    assert!(pin.get_stream_media_source(item, None, None).await.is_ok());
    let (status, info) = call(&f.app, "GET", "/Bluefin/TV/v1", &f.token, Value::Null).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "TV composition not registered: {info}"
    );
    assert_eq!(info["mediaLease"], true);
    drop(pin);
    f.close().await;
}

#[tokio::test]
async fn production_scoped_direct_head_and_ranges_keep_opened_bytes_after_replacement() {
    let f = fixture().await;
    let scoped = f.scoped(&f.video).await;
    assert!(std::sync::Arc::ptr_eq(&scoped.users, &f.wired.state.users));
    assert!(std::sync::Arc::ptr_eq(
        &scoped.sessions,
        &f.wired.state.sessions
    ));
    assert!(std::sync::Arc::ptr_eq(
        &scoped.config,
        &f.wired.state.config
    ));
    let app = ferrofin_api::create_router(scoped);
    let original = f.temp.path().join("videos/Original.mp4");
    let bytes = std::fs::read(&original).unwrap();
    std::fs::rename(&original, f.temp.path().join("videos/Retired.mp4")).unwrap();
    std::fs::write(&original, b"NEVER-SERVE-THIS-REPLACEMENT").unwrap();
    let other = Database::connect(&f.database_url).await.unwrap();
    sqlx::query(r#"UPDATE "BaseItems" SET "Path"='https://invalid.example/replacement', "PrimaryVersionId"=NULL WHERE "Id"=?"#)
        .bind(ferrofin_db::store::guid_to_db(uuid::Uuid::parse_str(&f.video).unwrap())).execute(other.writer()).await.unwrap();
    sqlx::query(r#"UPDATE "MediaStreamInfos" SET "Codec"='changed' WHERE "ItemId"=?"#)
        .bind(ferrofin_db::store::guid_to_db(
            uuid::Uuid::parse_str(&f.video).unwrap(),
        ))
        .execute(other.writer())
        .await
        .unwrap();
    let uri = format!("/Videos/{}/stream.mp4?static=true", f.video);
    for (method, range, expected_status, expected) in [
        ("GET", None, StatusCode::OK, bytes.as_slice()),
        ("HEAD", None, StatusCode::OK, &[][..]),
        (
            "GET",
            Some("bytes=7-41"),
            StatusCode::PARTIAL_CONTENT,
            &bytes[7..42],
        ),
        (
            "GET",
            Some("bytes=-29"),
            StatusCode::PARTIAL_CONTENT,
            &bytes[bytes.len() - 29..],
        ),
        (
            "HEAD",
            Some("bytes=7-41"),
            StatusCode::PARTIAL_CONTENT,
            &[][..],
        ),
        (
            "GET",
            Some("bytes=999999999-"),
            StatusCode::RANGE_NOT_SATISFIABLE,
            &[][..],
        ),
    ] {
        let mut req = request(method, &uri, &f.token, Value::Null);
        if let Some(range) = range {
            req.headers_mut()
                .insert(header::RANGE, range.parse().unwrap());
        }
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), expected_status);
        assert!(!format!("{:?}", response.headers()).contains("/proc/"));
        assert_eq!(
            to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap()
                .as_ref(),
            expected
        );
    }
    drop(app);
    other.close().await;
    f.close().await;
}

#[tokio::test]
async fn authenticated_audio_lease_runs_actual_progressive_encoder_and_releases() {
    let f = fixture().await;
    let (status, info) = call(
        &f.app,
        "POST",
        &format!("/Items/{}/PlaybackInfo", f.item),
        &f.token,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(info["PlaySessionId"].is_string());
    let lease = f.lease(&f.item, "audio").await;
    assert_ne!(info["PlaySessionId"], lease["playSessionId"]);
    let response = f
        .app
        .clone()
        .oneshot(request(
            "GET",
            lease["mediaUrl"].as_str().unwrap(),
            &f.token,
            Value::Null,
        ))
        .await
        .unwrap();
    if !response.status().is_success() {
        for entry in std::fs::read_dir(f.temp.path().join("data/log"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
            assert!(
                !text.contains("/proc/"),
                "private input escaped encoder log"
            );
            eprintln!("encoder fixture log: {text}");
        }
    }
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "no-store, private"
    );
    let audio = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    assert!(audio.len() > 4096);
    let output = f.temp.path().join("encoded.mp3");
    std::fs::write(&output, &audio).unwrap();
    let decoded = std::process::Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(&output)
        .args(["-f", "md5", "-"])
        .output()
        .unwrap();
    assert!(decoded.status.success(), "actual encoded MP3 decodes");
    let (status, _) = call(
        &f.app,
        "DELETE",
        lease["releaseUrl"].as_str().unwrap(),
        &f.token,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        &f.app,
        "GET",
        lease["mediaUrl"].as_str().unwrap(),
        &f.token,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    f.close().await;
}

fn manifest_links(bytes: &[u8]) -> Vec<String> {
    let text = std::str::from_utf8(bytes).unwrap();
    assert!(!text.contains("/proc/"));
    assert!(!text.to_ascii_lowercase().contains("apikey"));
    assert!(!text.to_ascii_lowercase().contains("authorization"));
    text.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

fn mutate_generations(
    writer: Database,
    video: String,
    alternate: String,
    replacement: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        for round in 0..8 {
            assert_eq!(
                sqlx::query(
                    r#"UPDATE "BaseItems" SET "Path"=?, "PrimaryVersionId"=? WHERE "Id"=?"#
                )
                .bind(if round % 2 == 0 {
                    replacement.as_str()
                } else {
                    "https://invalid.example/replacement"
                })
                .bind(if round % 2 == 0 {
                    Some(alternate.as_str())
                } else {
                    None
                })
                .bind(&video)
                .execute(writer.writer())
                .await
                .unwrap()
                .rows_affected(),
                1
            );
            assert!(
                sqlx::query(
                    r#"UPDATE "MediaStreamInfos" SET "Codec"=? WHERE "ItemId"=? AND "StreamType"=1"#
                )
                .bind(if round % 2 == 0 { "hevc" } else { "h264" })
                .bind(&video)
                .execute(writer.writer())
                .await
                .unwrap()
                .rows_affected()
                    > 0
            );
            tokio::task::yield_now().await;
        }
    })
}

#[tokio::test]
async fn production_video_hls_master_segments_seek_and_replacement_decode_original() {
    let f = fixture().await;
    let lease = f.lease(&f.video, "video").await;
    let path = f.temp.path().join("videos/Original.mp4");
    std::fs::rename(&path, f.temp.path().join("videos/Retired.mp4")).unwrap();
    std::fs::write(&path, b"NOT-A-VIDEO").unwrap();
    let other = Database::connect(&f.database_url).await.unwrap();
    let writer = other.clone();
    let video = ferrofin_db::store::guid_to_db(uuid::Uuid::parse_str(&f.video).unwrap());
    let alternate = ferrofin_db::store::guid_to_db(uuid::Uuid::parse_str(&f.item).unwrap());
    let replacement = path.to_string_lossy().into_owned();
    let writing = mutate_generations(writer, video, alternate, replacement);
    let response = f
        .app
        .clone()
        .oneshot(request(
            "GET",
            lease["mediaUrl"].as_str().unwrap(),
            &f.token,
            Value::Null,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let master = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    assert!(!String::from_utf8_lossy(&master).contains(&f.token));
    let variants = manifest_links(&master);
    let variant = variants.first().expect("master has an HLS variant");
    let response = f
        .app
        .clone()
        .oneshot(request("GET", variant, &f.token, Value::Null))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let segments = manifest_links(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap());
    assert!(segments.len() >= 6);
    for index in [0, 5, 1] {
        let response = f
            .app
            .clone()
            .oneshot(request("GET", &segments[index], &f.token, Value::Null))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "segment {index}");
        let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        let segment = f.temp.path().join(format!("check-{index}.ts"));
        std::fs::write(&segment, bytes).unwrap();
        let decoded = std::process::Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(&segment)
            .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .output()
            .unwrap();
        assert!(decoded.status.success(), "real segment {index} decodes");
        assert_eq!(decoded.stdout.len(), 160 * 90 * 3, "one actual RGB frame");
        assert!(
            decoded
                .stdout
                .as_chunks::<3>()
                .0
                .iter()
                .all(|p| p[0] > 180 && p[1] < 50 && p[2] < 50),
            "opened original red video"
        );
    }
    let (status, _) = call(
        &f.app,
        "DELETE",
        lease["releaseUrl"].as_str().unwrap(),
        &f.token,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    writing.await.unwrap();
    other.close().await;
    f.close().await;
}

#[rstest::rstest]
#[case::playback_permission(0)]
#[case::disabled_user(1)]
#[case::token_revoked_by_independent_writer(2)]
#[case::schedule(3)]
#[case::blocked_tag(4)]
#[case::blocked_library_folder(5)]
#[case::parental_rating(6)]
#[case::library_removed(7)]
#[case::device_permission(8)]
#[tokio::test]
async fn current_revocation_denies_get_and_head_despite_warm_caches(#[case] mode: u8) {
    use ferrofin_db::{
        enums::{PermissionKind, PreferenceKind},
        store::guid_to_db,
    };
    let f = fixture().await;
    let lease = f.lease(&f.item, "audio").await;
    let other = Database::connect(&f.database_url).await.unwrap();
    let user = guid_to_db(uuid::Uuid::parse_str(&f.viewer).unwrap());
    let item = guid_to_db(uuid::Uuid::parse_str(&f.item).unwrap());
    match mode {
        0 | 1 => {
            let kind = if mode == 0 {
                PermissionKind::EnableMediaPlayback
            } else {
                PermissionKind::IsDisabled
            };
            let updated =
                sqlx::query(r#"UPDATE "Permissions" SET "Value"=? WHERE "UserId"=? AND "Kind"=?"#)
                    .bind(i64::from(mode == 1))
                    .bind(&user)
                    .bind(kind as i32)
                    .execute(other.writer())
                    .await
                    .unwrap();
            assert_eq!(updated.rows_affected(), 1);
        }
        2 => {
            assert!(
                sqlx::query(r#"DELETE FROM "Devices" WHERE "AccessToken"=?"#)
                    .bind(&f.token)
                    .execute(other.writer())
                    .await
                    .unwrap()
                    .rows_affected()
                    > 0
            );
        }
        3 => {
            sqlx::query(r#"INSERT INTO "AccessSchedules" ("UserId","DayOfWeek","StartHour","EndHour") VALUES (?,0,25,26)"#)
                .bind(&user).execute(other.writer()).await.unwrap();
        }
        4 => {
            ferrofin_core::user_entity_ext::set_preference(
                &other,
                &user,
                PreferenceKind::BlockedTags,
                &["restricted".into()],
            )
            .await
            .unwrap();
            assert_eq!(
                sqlx::query(r#"UPDATE "BaseItems" SET "Tags"='restricted' WHERE "Id"=?"#)
                    .bind(&item)
                    .execute(other.writer())
                    .await
                    .unwrap()
                    .rows_affected(),
                1
            );
        }
        5 => {
            // The supported BlockedMediaFolders policy names collection-folder
            // ids, as the ordinary library UI does, not physical TopParentIds.
            let folder: String = sqlx::query_scalar(
                r#"SELECT "Id" FROM "BaseItems" WHERE "Type"=? AND "Name"='Music'"#,
            )
            .bind(
                ferrofin_model::data::BaseItemKind::CollectionFolder
                    .stored_type_name()
                    .unwrap(),
            )
            .fetch_one(other.pool())
            .await
            .unwrap();
            ferrofin_core::user_entity_ext::set_preference(
                &other,
                &user,
                PreferenceKind::BlockedMediaFolders,
                &[folder],
            )
            .await
            .unwrap();
        }
        6 => {
            assert_eq!(
                sqlx::query(r#"UPDATE "Users" SET "MaxParentalRatingScore"=1 WHERE "Id"=?"#)
                    .bind(&user)
                    .execute(other.writer())
                    .await
                    .unwrap()
                    .rows_affected(),
                1
            );
            assert_eq!(
                sqlx::query(r#"UPDATE "BaseItems" SET "OfficialRating"='R' WHERE "Id"=?"#)
                    .bind(&item)
                    .execute(other.writer())
                    .await
                    .unwrap()
                    .rows_affected(),
                1
            );
        }
        7 => {
            let kind = ferrofin_model::data::BaseItemKind::CollectionFolder
                .stored_type_name()
                .unwrap();
            assert_eq!(
                sqlx::query(r#"DELETE FROM "BaseItems" WHERE "Type"=? AND "Name"='Music'"#)
                    .bind(kind)
                    .execute(other.writer())
                    .await
                    .unwrap()
                    .rows_affected(),
                1
            );
        }
        8 => {
            assert_eq!(
                sqlx::query(r#"UPDATE "Permissions" SET "Value"=0 WHERE "UserId"=? AND "Kind"=?"#)
                    .bind(&user)
                    .bind(PermissionKind::EnableAllDevices as i32)
                    .execute(other.writer())
                    .await
                    .unwrap()
                    .rows_affected(),
                1
            );
        }
        _ => unreachable!(),
    }
    for method in ["GET", "HEAD"] {
        let response = f
            .app
            .clone()
            .oneshot(request(
                method,
                lease["mediaUrl"].as_str().unwrap(),
                &f.token,
                Value::Null,
            ))
            .await
            .unwrap();
        assert!(
            !response.status().is_success(),
            "mode {mode} method {method}"
        );
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("/proc/"));
        assert!(!String::from_utf8_lossy(&bytes).contains(&f.token));
    }
    other.close().await;
    f.close().await;
}

#[tokio::test]
async fn unsupported_or_transcode_denied_inputs_never_admit() {
    use ferrofin_db::{enums::PermissionKind, store::guid_to_db};
    let f = fixture().await;
    let other = Database::connect(&f.database_url).await.unwrap();
    let user = guid_to_db(uuid::Uuid::parse_str(&f.viewer).unwrap());
    for kind in [
        PermissionKind::EnableAudioPlaybackTranscoding,
        PermissionKind::EnablePlaybackRemuxing,
    ] {
        assert_eq!(
            sqlx::query(r#"UPDATE "Permissions" SET "Value"=0 WHERE "UserId"=? AND "Kind"=?"#)
                .bind(&user)
                .bind(kind as i32)
                .execute(other.writer())
                .await
                .unwrap()
                .rows_affected(),
            1
        );
    }
    let body =
        json!({"version":1,"itemId":f.item,"playback":{"kind":"audio","maxBitrate":8_000_000}});
    let (status, _) = call(
        &f.app,
        "POST",
        "/Bluefin/TV/v1/leases",
        &f.token,
        body.clone(),
    )
    .await;
    assert!(
        !status.is_success(),
        "transcoding permission remains authoritative"
    );
    for kind in [
        PermissionKind::EnableAudioPlaybackTranscoding,
        PermissionKind::EnablePlaybackRemuxing,
    ] {
        sqlx::query(r#"UPDATE "Permissions" SET "Value"=1 WHERE "UserId"=? AND "Kind"=?"#)
            .bind(&user)
            .bind(kind as i32)
            .execute(other.writer())
            .await
            .unwrap();
    }
    assert_eq!(
        sqlx::query(r#"UPDATE "BaseItems" SET "RunTimeTicks"=NULL WHERE "Id"=?"#)
            .bind(guid_to_db(uuid::Uuid::parse_str(&f.item).unwrap()))
            .execute(other.writer())
            .await
            .unwrap()
            .rows_affected(),
        1
    );
    let (status, _) = call(&f.app, "POST", "/Bluefin/TV/v1/leases", &f.token, body).await;
    assert!(
        !status.is_success(),
        "unknown runtime is refused before negotiation"
    );
    other.close().await;
    f.close().await;
}

#[tokio::test]
async fn synthetic_loopback_http_serves_authenticated_lease_without_account_headers() {
    let f = fixture().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let router = f.app.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .await
        .unwrap();
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let auth = request("GET", "/", &f.token, Value::Null).headers()[header::AUTHORIZATION].clone();
    let origin = format!("http://{address}");
    let response = client.post(format!("{origin}/Bluefin/TV/v1/leases")).header(header::AUTHORIZATION, auth)
        .json(&json!({"version":1,"itemId":f.item,"playback":{"kind":"audio","maxBitrate":8_000_000}})).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let lease: Value = response.json().await.unwrap();
    // This GET deliberately carries no account header. The private capability
    // reauthenticates its captured ordinary session, on the real loopback peer.
    let response = client
        .get(format!("{origin}{}", lease["mediaUrl"].as_str().unwrap()))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!format!("{:?}", response.headers()).contains(&f.token));
    assert!(response.bytes().await.unwrap().len() > 4096);
    let response = client
        .delete(format!("{origin}{}", lease["releaseUrl"].as_str().unwrap()))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    stop.send(()).unwrap();
    server.await.unwrap();
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    f.close().await;
}

async fn pending_encoders(root: &Path, minimum: usize) -> Vec<u32> {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let ids: Vec<_> = std::fs::read_dir(root)
                .unwrap()
                .flatten()
                .filter_map(|entry| {
                    entry
                        .file_name()
                        .to_str()?
                        .strip_prefix("pending-")?
                        .parse::<u32>()
                        .ok()
                })
                .collect();
            if ids.len() >= minimum {
                break ids;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("actual encoder reached its pending input open")
}

fn input_descriptor_count(path: &Path) -> usize {
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .flatten()
        .filter(|entry| std::fs::read_link(entry.path()).is_ok_and(|target| target == path))
        .count()
}

async fn await_retirement(root: &Path, input: &Path, pids: &[u32]) {
    tokio::time::timeout(std::time::Duration::from_secs(12), async {
        loop {
            let live = pids.iter().any(|pid| {
                std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
                    !stat
                        .rsplit_once(") ")
                        .is_some_and(|(_, rest)| rest.starts_with('Z') || rest.starts_with('X'))
                })
            });
            if !live && input_descriptor_count(input) == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "fixture child or opened input survived retirement in {}",
            root.display()
        )
    });
}

#[tokio::test]
async fn disconnect_release_and_router_drop_retire_pending_actual_encoder_without_fd_reuse() {
    for drop_router in [false, true] {
        let mut f = fixture_with_encoder(true).await;
        std::fs::write(f.temp.path().join("delay-enabled"), b"delay").unwrap();
        let lease = f.lease(&f.item, "audio").await;
        let input = f.temp.path().join("music/Original.wav");
        assert_eq!(input_descriptor_count(&input), 1);
        let req = request(
            "GET",
            lease["mediaUrl"].as_str().unwrap(),
            &f.token,
            Value::Null,
        );
        let app = f.app.clone();
        let pending = tokio::spawn(async move { app.oneshot(req).await.unwrap() });
        let pids = pending_encoders(f.temp.path(), 1).await;
        pending.abort();
        let _ = pending.await;
        if drop_router {
            drop(std::mem::replace(&mut f.app, Router::new()));
        } else {
            let (status, _) = call(
                &f.app,
                "DELETE",
                lease["releaseUrl"].as_str().unwrap(),
                &f.token,
                Value::Null,
            )
            .await;
            assert_eq!(status, StatusCode::NO_CONTENT);
        }
        await_retirement(f.temp.path(), &input, &pids).await;
        // Make numeric descriptor reuse likely after retirement. The delayed
        // encoder must already be dead before the gate could admit any open.
        let reused: Vec<_> = (0..64)
            .map(|_| std::fs::File::open(&input).unwrap())
            .collect();
        std::fs::write(f.temp.path().join("open-gate"), b"open").unwrap();
        assert!(
            pids.iter()
                .all(|pid| !std::path::Path::new(&format!("/proc/{pid}")).exists())
        );
        drop(reused);
        f.close().await;
    }
}

#[rstest::rstest]
#[case::global(2, 0)]
#[case::per_user(0, 2)]
#[tokio::test]
async fn ordinary_and_two_scoped_players_share_global_and_per_user_job_limits(
    #[case] server_limit: u32,
    #[case] user_limit: u32,
) {
    let f = fixture_with_limits(true, server_limit, user_limit).await;
    std::fs::write(f.temp.path().join("delay-enabled"), b"delay").unwrap();
    let one = f.lease(&f.item, "audio").await;
    let two = f.lease(&f.item, "audio").await;
    let ordinary_session = uuid::Uuid::new_v4().simple().to_string();
    let uri = format!(
        "/Audio/{}/stream.mp3?audioCodec=mp3&audioBitrate=128000&deviceId=ordinary-fixture&playSessionId={ordinary_session}",
        f.item
    );
    let app = f.app.clone();
    let req = request("GET", &uri, &f.token, Value::Null);
    let ordinary = tokio::spawn(async move { app.oneshot(req).await.unwrap() });
    pending_encoders(f.temp.path(), 1).await;
    let app = f.app.clone();
    let req = request(
        "GET",
        one["mediaUrl"].as_str().unwrap(),
        &f.token,
        Value::Null,
    );
    let scoped = tokio::spawn(async move { app.oneshot(req).await.unwrap() });
    pending_encoders(f.temp.path(), 2).await;
    let response = f
        .app
        .clone()
        .oneshot(request(
            "GET",
            two["mediaUrl"].as_str().unwrap(),
            &f.token,
            Value::Null,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "shared cap must reject a third actual job"
    );
    let app = f.app.clone();
    let req = request(
        "DELETE",
        one["releaseUrl"].as_str().unwrap(),
        &f.token,
        Value::Null,
    );
    let retirement = tokio::spawn(async move { app.oneshot(req).await.unwrap() });
    // The helper records the actual graceful-stop write, then deliberately
    // remains alive until the real runtime's kill fallback. Caps must still
    // count it throughout that interval.
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while !std::fs::read_dir(f.temp.path())
            .unwrap()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("stopping-"))
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        f.wired
            .state
            .hls
            .get_transcoding_job(one["playSessionId"].as_str().unwrap())
            .await
            .unwrap()
            .is_some(),
        "stopping child retains its shared admission slot until join"
    );
    let response = f
        .app
        .clone()
        .oneshot(request(
            "GET",
            two["mediaUrl"].as_str().unwrap(),
            &f.token,
            Value::Null,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "stop in progress cannot lend its slot early"
    );
    assert_eq!(retirement.await.unwrap().status(), StatusCode::NO_CONTENT);
    assert!(
        f.wired
            .state
            .hls
            .get_transcoding_job(&ordinary_session)
            .await
            .unwrap()
            .is_some(),
        "lease release must preserve ordinary work"
    );
    std::fs::write(f.temp.path().join("open-gate"), b"open").unwrap();
    let response = f
        .app
        .clone()
        .oneshot(request(
            "GET",
            two["mediaUrl"].as_str().unwrap(),
            &f.token,
            Value::Null,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "released slot admits the other scoped player"
    );
    assert!(
        !to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap()
            .is_empty()
    );
    let _ = scoped.await;
    let response = ordinary.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    drop(response);
    call(
        &f.app,
        "DELETE",
        two["releaseUrl"].as_str().unwrap(),
        &f.token,
        Value::Null,
    )
    .await;
    f.wired
        .state
        .hls
        .stop_encoding(&ferrofin_traits::media_encoding::HlsStreamRequest {
            device_id: Some("ordinary-fixture".into()),
            play_session_id: Some(ordinary_session),
            ..Default::default()
        })
        .await
        .unwrap();
    f.close().await;
}

fn subtitled_media(root: &Path, path: &Path, label: &str) {
    let subtitle = root.join("embedded.srt");
    let font = root.join("fixture.ttf");
    std::fs::write(
        &subtitle,
        format!("1\n00:00:00,000 --> 00:00:02,000\n{label}\n"),
    )
    .unwrap();
    std::fs::write(&font, label).unwrap();
    assert!(
        std::process::Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(root.join("videos/Original.mp4"))
            .arg("-i")
            .arg(&subtitle)
            .args(["-map", "0", "-map", "1", "-c", "copy", "-attach"])
            .arg(&font)
            .args([
                "-metadata:s:t",
                "mimetype=application/x-truetype-font",
                "-y"
            ])
            .arg(path)
            .status()
            .unwrap()
            .success()
    );
}

#[tokio::test]
async fn scoped_subtitle_and_attachment_resolvers_cannot_reuse_replacement_cache() {
    let f = fixture().await;
    let media = f.temp.path().join("videos/Subtitled.mkv");
    subtitled_media(f.temp.path(), &media, "ORIGINAL-EMBEDDED");
    assert!(
        f.wired
            .state
            .library
            .run_library_scan(ScanTrigger::Api, None)
            .await
            .unwrap()
    );
    let (_, items) = call(
        &f.app,
        "GET",
        "/Items?recursive=true&includeItemTypes=Video&fields=Path",
        &f.token,
        Value::Null,
    )
    .await;
    let item = items["Items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["Path"].as_str() == media.to_str())
        .unwrap()["Id"]
        .as_str()
        .unwrap();
    let id = uuid::Uuid::parse_str(item).unwrap();
    let scoped = f.scoped(item).await;
    let captured = scoped
        .media_sources
        .get_stream_media_source(id, None, None)
        .await
        .unwrap();
    let attachment = captured
        .media_attachments
        .first()
        .expect("real ffprobe attachment");
    let subtitle = captured
        .media_streams
        .iter()
        .find(|stream| stream.stream_type == ferrofin_model::entities::MediaStreamType::Subtitle)
        .expect("real ffprobe embedded subtitle");
    let source_id = captured.id.as_deref().unwrap();
    std::fs::rename(&media, f.temp.path().join("retired-subtitled.mkv")).unwrap();
    subtitled_media(f.temp.path(), &media, "REPLACEMENT-INPUT");
    // An ordinary player now populates the standard source-id cache from the
    // replacement. Scoped resolvers must use a private generation namespace.
    let current = f
        .wired
        .state
        .attachments
        .get_attachment(id, source_id, attachment.index)
        .await
        .unwrap();
    assert_eq!(current.data, b"REPLACEMENT-INPUT");
    let held = scoped
        .attachments
        .get_attachment(id, source_id, attachment.index)
        .await
        .unwrap();
    assert_eq!(
        held.data, b"ORIGINAL-EMBEDDED",
        "must not reuse ordinary replacement attachment cache"
    );
    let text = scoped
        .subtitle_encoder
        .get_subtitles(id, source_id, subtitle.index, "srt", 0, 420_000_000, true)
        .await
        .unwrap();
    assert!(
        String::from_utf8(text)
            .unwrap()
            .contains("ORIGINAL-EMBEDDED")
    );
    drop(scoped);
    f.close().await;
}

#[tokio::test]
async fn embedded_subtitle_extraction_failure_uses_real_filter_without_logging_input_capability() {
    let f = fixture().await;
    let media = f.temp.path().join("videos/Subtitled.mkv");
    subtitled_media(f.temp.path(), &media, "FALLBACK-SUBTITLE");
    assert!(
        f.wired
            .state
            .library
            .run_library_scan(ScanTrigger::Api, None)
            .await
            .unwrap()
    );
    let item = scanned_item(&f.app, &f.token, "Video", &media).await;
    let scoped = f.scoped(&item).await;
    let source = scoped
        .media_sources
        .get_stream_media_source(uuid::Uuid::parse_str(&item).unwrap(), None, None)
        .await
        .unwrap();
    let subtitle = source
        .media_streams
        .iter()
        .find(|s| s.stream_type == ferrofin_model::entities::MediaStreamType::Subtitle)
        .unwrap();
    // A file at the supported subtitle-cache root makes actual standalone
    // extraction fail. Attachment/font extraction remains available.
    let cache = f.temp.path().join("data/data/subtitles");
    assert!(!cache.exists());
    std::fs::write(&cache, b"test-owned extraction failure").unwrap();
    let failed_path = scoped
        .subtitle_encoder
        .get_subtitle_file_path(subtitle, &source)
        .await
        .unwrap();
    assert!(
        !Path::new(&failed_path).is_file(),
        "real extraction failed to produce its advertised path"
    );
    let app = ferrofin_api::create_router(scoped);
    let session = uuid::Uuid::new_v4().simple().to_string();
    let uri = format!(
        "/Videos/{item}/stream.mp4?videoCodec=h264&audioCodec=aac&videoBitrate=1000000&audioBitrate=128000&subtitleStreamIndex={}&subtitleMethod=Encode&alwaysBurnInSubtitleWhenTranscoding=true&deviceId=subtitle-fallback&playSessionId={session}",
        subtitle.index
    );
    let response = app
        .clone()
        .oneshot(request("GET", &uri, &f.token, Value::Null))
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "fallback encode: {}",
        String::from_utf8_lossy(&bytes)
    );
    let output = f.temp.path().join("fallback.mp4");
    std::fs::write(&output, bytes).unwrap();
    assert!(
        std::process::Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(&output)
            .args(["-f", "null", "-"])
            .status()
            .unwrap()
            .success(),
        "real fallback encode decodes"
    );
    let logs: Vec<_> = std::fs::read_dir(f.temp.path().join("data/log"))
        .unwrap()
        .flatten()
        .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default())
        .collect();
    assert!(
        logs.iter()
            .any(|log| log.contains("subtitles=") && log.contains("<opened-input>")),
        "actual planner fallback reached the spawned command header"
    );
    assert!(logs.iter().all(|log| !log.contains("/proc/")));
    // The private router is only this test's captured viewer; release through
    // its dedicated owner rather than generic legacy control.
    f.wired
        .state
        .hls
        .stop_owned_encoding(
            uuid::Uuid::parse_str(&f.viewer).unwrap(),
            "subtitle-fallback",
            &session,
        )
        .await
        .unwrap();
    drop(app);
    f.close().await;
}

#[tokio::test]
async fn current_pinned_policy_is_fresh_for_its_captured_viewer_and_keeps_direct_play_allowed() {
    use ferrofin_db::{enums::PermissionKind, store::guid_to_db};
    let f = fixture().await;
    assert!(
        f.wired
            .state
            .media_sources
            .current_pinned_policy()
            .await
            .is_err()
    );
    let scoped = f.scoped(&f.video).await;
    let policy = scoped.media_sources.current_pinned_policy().await.unwrap();
    assert!(
        policy.enable_media_playback
            && policy.enable_content_downloading
            && policy.playback.audio_transcoding
            && policy.playback.video_transcoding
    );
    let other = Database::connect(&f.database_url).await.unwrap();
    for permission in [
        PermissionKind::EnableContentDownloading,
        PermissionKind::EnableAudioPlaybackTranscoding,
        PermissionKind::EnableVideoPlaybackTranscoding,
        PermissionKind::EnablePlaybackRemuxing,
    ] {
        assert_eq!(
            sqlx::query(r#"UPDATE "Permissions" SET "Value"=0 WHERE "UserId"=? AND "Kind"=?"#)
                .bind(guid_to_db(uuid::Uuid::parse_str(&f.viewer).unwrap()))
                .bind(permission as i32)
                .execute(other.writer())
                .await
                .unwrap()
                .rows_affected(),
            1
        );
    }
    let policy = scoped.media_sources.current_pinned_policy().await.unwrap();
    assert!(policy.enable_media_playback);
    assert!(
        !policy.enable_content_downloading
            && !policy.playback.audio_transcoding
            && !policy.playback.video_transcoding
            && !policy.playback.remuxing
    );
    let app = ferrofin_api::create_router(scoped);
    let response = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/Videos/{}/stream.mp4?static=true", f.video),
            &f.token,
            Value::Null,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "transcode revocation does not forbid authorized direct play"
    );
    assert_eq!(
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .as_ref(),
        std::fs::read(f.temp.path().join("videos/Original.mp4")).unwrap()
    );
    drop(app);
    other.close().await;
    f.close().await;
}

#[rstest::rstest]
#[case::playback(ferrofin_db::enums::PermissionKind::EnableMediaPlayback)]
#[case::audio_transcoding(ferrofin_db::enums::PermissionKind::EnableAudioPlaybackTranscoding)]
#[tokio::test]
async fn active_progressive_body_rechecks_independent_permission_revocation(
    #[case] permission: ferrofin_db::enums::PermissionKind,
) {
    let f = fixture_with_encoder(true).await;
    std::fs::write(f.temp.path().join("realtime-enabled"), b"realtime").unwrap();
    let lease = f.lease(&f.item, "audio").await;
    let response = f
        .app
        .clone()
        .oneshot(request(
            "GET",
            lease["mediaUrl"].as_str().unwrap(),
            &f.token,
            Value::Null,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let active_pids: Vec<u32> = std::fs::read_dir(f.temp.path())
        .unwrap()
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_prefix("realtime-")?
                .parse()
                .ok()
        })
        .collect();
    assert_eq!(active_pids.len(), 1);
    assert!(
        active_pids
            .iter()
            .all(|pid| Path::new(&format!("/proc/{pid}")).exists())
    );
    let other = Database::connect(&f.database_url).await.unwrap();
    assert_eq!(
        sqlx::query(r#"UPDATE "Permissions" SET "Value"=0 WHERE "UserId"=? AND "Kind"=?"#)
            .bind(ferrofin_db::store::guid_to_db(
                uuid::Uuid::parse_str(&f.viewer).unwrap()
            ))
            .bind(permission as i32)
            .execute(other.writer())
            .await
            .unwrap()
            .rows_affected(),
        1
    );
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    assert!(
        bytes.len() <= 128 * 1024,
        "stalled reader receives only bounded already-authorized frames"
    );
    let (status, _) = call(
        &f.app,
        "GET",
        lease["mediaUrl"].as_str().unwrap(),
        &f.token,
        Value::Null,
    )
    .await;
    assert!(!status.is_success(), "revoked active lease must retire");
    call(
        &f.app,
        "DELETE",
        lease["releaseUrl"].as_str().unwrap(),
        &f.token,
        Value::Null,
    )
    .await;
    other.close().await;
    f.close().await;
}

#[rstest::rstest]
#[case::audio(
    "audio",
    ferrofin_db::enums::PermissionKind::EnableAudioPlaybackTranscoding
)]
#[case::video(
    "video",
    ferrofin_db::enums::PermissionKind::EnableVideoPlaybackTranscoding
)]
#[case::video_audio(
    "video",
    ferrofin_db::enums::PermissionKind::EnableAudioPlaybackTranscoding
)]
#[tokio::test]
async fn admitted_lease_rechecks_required_transcode_operation_before_first_get(
    #[case] kind: &str,
    #[case] permission: ferrofin_db::enums::PermissionKind,
) {
    let f = fixture_with_encoder(true).await;
    let item = if kind == "audio" { &f.item } else { &f.video };
    let lease = f.lease(item, kind).await;
    let other = Database::connect(&f.database_url).await.unwrap();
    for permission in [
        permission,
        ferrofin_db::enums::PermissionKind::EnablePlaybackRemuxing,
    ] {
        assert_eq!(
            sqlx::query(r#"UPDATE "Permissions" SET "Value"=0 WHERE "UserId"=? AND "Kind"=?"#)
                .bind(ferrofin_db::store::guid_to_db(
                    uuid::Uuid::parse_str(&f.viewer).unwrap()
                ))
                .bind(permission as i32)
                .execute(other.writer())
                .await
                .unwrap()
                .rows_affected(),
            1
        );
    }
    std::fs::write(f.temp.path().join("realtime-enabled"), b"realtime").unwrap();
    let response = f
        .app
        .clone()
        .oneshot(request(
            "GET",
            lease["mediaUrl"].as_str().unwrap(),
            &f.token,
            Value::Null,
        ))
        .await
        .unwrap();
    let status = response.status();
    drop(response);
    let spawned = std::fs::read_dir(f.temp.path())
        .unwrap()
        .flatten()
        .any(|e| {
            e.file_name()
                .to_str()
                .and_then(|name| name.strip_prefix("realtime-"))
                .is_some_and(|pid| pid.parse::<u32>().is_ok())
        });
    let registered = f
        .wired
        .state
        .hls
        .get_transcoding_job(lease["playSessionId"].as_str().unwrap())
        .await
        .unwrap()
        .is_some();
    call(
        &f.app,
        "DELETE",
        lease["releaseUrl"].as_str().unwrap(),
        &f.token,
        Value::Null,
    )
    .await;
    other.close().await;
    f.close().await;
    assert!(
        !status.is_success(),
        "an already admitted {kind} lease must enforce the freshly revoked required operation, got {status}"
    );
    assert!(
        !spawned,
        "refused dispatch must precede an actual encoder spawn"
    );
    assert!(
        !registered,
        "refused operation cannot create a transcode job"
    );
}

async fn ordinary_viewer_pair(f: &Fixture) -> (String, String, String) {
    let (admin, _) = sign_in(&f.app, "fixture", "fixture-password").await;
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/Users/New",
            &admin,
            json!({"Name":"other-viewer", "Password":"other-password"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (other, other_id) = sign_in(&f.app, "other-viewer", "other-password").await;
    for token in [&f.token, &other] {
        let (status, me) = call(&f.app, "GET", "/Users/Me", token, Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(me["Policy"]["IsAdministrator"], false);
        assert_eq!(me["Policy"]["EnableRemoteControlOfOtherUsers"], false);
    }
    (admin, other, other_id)
}

async fn legacy_stop_preserves_outputs(f: &Fixture, other: &str, sessions: &[&str]) -> bool {
    let mut intact = true;
    for &session in sessions {
        let handle = f
            .wired
            .state
            .hls
            .get_transcoding_job(session)
            .await
            .unwrap()
            .unwrap();
        assert!(
            Path::new(&handle.path).is_file(),
            "actual progressive output exists"
        );
        for (device, session) in [
            ("unrelated-device", session),
            (handle.device_id.as_deref().unwrap(), session),
            (handle.device_id.as_deref().unwrap(), ""),
        ] {
            let uri = format!("/Videos/ActiveEncodings?deviceId={device}&playSessionId={session}");
            let (status, _) = call(&f.app, "DELETE", &uri, other, Value::Null).await;
            assert!(
                status.is_success()
                    || matches!(status, StatusCode::FORBIDDEN | StatusCode::BAD_REQUEST)
            );
            intact &= Path::new(&handle.path).is_file()
                && f.wired
                    .state
                    .hls
                    .get_transcoding_job(handle.play_session_id.as_deref().unwrap())
                    .await
                    .unwrap()
                    .is_some();
        }
    }
    let pids: Vec<u32> = std::fs::read_dir(f.temp.path())
        .unwrap()
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_prefix("realtime-")?
                .parse()
                .ok()
        })
        .collect();
    intact
        && pids.len() == 2
        && pids
            .iter()
            .all(|pid| Path::new(&format!("/proc/{pid}")).exists())
}

async fn controller_can_stop_ordinary(
    f: &Fixture,
    admin: &str,
    other: &str,
    other_id: &str,
) -> axum::response::Response {
    let session = uuid::Uuid::new_v4().simple().to_string();
    let uri = format!(
        "/Audio/{}/stream.mp3?audioCodec=mp3&audioBitrate=128000&deviceId=controlled-fixture&playSessionId={session}",
        f.item
    );
    let response = f
        .app
        .clone()
        .oneshot(request("GET", &uri, &f.token, Value::Null))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let (_, me) = call(&f.app, "GET", "/Users/Me", other, Value::Null).await;
    let mut policy = me["Policy"].clone();
    policy["EnableRemoteControlOfOtherUsers"] = json!(true);
    assert_eq!(
        call(
            &f.app,
            "POST",
            &format!("/Users/{other_id}/Policy"),
            admin,
            policy
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let uri =
        format!("/Videos/ActiveEncodings?deviceId=controlled-fixture&playSessionId={session}");
    assert_eq!(
        call(&f.app, "DELETE", &uri, other, Value::Null).await.0,
        StatusCode::NO_CONTENT
    );
    assert!(
        f.wired
            .state
            .hls
            .get_transcoding_job(&session)
            .await
            .unwrap()
            .is_none(),
        "existing explicit controller privilege remains effective for ordinary jobs"
    );
    response
}

#[tokio::test]
async fn public_legacy_stop_requires_caller_authority_and_preserves_private_leases() {
    let f = fixture_with_encoder(true).await;
    let (admin, other, other_id) = ordinary_viewer_pair(&f).await;
    std::fs::write(f.temp.path().join("realtime-enabled"), b"realtime").unwrap();
    let lease = f.lease(&f.item, "audio").await;
    let response = f
        .app
        .clone()
        .oneshot(request(
            "GET",
            lease["mediaUrl"].as_str().unwrap(),
            &f.token,
            Value::Null,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let ordinary_session = uuid::Uuid::new_v4().simple().to_string();
    let uri = format!(
        "/Audio/{}/stream.mp3?audioCodec=mp3&audioBitrate=128000&deviceId=ordinary-fixture&playSessionId={ordinary_session}",
        f.item
    );
    let ordinary = f
        .app
        .clone()
        .oneshot(request("GET", &uri, &f.token, Value::Null))
        .await
        .unwrap();
    assert_eq!(ordinary.status(), StatusCode::OK);
    let intact = legacy_stop_preserves_outputs(
        &f,
        &other,
        &[lease["playSessionId"].as_str().unwrap(), &ordinary_session],
    )
    .await;
    // A generic stop may control an ordinary job owned by this caller.
    let uri = format!(
        "/Videos/ActiveEncodings?deviceId=ordinary-fixture&playSessionId={ordinary_session}"
    );
    assert_eq!(
        call(&f.app, "DELETE", &uri, &f.token, Value::Null).await.0,
        StatusCode::NO_CONTENT
    );
    assert!(
        f.wired
            .state
            .hls
            .get_transcoding_job(&ordinary_session)
            .await
            .unwrap()
            .is_none()
    );
    let controlled = controller_can_stop_ordinary(&f, &admin, &other, &other_id).await;
    // Even an owner's generic stop does not substitute for private lease release.
    let uri = format!(
        "/Videos/ActiveEncodings?deviceId=unrelated-device&playSessionId={}",
        lease["playSessionId"].as_str().unwrap()
    );
    call(&f.app, "DELETE", &uri, &f.token, Value::Null).await;
    let isolated = f
        .wired
        .state
        .hls
        .get_transcoding_job(lease["playSessionId"].as_str().unwrap())
        .await
        .unwrap()
        .is_some();
    for controller in [&other, &admin] {
        call(&f.app, "DELETE", &uri, controller, Value::Null).await;
    }
    let isolated = isolated
        && f.wired
            .state
            .hls
            .get_transcoding_job(lease["playSessionId"].as_str().unwrap())
            .await
            .unwrap()
            .is_some();
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
        StatusCode::NO_CONTENT
    );
    drop(response);
    drop(ordinary);
    drop(controlled);
    f.close().await;
    assert!(
        intact,
        "another authenticated ordinary viewer stopped real output/children"
    );
    assert!(isolated, "generic stop reached a private lease job");
}

#[tokio::test]
async fn dispatch_timeout_keeps_pending_input_until_explicit_stop_and_join() {
    let f = fixture_with_encoder(true).await;
    std::fs::write(f.temp.path().join("delay-enabled"), b"delay").unwrap();
    let lease = f.lease(&f.item, "audio").await;
    let req = request(
        "GET",
        lease["mediaUrl"].as_str().unwrap(),
        &f.token,
        Value::Null,
    );
    let app = f.app.clone();
    let pending = tokio::spawn(async move { app.oneshot(req).await.unwrap() });
    let pids = pending_encoders(f.temp.path(), 1).await;
    assert_eq!(pending.await.unwrap().status(), StatusCode::GATEWAY_TIMEOUT);
    let input = f.temp.path().join("music/Original.wav");
    assert_eq!(
        input_descriptor_count(&input),
        1,
        "timeout does not release a late encoder's capability"
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
        StatusCode::NO_CONTENT
    );
    await_retirement(f.temp.path(), &input, &pids).await;
    f.close().await;
}

#[tokio::test]
async fn failed_late_ffmpeg_input_open_never_falls_back_to_valid_replacement() {
    let f = fixture_with_encoder(true).await;
    std::fs::write(f.temp.path().join("delay-enabled"), b"delay").unwrap();
    let lease = f.lease(&f.item, "audio").await;
    let app = f.app.clone();
    let req = request(
        "GET",
        lease["mediaUrl"].as_str().unwrap(),
        &f.token,
        Value::Null,
    );
    let pending = tokio::spawn(async move { app.oneshot(req).await.unwrap() });
    let pids = pending_encoders(f.temp.path(), 1).await;
    let original = f.temp.path().join("music/Original.wav");
    let retired = f.temp.path().join("retired.wav");
    std::fs::rename(&original, &retired).unwrap();
    generate(&original);
    // Pinning does not freeze mutable bytes. Make the captured inode invalid
    // while the replacement remains decodable. A fallback would wrongly play it.
    std::fs::write(&retired, b"CORRUPT-CAPTURED-INPUT").unwrap();
    std::fs::write(f.temp.path().join("open-gate"), b"open").unwrap();
    let response = pending.await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("/proc/"));
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
        StatusCode::NO_CONTENT
    );
    await_retirement(f.temp.path(), &retired, &pids).await;
    f.close().await;
}

#[tokio::test]
async fn actual_child_panic_and_failed_start_retain_input_until_reaped() {
    use ferrofin_mediaencoding::{SegmentTranscoder, SpawnRequest, TokioSegmentTranscoder};
    for fail_log_open in [false, true] {
        let f = fixture_with_encoder(true).await;
        std::fs::write(f.temp.path().join("delay-enabled"), b"delay").unwrap();
        let scoped = f.scoped(&f.item).await;
        let source = scoped
            .media_sources
            .get_stream_media_source(uuid::Uuid::parse_str(&f.item).unwrap(), None, None)
            .await
            .unwrap();
        let owner = std::sync::Arc::downgrade(&scoped.media_sources);
        let transcoder = TokioSegmentTranscoder::new().with_input_owner(
            ferrofin_mediaencoding::transcoding::retained_input::InputProcessBinding::new(
                scoped.media_sources.clone(),
                std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
            ),
        );
        let req = SpawnRequest {
            program: f
                .temp
                .path()
                .join("delayed-encoder")
                .to_string_lossy()
                .into_owned(),
            arguments: vec![
                "-nostdin".into(),
                "-v".into(),
                "error".into(),
                "-i".into(),
                source.path.unwrap(),
                "-y".into(),
                f.temp
                    .path()
                    .join("panic.mp3")
                    .to_string_lossy()
                    .into_owned(),
            ],
            working_dir: None,
            output_dir: f.temp.path().to_owned(),
            log_path: if fail_log_open {
                f.temp.path().to_owned()
            } else {
                f.temp.path().join("panic.log")
            },
            env: Vec::new(),
        };
        drop(scoped);
        if fail_log_open {
            assert!(transcoder.start_transcode(&req).await.is_err());
        } else {
            let child = transcoder.start_transcode(&req).await.unwrap();
            assert!(
                transcoder.start_transcode(&req).await.is_err(),
                "retained child exhausts the shared process budget before a second fork"
            );
            pending_encoders(f.temp.path(), 1).await;
            let panicked = tokio::spawn(async move {
                let _child = child;
                panic!("synthetic post-spawn unwind");
            })
            .await;
            assert!(panicked.unwrap_err().is_panic());
        }
        drop(transcoder);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while owner.upgrade().is_some() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("child reaped before final source owner release");
        assert_eq!(
            input_descriptor_count(&f.temp.path().join("music/Original.wav")),
            0
        );
        f.close().await;
    }
}

#[tokio::test]
async fn idle_expiry_retires_actual_scoped_input() {
    let f = fixture().await;
    for task in &f.wired.background {
        task.abort();
    }
    f.wired.task_manager.shutdown().await;
    let lease = f.lease(&f.item, "audio").await;
    let input = f.temp.path().join("music/Original.wav");
    assert_eq!(input_descriptor_count(&input), 1);
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(92)).await;
    assert_eq!(
        call(
            &f.app,
            "HEAD",
            lease["mediaUrl"].as_str().unwrap(),
            &f.token,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    for _ in 0..128 {
        if input_descriptor_count(&input) == 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    tokio::time::resume();
    assert_eq!(
        input_descriptor_count(&input),
        0,
        "expiry releases the actual opened input"
    );
    f.close().await;
}
