//! Real normal-library/Core/DB/auth -> Rust decode/resample -> official model ->
//! authenticated HTTP bytes. Only original synthetic audio and disposable users.
#![allow(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)]
use axum::response::IntoResponse;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
};
use ferrofin_api::karaoke::{Config as KaraokeConfig, Service};
use ferrofin_db::{Database, store::guid_to_db};
use ferrofin_model::users::UserPolicy;
use ferrofin_server::{
    config::Config,
    state::{WiredApp, build_app_state},
};
use ferrofin_traits::library::ScanTrigger;
use serde_json::{Value, json};
use std::{
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    time::Duration,
};
use tower::ServiceExt as _;
use uuid::Uuid;

fn original(rate: u32, seconds: u32) -> Vec<u8> {
    let frames = rate * seconds;
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((frames * 2 + 36).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(rate.to_le_bytes());
    bytes.extend((rate * 2).to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend((frames * 2).to_le_bytes());
    for index in 0..frames {
        let t = f64::from(index) / f64::from(rate);
        let sample = ((t * 440. * std::f64::consts::TAU).sin() * 6000.
            + (t * 180. * std::f64::consts::TAU).sin() * 3000.) as i16;
        bytes.extend(sample.to_le_bytes());
    }
    bytes
}
fn permitted() -> UserPolicy {
    // The wire constructor sets normal allow flags; Rust's derived Default
    // deliberately means false and is not the ordinary new-user policy.
    serde_json::from_value(json!({})).unwrap()
}
fn request(method: &str, path: &str, token: &str, body: Value, remote: bool) -> Request<Body> {
    let mut request = Request::builder().method(method).uri(path)
        .header(header::AUTHORIZATION,format!(r#"MediaBrowser Client="karaoke-fixture",Device="fixture",DeviceId="karaoke-fixture",Version="1",Token="{token}""#))
        .header(header::CONTENT_TYPE,"application/json")
        .body(if body.is_null() { Body::empty() } else { axum::Json(body).into_response().into_body() }).unwrap();
    if remote {
        request.extensions_mut().insert(ConnectInfo(
            "203.0.113.123:41001"
                .parse::<std::net::SocketAddr>()
                .unwrap(),
        ));
    }
    request
}
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(request(method, path, token, body, false))
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn login(app: &Router, name: &str, password: &str) -> (String, Uuid) {
    let (status, value) = call(
        app,
        "POST",
        "/Users/AuthenticateByName",
        "",
        json!({"Username":name,"Pw":password}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    (
        value["AccessToken"].as_str().unwrap().to_owned(),
        Uuid::parse_str(value["User"]["Id"].as_str().unwrap()).unwrap(),
    )
}
struct Fixture {
    temp: tempfile::TempDir,
    db: Database,
    wired: WiredApp,
    app: Router,
    admin: String,
    token: String,
    user: Uuid,
    other: String,
    item: Uuid,
    source: String,
}
impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        for name in ["config", "data", "cache", "music", "timing"] {
            std::fs::create_dir(temp.path().join(name)).unwrap();
        }
        std::fs::set_permissions(
            temp.path().join("timing"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        std::fs::write(temp.path().join("music/Original.wav"), original(48000, 2)).unwrap();
        let encoder = std::env::var_os("BLUEFIN_TEST_FFMPEG")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("PATH").and_then(|paths| {
                    std::env::split_paths(&paths)
                        .map(|directory| directory.join("ffmpeg"))
                        .find(|path| path.is_file())
                })
            })
            .expect("Install FFmpeg on PATH or set BLUEFIN_TEST_FFMPEG")
            .canonicalize()
            .expect("FFmpeg path must resolve to an installed executable");
        let probe = std::env::var_os("BLUEFIN_TEST_FFPROBE").map_or_else(
            || encoder.with_file_name("ffprobe"),
            std::path::PathBuf::from,
        );
        let config = Config {
            // Upstream encoding/probing is fixture setup only. Preparation
            // itself must decode these encoded FLAC bytes in Rust.
            admin_user: "operator".into(),
            admin_password: "operator-password".into(),
            ffmpeg_path: Some(encoder),
            ffprobe_path: Some(probe),
            ..Config::test_stub(temp.path())
        };
        assert!(
            std::process::Command::new(config.ffmpeg_path.as_ref().unwrap())
                .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
                .arg(temp.path().join("music/Original.wav"))
                .args(["-c:a", "flac", "-y"])
                .arg(temp.path().join("music/Original.flac"))
                .status()
                .unwrap()
                .success()
        );
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
        let (admin, _) = login(&app, "operator", "operator-password").await;
        let (status,value) = call(&app,"POST",&format!("/Library/VirtualFolders?name=Music&collectionType=music&paths={}&refreshLibrary=false",temp.path().join("music").display()),&admin,Value::Null).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{value}");
        assert!(
            wired
                .state
                .library
                .run_library_scan(ScanTrigger::Api, None)
                .await
                .unwrap()
        );
        for name in ["singer", "other"] {
            let (status, value) = call(
                &app,
                "POST",
                "/Users/New",
                &admin,
                json!({"Name":name,"Password":"fixture-password"}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{value}");
        }
        let (token, user) = login(&app, "singer", "fixture-password").await;
        let (other, _) = login(&app, "other", "fixture-password").await;
        let (status, items) = call(
            &app,
            "GET",
            "/Items?recursive=true&includeItemTypes=Audio&fields=Path",
            &token,
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{items}");
        let audio_item = items["Items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["Path"].as_str() == temp.path().join("music/Original.flac").to_str())
            .unwrap();
        let item = Uuid::parse_str(audio_item["Id"].as_str().unwrap()).unwrap();
        let sources = wired
            .state
            .media_sources
            .get_static_media_sources(item, false, Some(user))
            .await
            .unwrap();
        let source = sources[0].id.clone().unwrap();
        Self {
            temp,
            db,
            wired,
            app,
            admin,
            token,
            user,
            other,
            item,
            source,
        }
    }
    fn settings(&self, model: &Path) -> KaraokeConfig {
        KaraokeConfig {
            model_archive: model.into(),
            cache_root: self.temp.path().join("owned-model-cache"),
            timing_root: self.temp.path().join("timing"),
            cache_bytes: 64 * 1024 * 1024,
            cpu_threads: 3,
            deadline_seconds: 60,
        }
    }
    fn timing(&self) {
        // Rights/timing are supplied only through trusted server metadata, scoped
        // to the real normal-library source and original encoded-byte digest.
        let folder = self.temp.path().join("timing").join(self.item.to_string());
        std::fs::create_dir_all(&folder).unwrap();
        let hash =
            bluefin_hash(&std::fs::read(self.temp.path().join("music/Original.flac")).unwrap());
        std::fs::write(folder.join(format!("{}.json",self.source)),serde_json::to_vec(&json!({
            "itemId":self.item,"mediaSourceId":self.source,"sourceSha256":hash,
            "rightsReference":"Original synthetic fixture authored for this qualification; operator permits separation and playback",
            "timingKind":"word","timeline":{"offsetSeconds":0.,"lines":[{"id":"line1","voice":"lead","tokens":[{"text":"Test","start":0.1,"end":0.9}]}]}
        })).unwrap()).unwrap();
    }
    fn preparation(&self) -> String {
        format!("/Bluefin/Karaoke/{}/Preparation", self.item)
    }
    async fn post(&self, id: Uuid, token: &str) -> Value {
        let (status, value) = call(
            &self.app,
            "POST",
            &self.preparation(),
            token,
            json!({"requestId":id,"mediaSourceId":self.source}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value
    }
    async fn wait(&self, id: Uuid, token: &str) -> Value {
        for _ in 0..600 {
            let (status, value) = call(
                &self.app,
                "GET",
                &format!("{}/{id}", self.preparation()),
                token,
                Value::Null,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{value}");
            if !matches!(
                value["state"].as_str(),
                Some("queued" | "preparing" | "cancelling")
            ) {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("bounded fixture preparation did not join");
    }
    async fn policy(&self, policy: UserPolicy) {
        // An independent writer bypasses normal manager cache invalidation and
        // policy-save token revocation, proving these reads are genuinely fresh.
        use ferrofin_db::enums::PermissionKind as P;
        for (kind, value) in [
            (P::EnableRemoteAccess, policy.enable_remote_access),
            (
                P::EnableAudioPlaybackTranscoding,
                policy.enable_audio_playback_transcoding,
            ),
            (
                P::EnableContentDownloading,
                policy.enable_content_downloading,
            ),
            (P::EnableMediaPlayback, policy.enable_media_playback),
            (P::EnableAllFolders, policy.enable_all_folders),
            (P::EnableAllDevices, policy.enable_all_devices),
        ] {
            let result = sqlx::query(
                r#"UPDATE "Permissions" SET "Value"=?1 WHERE "UserId"=?2 AND "Kind"=?3"#,
            )
            .bind(value)
            .bind(guid_to_db(self.user))
            .bind(i32::from(kind))
            .execute(self.db.pool())
            .await
            .unwrap();
            assert_eq!(result.rows_affected(), 1);
        }
    }
    async fn preparing(&self, id: Uuid) {
        for _ in 0..200 {
            let (status, body) = call(
                &self.app,
                "GET",
                &format!("{}/{id}", self.preparation()),
                &self.token,
                Value::Null,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            if body["state"] == "preparing" {
                return;
            }
            assert_eq!(
                body["state"], "queued",
                "expected an active model job: {body}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("bounded fixture did not observe preparation");
    }
    async fn joined(&self, service: &Service) {
        for _ in 0..200 {
            if service.testing_resource_counts() == (3, 1) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("revoked work did not join and release admission");
    }
    async fn close(self) {
        if let Some(service) = self.wired.state.karaoke() {
            service.shutdown().await;
        }
        self.wired.state.library.shutdown_scans().await;
        self.wired.task_manager.shutdown().await;
        for task in self.wired.background {
            task.abort();
            let _ = task.await;
        }
        self.db.close().await;
    }
}
fn bluefin_hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .flat_map(|byte| {
            let hex = b"0123456789abcdef";
            [
                char::from(hex[usize::from(byte >> 4)]),
                char::from(hex[usize::from(byte & 15)]),
            ]
        })
        .collect()
}

#[tokio::test]
#[ignore = "requires original official BLUEFIN_MODEL_ARCHIVE and startup thread settings"]
async fn normal_library_auth_decode_official_model_serving_and_lifecycle() {
    let archive = std::env::var_os("BLUEFIN_MODEL_ARCHIVE").expect("official archive path");
    let mut f = Fixture::new().await;
    let (status, capabilities) = call(
        &f.app,
        "GET",
        "/Bluefin/Capabilities",
        &f.token,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(capabilities["karaokePreparation"]["enabled"], false);
    assert_eq!(
        call(
            &f.app,
            "POST",
            &f.preparation(),
            &f.token,
            json!({"requestId":Uuid::new_v4()})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    // Existing supplied packages remain valid with the legacy absent fields.
    let supplied = std::path::PathBuf::from(
        f.wired
            .state
            .config
            .application_paths()
            .internal_metadata_path(),
    )
    .join("bluefin-karaoke")
    .join(f.item.to_string());
    std::fs::create_dir_all(&supplied).unwrap();
    let input = std::fs::read(f.temp.path().join("music/Original.wav")).unwrap();
    let encoded_input = std::fs::read(f.temp.path().join("music/Original.flac")).unwrap();
    let source_hash = bluefin_hash(&encoded_input);
    let stem_hash = bluefin_hash(&input);
    let old = json!({"itemId":f.item,"sourceSha256":source_hash,"durationSeconds":2.,
        "rightsReference":"Original synthetic fixture, supplied stems permitted",
        "accompaniment":{"sha256":stem_hash,"bytes":input.len()},"leadVocal":{"sha256":stem_hash,"bytes":input.len()},
        "timeline":{"offsetSeconds":0.,"lines":[{"id":"supplied","voice":"lead","tokens":[{"text":"Test","start":0.1,"end":0.9}]}]}});
    std::fs::write(
        supplied.join("manifest.json"),
        serde_json::to_vec(&old).unwrap(),
    )
    .unwrap();
    for name in ["accompaniment.wav", "lead-vocal.wav"] {
        std::fs::write(supplied.join(name), &input).unwrap();
    }
    let (status, legacy) = call(
        &f.app,
        "GET",
        &format!("/Bluefin/Karaoke/{}", f.item),
        &f.token,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{legacy}");
    assert_eq!(legacy["processing"], "suppliedStems");
    assert!(legacy.get("mediaSourceId").is_none());
    let bad = f.temp.path().join("bad-model");
    std::fs::write(&bad, b"bad official hash").unwrap();
    assert!(Service::start(f.settings(&bad)).await.is_err());
    let mut invalid = f.settings(Path::new(&archive));
    invalid.cpu_threads = 5;
    assert!(Service::start(invalid).await.is_err());
    let settings = f.settings(Path::new(&archive));
    let service = Service::start(settings.clone()).await.unwrap();
    f.wired.state = f.wired.state.clone().with_karaoke(Some(service.clone()));
    f.app = ferrofin_api::create_router(f.wired.state.clone());
    let missing = Uuid::new_v4();
    f.post(missing, &f.token).await;
    let status = f.wait(missing, &f.token).await;
    assert_eq!(status["state"], "unavailable", "{status}");
    assert_eq!(status["reason"], "timingRequired");
    f.timing();
    let timing_path = settings
        .timing_root
        .join(f.item.to_string())
        .join(format!("{}.json", f.source));
    let timing_before = std::fs::read(&timing_path).unwrap();
    let mut wrong: Value = serde_json::from_slice(&timing_before).unwrap();
    wrong["sourceSha256"] = json!("a".repeat(64));
    std::fs::write(&timing_path, serde_json::to_vec(&wrong).unwrap()).unwrap();
    let mismatch = Uuid::new_v4();
    f.post(mismatch, &f.token).await;
    assert_eq!(f.wait(mismatch, &f.token).await["reason"], "timingRequired");
    std::fs::write(&timing_path, &timing_before).unwrap();
    sqlx::query(r#"UPDATE "BaseItems" SET "RunTimeTicks"=3010000000 WHERE "Id"=?1"#)
        .bind(guid_to_db(f.item))
        .execute(f.db.pool())
        .await
        .unwrap();
    let long = Uuid::new_v4();
    f.post(long, &f.token).await;
    assert_eq!(
        f.wait(long, &f.token).await["reason"],
        "durationUnsupported"
    );
    // Rounded catalogue duration differs from the actual two-second decoded PCM.
    sqlx::query(r#"UPDATE "BaseItems" SET "RunTimeTicks"=20100000 WHERE "Id"=?1"#)
        .bind(guid_to_db(f.item))
        .execute(f.db.pool())
        .await
        .unwrap();
    let tombstone = Uuid::new_v4();
    let (status, value) = call(
        &f.app,
        "DELETE",
        &format!("{}/{tombstone}", f.preparation()),
        &f.token,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["state"], "cancelled");
    assert_eq!(f.post(tombstone, &f.token).await["state"], "cancelled");
    assert_eq!(service.testing_resource_counts(), (3, 1));
    let ready = Uuid::new_v4();
    f.post(ready, &f.token).await;
    assert_eq!(
        call(
            &f.app,
            "GET",
            &format!("{}/{ready}", f.preparation()),
            &f.other,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let result = f.wait(ready, &f.token).await;
    assert_eq!(result["state"], "ready", "{result}");
    assert_eq!(result["mediaSourceId"], f.source);
    assert_eq!(service.testing_resource_counts(), (3, 1));
    assert_eq!(f.post(ready, &f.token).await["state"], "ready");
    let manifest_path = format!("/Bluefin/Karaoke/{}?MediaSourceId={}", f.item, f.source);
    let (status, manifest) = call(&f.app, "GET", &manifest_path, &f.token, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{manifest}");
    assert_eq!(manifest["processing"], "modelSeparatedStems");
    assert_eq!(
        manifest["processingReference"],
        "deezer/spleeter/2stems@v1.4.0"
    );
    assert_eq!(manifest["mediaSourceId"], f.source);
    assert_eq!(manifest["durationSeconds"], 2.0);
    for forbidden in [
        "sourceSha256",
        "inputPcmSha256",
        "rightsReference",
        "generation",
        "ownerScopeSha256",
    ] {
        assert!(manifest.get(forbidden).is_none());
    }
    let private = json!({"manifest":manifest,"status":result}).to_string();
    assert!(!private.contains(&f.token));
    assert!(!private.contains("/proc/"));
    assert!(!private.contains(f.temp.path().to_str().unwrap()));
    for (role, field) in [
        ("accompaniment", "accompaniment"),
        ("lead-vocal", "leadVocal"),
    ] {
        let uri = format!(
            "/Bluefin/Karaoke/{}/{}/Stems/{role}?MediaSourceId={}",
            f.item,
            manifest["revision"].as_str().unwrap(),
            f.source
        );
        let response = f
            .app
            .clone()
            .oneshot(request("GET", &uri, &f.token, Value::Null, false))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        assert_eq!(bluefin_hash(&bytes), manifest[field]["sha256"]);
        assert_eq!(bytes.len() as u64, manifest[field]["bytes"]);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 2 * 44100 * 4 + 44);
        // Native admission permits at most one decoded sample of discrepancy.
        let rate = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
        let block = u16::from_le_bytes(bytes[32..34].try_into().unwrap());
        let data = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
        let duration = f64::from(data / u32::from(block)) / f64::from(rate);
        assert!(
            (manifest["durationSeconds"].as_f64().unwrap() - duration).abs()
                <= 1.0 / f64::from(rate)
        );
        if let Some(root) = std::env::var_os("BLUEFIN_KARAOKE_RECEIPT_DIR") {
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(
                std::path::PathBuf::from(&root).join(format!("{role}.wav")),
                &bytes,
            )
            .unwrap();
            std::fs::write(
                std::path::PathBuf::from(&root).join("public-manifest.json"),
                serde_json::to_vec_pretty(&manifest).unwrap(),
            )
            .unwrap();
            std::fs::write(
                std::path::PathBuf::from(&root).join("original-encoded.flac"),
                &encoded_input,
            )
            .unwrap();
            let cache = std::fs::read_dir(settings.cache_root.join("cache"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let receipt = std::fs::read(cache.join("receipt.json")).unwrap();
            let custody: Value = serde_json::from_slice(&receipt).unwrap();
            assert_eq!(custody["sourceSha256"], source_hash);
            assert_ne!(custody["sourceSha256"], custody["inputPcmSha256"]);
            assert_eq!(custody["engine"]["frames"], 2 * 44100);
            assert_eq!(custody["engine"]["cpuThreads"], 3);
            std::fs::write(
                std::path::PathBuf::from(&root).join("private-preparation-receipt.json"),
                receipt,
            )
            .unwrap();
        }
    }
    // Another real account may prepare the same admitted cache, but never owns
    // the first account/session's job identifier.
    f.post(ready, &f.other).await;
    assert_eq!(f.wait(ready, &f.other).await["state"], "ready");
    let (status, _) = call(
        &f.app,
        "POST",
        "/Auth/Keys?app=fixture",
        &f.admin,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, keys) = call(&f.app, "GET", "/Auth/Keys", &f.admin, Value::Null).await;
    let key = keys["Items"][0]["AccessToken"].as_str().unwrap();
    assert!(matches!(
        call(&f.app, "GET", &manifest_path, key, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ));
    f.policy(UserPolicy {
        enable_remote_access: false,
        ..permitted()
    })
    .await;
    let remote = f
        .app
        .clone()
        .oneshot(request("GET", &manifest_path, &f.token, Value::Null, true))
        .await
        .unwrap();
    assert_eq!(remote.status(), StatusCode::FORBIDDEN);
    f.policy(UserPolicy {
        enable_audio_playback_transcoding: false,
        ..permitted()
    })
    .await;
    assert_eq!(
        call(&f.app, "GET", &manifest_path, &f.token, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    for policy in [
        UserPolicy {
            enable_content_downloading: false,
            ..permitted()
        },
        UserPolicy {
            enable_media_playback: false,
            ..permitted()
        },
        UserPolicy {
            enable_all_devices: false,
            ..permitted()
        },
    ] {
        f.policy(policy).await;
        assert_eq!(
            call(&f.app, "GET", &manifest_path, &f.token, Value::Null)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    f.policy(UserPolicy {
        enable_all_folders: false,
        ..permitted()
    })
    .await;
    assert!(matches!(
        call(&f.app, "GET", &manifest_path, &f.token, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
    ));
    f.policy(permitted()).await;
    // In-place byte change (same inode and size), path replacement and version
    // changes cannot fall back to an old prepared snapshot.
    let path = f.temp.path().join("music/Original.flac");
    let saved = std::fs::read(&path).unwrap();
    let mut changed = saved.clone();
    changed[48] ^= 1;
    std::fs::write(&path, &changed).unwrap();
    assert_eq!(
        call(&f.app, "GET", &manifest_path, &f.token, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    std::fs::write(&path, &saved).unwrap();
    f.timing();
    let cancelled = Uuid::new_v4();
    f.post(cancelled, &f.token).await;
    for _ in 0..200 {
        let (_, status) = call(
            &f.app,
            "GET",
            &format!("{}/{cancelled}", f.preparation()),
            &f.token,
            Value::Null,
        )
        .await;
        if status["state"] == "preparing" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        service.testing_resource_counts().1,
        0,
        "heavy ownership retained while preparing"
    );
    let queued = Uuid::new_v4();
    f.post(queued, &f.token).await;
    let queued2 = Uuid::new_v4();
    f.post(queued2, &f.token).await;
    let overload = Uuid::new_v4();
    assert_eq!(f.post(overload, &f.token).await["reason"], "busy");
    for id in [queued, queued2] {
        let (_, stopping) = call(
            &f.app,
            "DELETE",
            &format!("{}/{id}", f.preparation()),
            &f.token,
            Value::Null,
        )
        .await;
        assert_eq!(stopping["state"], "cancelling");
        assert_eq!(f.wait(id, &f.token).await["state"], "cancelled");
        assert_eq!(f.post(id, &f.token).await["state"], "cancelled");
    }
    let (_, stopping) = call(
        &f.app,
        "DELETE",
        &format!("{}/{cancelled}", f.preparation()),
        &f.token,
        Value::Null,
    )
    .await;
    assert_eq!(stopping["state"], "cancelling");
    assert_eq!(f.wait(cancelled, &f.token).await["state"], "cancelled");
    assert_eq!(service.testing_resource_counts(), (3, 1));
    assert_eq!(f.post(cancelled, &f.token).await["state"], "cancelled");
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(supplied.join("manifest.json")).unwrap())
            .unwrap(),
        old
    );
    let cache_count = std::fs::read_dir(settings.cache_root.join("cache"))
        .unwrap()
        .count();
    let revoked = Uuid::new_v4();
    f.post(revoked, &f.token).await;
    f.preparing(revoked).await;
    f.policy(UserPolicy {
        enable_content_downloading: false,
        ..permitted()
    })
    .await;
    f.joined(&service).await;
    f.policy(permitted()).await;
    assert_ne!(f.wait(revoked, &f.token).await["state"], "ready");
    assert_eq!(
        std::fs::read_dir(settings.cache_root.join("cache"))
            .unwrap()
            .count(),
        cache_count
    );
    let revoked_token = Uuid::new_v4();
    f.post(revoked_token, &f.token).await;
    f.preparing(revoked_token).await;
    // Logout through an independent DB write must revoke active work and a
    // warmed token, without a cached AuthorizationInfo granting late output.
    sqlx::query(r#"DELETE FROM "Devices" WHERE "AccessToken"=?1"#)
        .bind(&f.token)
        .execute(f.db.pool())
        .await
        .unwrap();
    f.joined(&service).await;
    assert_eq!(
        std::fs::read_dir(settings.cache_root.join("cache"))
            .unwrap()
            .count(),
        cache_count
    );
    assert_eq!(
        call(&f.app, "GET", &manifest_path, &f.token, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    service.shutdown().await;
    assert_eq!(service.testing_resource_counts(), (3, 1));
    assert_eq!(
        std::fs::read_dir(settings.cache_root.join("staging"))
            .unwrap()
            .count(),
        0
    );
    f.wired.state = f.wired.state.clone().with_karaoke(None);
    f.app = ferrofin_api::create_router(f.wired.state.clone());
    drop(service);
    // Simulate a process that died after a partial owned staging write. The
    // next real engine lifetime must recover it and retain stopped requests.
    let orphan = settings
        .cache_root
        .join("cache")
        .join(format!("pending-{}", Uuid::new_v4().simple()));
    std::fs::create_dir(&orphan).unwrap();
    std::fs::set_permissions(&orphan, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(orphan.join("manifest.json"))
        .unwrap()
        .write_all(b"partial crash fixture")
        .unwrap();
    let mut quota = settings.clone();
    quota.cache_bytes = 2 * 1024 * 1024;
    let restarted = Service::start(quota).await.unwrap();
    assert!(!orphan.exists());
    f.wired.state = f.wired.state.clone().with_karaoke(Some(restarted.clone()));
    f.app = ferrofin_api::create_router(f.wired.state.clone());
    assert_eq!(
        f.post(ready, &f.other).await["state"],
        "cancelled",
        "restart cannot resurrect an earlier request"
    );
    let quota = Uuid::new_v4();
    f.post(quota, &f.other).await;
    let result = f.wait(quota, &f.other).await;
    assert_eq!(result["reason"], "busy", "{result}");
    assert_eq!(restarted.testing_resource_counts(), (3, 1));
    f.close().await;
    drop(restarted);
}
