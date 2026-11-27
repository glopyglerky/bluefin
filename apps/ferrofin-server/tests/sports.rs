//! Managed pairing through the real router, database, session manager and cache.
//! All users and credentials belong to a fresh, disposable test database.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ferrofin_server::config::Config;
use ferrofin_server::state::{WiredApp, build_app_state};
use tower::{Layer as _, ServiceExt as _};

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

#[tokio::test]
async fn sports_route_exists() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["config", "data", "cache"] {
        std::fs::create_dir_all(temp.path().join(name)).unwrap();
    }
    let config = Config::test_stub(temp.path());
    let db = ferrofin_db::Database::connect(&config.database_url())
        .await
        .unwrap();
    db.run_migrations().await.unwrap();
    let wired = wire(&db, &config).await;
    let router = ferrofin_api::create_router(wired.state.clone());
    for path in [
        "/sports/api/v1/preferences",
        "/sports/api/v1/scoreboard?league=nhl&date=2026-11-27",
        "/sports/api/v1/events/nhl/401892781",
        "/sports/api/v1/standings/nhl",
        "/sports/api/v1/logos/0000000000000000000000000000000000000000000000000000000000000000",
    ] {
        let response = router
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
    }
    retire(wired).await;
}

#[path = "../src/base_url.rs"]
mod base_url;
use axum::http::{HeaderMap, header};
use ferrofin_db::entities::display_preferences::{
    DisplayPreferencesEntity, HomeSectionEntity, ItemDisplayPreferencesEntity,
};
use ferrofin_db::store::guid_to_db;
use ferrofin_traits::error::ServiceError;
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;
const PREFS: &str = "/sports/api/v1/preferences";
const HOME: &str = "/sports/api/v1/home";
const CLIENT: &str = "bluefin-sports-v1";
async fn call(
    router: &axum::Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    tags: &[&str],
    body: Vec<u8>,
) -> (StatusCode, HeaderMap, Value) {
    let auth = token.map_or_else(|| "MediaBrowser Client=\"Synthetic\", Device=\"Synthetic\", Version=\"1\", DeviceId=\"sports-test\"".to_owned(), |t| format!("MediaBrowser Client=\"Synthetic\", Device=\"Synthetic\", Version=\"1\", DeviceId=\"sports-test\", Token=\"{t}\""));
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::AUTHORIZATION, auth)
        .header(header::CONTENT_TYPE, "application/json");
    for tag in tags {
        req = req.header(header::IF_MATCH, *tag);
    }
    let response = router
        .clone()
        .oneshot(req.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .unwrap();
    (
        status,
        headers,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn json_call(
    router: &axum::Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    tags: &[&str],
    body: Value,
) -> (StatusCode, HeaderMap, Value) {
    call(
        router,
        method,
        path,
        token,
        tags,
        serde_json::to_vec(&body).unwrap(),
    )
    .await
}
async fn login(router: &axum::Router, name: &str) -> (String, String) {
    let (status, _, value) = json_call(
        router,
        "POST",
        "/Users/AuthenticateByName",
        None,
        &[],
        json!({"Username": name, "Pw":"synthetic-password"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    (
        value["AccessToken"].as_str().unwrap().into(),
        value["User"]["Id"].as_str().unwrap().into(),
    )
}
fn defaults() -> Value {
    serde_json::to_value(bluefin_contract::sports::Preferences::default()).unwrap()
}
async fn setup() -> (
    tempfile::TempDir,
    Config,
    ferrofin_db::Database,
    WiredApp,
    axum::Router,
    String,
    String,
    String,
) {
    let temp = tempfile::tempdir().unwrap();
    for name in ["config", "data", "cache"] {
        std::fs::create_dir_all(temp.path().join(name)).unwrap();
    }
    let config = Config {
        admin_user: "admin".into(),
        admin_password: "synthetic-password".into(),
        ..Config::test_stub(temp.path())
    };
    let db = ferrofin_db::Database::connect(&config.database_url())
        .await
        .unwrap();
    db.run_migrations().await.unwrap();
    let wired = wire(&db, &config).await;
    ferrofin_server::seed::seed_default_admin(wired.state.users.as_ref(), &config)
        .await
        .unwrap();
    let router = ferrofin_api::create_router(wired.state.clone());
    let (admin, _) = login(&router, "admin").await;
    for name in ["viewer", "other"] {
        let result = json_call(
            &router,
            "POST",
            "/Users/New",
            Some(&admin),
            &[],
            json!({"Name":name,"Password":"synthetic-password"}),
        )
        .await;
        assert_eq!(result.0, StatusCode::OK, "{}", result.2);
    }
    let (viewer, uid) = login(&router, "viewer").await;
    let (other, _) = login(&router, "other").await;
    (temp, config, db, wired, router, viewer, uid, other)
}
#[tokio::test]
#[allow(clippy::too_many_lines)] // Keep the end-to-end rejection matrix with its fixture.
async fn preferences_auth_limits_and_mounts() {
    let (_temp, _config, _db, wired, router, token, _uid, other) = setup().await;
    for base in ["", "/example"] {
        // Exact production mount middleware, composed without any listener.
        let mounted = axum::Router::new().fallback_service(
            axum::middleware::from_fn_with_state(base_url::normalize(base), base_url::rewrite)
                .layer(router.clone()),
        );
        let path = format!("{base}{PREFS}");
        assert_eq!(
            call(&mounted, "GET", &path, None, &[], vec![]).await.0,
            StatusCode::UNAUTHORIZED
        );
        let (status, headers, body) = call(&mounted, "GET", &path, Some(&token), &[], vec![]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::ETAG], "\"0\"");
        assert_eq!(body, json!({"version":0,"preferences":defaults()}));
        assert_eq!(
            call(
                &mounted,
                "GET",
                &format!("{base}{HOME}"),
                Some(&token),
                &[],
                vec![]
            )
            .await
            .2,
            json!({"schema":1,"sports_only":false,"games":[],"local_broadcasts":[],"presentation_only":true,"providers":[]})
        );
        if !base.is_empty() {
            assert_eq!(
                call(&mounted, "GET", PREFS, Some(&token), &[], vec![])
                    .await
                    .0,
                StatusCode::FOUND
            );
        }
    }
    assert_eq!(
        call(&router, "PUT", PREFS, None, &["\"0\""], vec![b'x'; 60001])
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        json_call(&router, "PUT", PREFS, Some(&token), &[], defaults())
            .await
            .0,
        StatusCode::PRECONDITION_REQUIRED
    );
    for tags in [
        vec!["0"],
        vec!["W/\"0\""],
        vec!["*"],
        vec!["\"00\""],
        vec!["\"+0\""],
        vec!["\"18446744073709551616\""],
        vec!["\"0\", \"1\""],
        vec!["\"0\"", "\"0\""],
    ] {
        assert_eq!(
            json_call(&router, "PUT", PREFS, Some(&token), &tags, defaults())
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    for (field, value) in [
        ("schema", json!(2)),
        ("home", json!("true")),
        ("teams", json!([""])),
        ("teams", json!(["x".repeat(129)])),
        ("players", json!(vec!["x"; 257])),
        ("uid", json!("other")),
        ("version", json!(0)),
    ] {
        let mut body = defaults();
        body[field] = value;
        assert_eq!(
            json_call(&router, "PUT", PREFS, Some(&token), &["\"0\""], body)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    let mut body = defaults();
    body["home"] = json!(false);
    body["away"] = json!(false);
    assert_eq!(
        json_call(&router, "PUT", PREFS, Some(&token), &["\"0\""], body)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &router,
            "PUT",
            PREFS,
            Some(&token),
            &["\"0\""],
            vec![b' '; 60001]
        )
        .await
        .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    for method in ["GET", "PUT"] {
        assert_eq!(
            json_call(
                &router,
                method,
                &format!("{PREFS}?userId=other"),
                Some(&token),
                &["\"0\""],
                defaults()
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    for method in ["GET", "POST"] {
        assert_eq!(
            json_call(
                &router,
                method,
                "/DisplayPreferences/example?client=bluefin-sports-v1",
                Some(&token),
                &[],
                json!({})
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    let mut body = defaults();
    body["auto_record"] = json!(true);
    body["teams"] = json!(["opaque-b", "opaque-a"]);
    let (status, headers, saved) =
        json_call(&router, "PUT", PREFS, Some(&token), &["\"0\""], body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::ETAG], "\"1\"");
    assert_eq!(saved["version"], 1);
    assert_eq!(
        call(&router, "GET", PREFS, Some(&other), &[], vec![])
            .await
            .2["version"],
        0
    );
    let (admin, _) = login(&router, "admin").await;
    assert_eq!(
        call(
            &router,
            "POST",
            "/Auth/Keys?app=synthetic",
            Some(&admin),
            &[],
            vec![]
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let keys = call(&router, "GET", "/Auth/Keys", Some(&admin), &[], vec![])
        .await
        .2;
    let key = keys["Items"][0]["AccessToken"].as_str().unwrap();
    for (method, path) in [
        ("GET", PREFS),
        ("PUT", PREFS),
        ("GET", HOME),
        (
            "GET",
            "/sports/api/v1/scoreboard?league=nhl&date=2026-11-27",
        ),
        ("GET", "/sports/api/v1/events/nhl/401892781"),
        ("GET", "/sports/api/v1/standings/nhl"),
        (
            "GET",
            "/sports/api/v1/logos/0000000000000000000000000000000000000000000000000000000000000000",
        ),
    ] {
        assert_eq!(
            call(
                &router,
                method,
                path,
                Some(key),
                &["\"1\""],
                vec![b'x'; 60001]
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        call(
            &router,
            "GET",
            PREFS,
            Some("invalid-synthetic-token"),
            &[],
            vec![]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let mut boundary = defaults();
    boundary["teams"] = json!((0..256).map(|i| format!("opaque-{i}")).collect::<Vec<_>>());
    boundary["players"] = json!(["é".repeat(64)]);
    let mut bytes = serde_json::to_vec(&boundary).unwrap();
    bytes.resize(60000, b' ');
    let result = call(&router, "PUT", PREFS, Some(&token), &["\"1\""], bytes).await;
    assert_eq!(result.0, StatusCode::OK);
    assert_eq!(result.1[header::ETAG], "\"2\"");
    let timers = call(&router, "GET", "/LiveTv/Timers", Some(&token), &[], vec![]).await;
    assert_eq!(timers.0, StatusCode::OK);
    assert_eq!(timers.2["Items"], json!([]));
    retire(wired).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn database_cas_restart_corruption_and_rollback() {
    let (_temp, config, db, wired, router, token, uid, _other) = setup().await;
    let db2 = ferrofin_db::Database::connect(&config.database_url())
        .await
        .unwrap();
    let second = wire(&db2, &config).await;
    let router2 = ferrofin_api::create_router(second.state.clone());
    let (a, b) = tokio::join!(
        json_call(&router, "PUT", PREFS, Some(&token), &["\"0\""], defaults()),
        json_call(&router2, "PUT", PREFS, Some(&token), &["\"0\""], defaults())
    );
    let mut statuses = [a.0.as_u16(), b.0.as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 412]);
    retire(wired).await;
    retire(second).await;
    let restarted = wire(&db2, &config).await;
    let router = ferrofin_api::create_router(restarted.state.clone());
    assert_eq!(
        call(&router, "GET", PREFS, Some(&token), &[], vec![])
            .await
            .2["version"],
        1
    );
    let user = Uuid::parse_str(&uid).unwrap();
    let before: String = sqlx::query_scalar(
        "SELECT Value FROM CustomItemDisplayPreferences WHERE UserId=? AND Client=?",
    )
    .bind(guid_to_db(user))
    .bind(CLIENT)
    .fetch_one(db.pool())
    .await
    .unwrap();
    sqlx::query("CREATE TRIGGER sports_fail BEFORE INSERT ON CustomItemDisplayPreferences WHEN NEW.Client='bluefin-sports-v1' BEGIN SELECT RAISE(ABORT,'synthetic failure'); END").execute(db.writer()).await.unwrap();
    assert_eq!(
        json_call(&router, "PUT", PREFS, Some(&token), &["\"1\""], defaults())
            .await
            .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let after: String = sqlx::query_scalar(
        "SELECT Value FROM CustomItemDisplayPreferences WHERE UserId=? AND Client=?",
    )
    .bind(guid_to_db(user))
    .bind(CLIENT)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(before, after);
    sqlx::query("DROP TRIGGER sports_fail")
        .execute(db.writer())
        .await
        .unwrap();
    let mut invalid_counts = defaults();
    invalid_counts["teams"] = json!(vec!["opaque"; 257]);
    for value in [
        "broken".to_owned(),
        json!({"version":1,"preferences":invalid_counts}).to_string(),
        json!({"version":u64::MAX,"preferences":defaults()}).to_string(),
    ] {
        sqlx::query("UPDATE CustomItemDisplayPreferences SET Value=? WHERE UserId=? AND Client=?")
            .bind(&value)
            .bind(guid_to_db(user))
            .bind(CLIENT)
            .execute(db.writer())
            .await
            .unwrap();
        assert_eq!(
            json_call(
                &router,
                "PUT",
                PREFS,
                Some(&token),
                &["\"18446744073709551615\""],
                defaults()
            )
            .await
            .0,
            StatusCode::INTERNAL_SERVER_ERROR
        );
        let after: String = sqlx::query_scalar(
            "SELECT Value FROM CustomItemDisplayPreferences WHERE UserId=? AND Client=?",
        )
        .bind(guid_to_db(user))
        .bind(CLIENT)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(value, after);
    }
    retire(restarted).await;
}

// Production case middleware copied for the included base_url module tests.
async fn canonicalize_path_case(
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    // Path: re-case to the registered route where recognized (asset/unknown paths
    // return `None` and keep their significant case). Query keys are left as
    // sent: `ferrofin_api::extract::Query` binds them to each handler's members
    // ignoring case, as ASP.NET does. Preserve the original query for
    // authentication, streaming URLs and plugin forwarding.
    // Jellyfin's `/health` is an ASP.NET route too, so its case is not
    // significant either, unlike Ferrofin's own `/health/live` and `/health/ready`.
    let path = request.uri().path();
    let canonical = if path.eq_ignore_ascii_case("/health") || path.eq_ignore_ascii_case("/health/")
    {
        Some(path.to_ascii_lowercase())
    } else {
        ferrofin_api::routes::canonicalize_path(path)
    };
    if let Some(new_path) = canonical
        && new_path != path
    {
        let target = match request.uri().query() {
            Some(q) => format!("{new_path}?{q}"),
            None => new_path,
        };
        // Origin-form request URI (path + query, no scheme/authority), so
        // rebuilding it from the parts is lossless.
        if let Ok(uri) = target.parse() {
            *request.uri_mut() = uri;
        }
    }
    next.run(request).await
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Guide creation and revocation are one composed scenario.
async fn guide_uses_actual_viewer_visibility_and_truthful_states() {
    use ferrofin_db::entities::base_items::BaseItemEntity;
    use ferrofin_traits::persistence::ItemPersistenceService as _;
    let (_temp, _config, db, wired, router, token, uid, _other) = setup().await;
    let (admin, _) = login(&router, "admin").await;
    let repository = ferrofin_core::FerrofinItemPersistenceService::new(db.clone());
    let channel = Uuid::new_v4();
    let now = chrono::Utc::now();
    sqlx::query("INSERT INTO FerrofinLiveTvChannels (Id,TunerHostId,Name,StreamUrl) VALUES (?,'synthetic','Generic channel','https://example.invalid/never-fetched')").bind(guid_to_db(channel)).execute(db.writer()).await.unwrap();
    let mut channel_item = BaseItemEntity {
        id: guid_to_db(channel),
        type_: "MediaBrowser.Controller.LiveTv.LiveTvChannel".into(),
        name: Some("Generic channel".into()),
        ..Default::default()
    };
    repository
        .save_items(&[channel_item.clone()])
        .await
        .unwrap();
    let mut programs = vec![];
    for (index, (offset, end, sport)) in [
        (-1, Some(1), true),
        (2, Some(3), true),
        (3, None, true),
        (-1, Some(1), false),
    ]
    .into_iter()
    .enumerate()
    {
        let id = Uuid::new_v4();
        let start = now + chrono::Duration::hours(offset);
        let finish = end.map(|h| now + chrono::Duration::hours(h));
        sqlx::query("INSERT INTO FerrofinLiveTvPrograms (Id,ChannelId,StartDate,EndDate,Title,IsSports) VALUES (?,?,?,?,?,?)")
            .bind(guid_to_db(id)).bind(guid_to_db(channel)).bind(ferrofin_db::store::datetime_to_db(start)).bind(finish.map(ferrofin_db::store::datetime_to_db)).bind(format!("Generic listing {index}")).bind(sport).execute(db.writer()).await.unwrap();
        programs.push(BaseItemEntity {
            id: guid_to_db(id),
            type_: "MediaBrowser.Controller.LiveTv.LiveTvProgram".into(),
            name: Some(format!("Generic listing {index}")),
            channel_id: Some(guid_to_db(channel)),
            parent_id: Some(guid_to_db(channel)),
            start_date: Some(start),
            end_date: finish,
            ..Default::default()
        });
    }
    repository.save_items(&programs).await.unwrap();
    let (status, _, feed) = call(&router, "GET", HOME, Some(&token), &[], vec![]).await;
    assert_eq!(status, StatusCode::OK, "{feed}");
    let listings = feed["local_broadcasts"].as_array().unwrap();
    assert_eq!(listings.len(), 3, "{feed}");
    assert_eq!(
        listings
            .iter()
            .map(|l| l["kind"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["ON_NOW", "UPCOMING", "UNKNOWN"]
    );
    assert!(listings.iter().all(|l| l["playback_verified"] == false));
    assert_eq!(feed["games"], json!([]));
    assert_eq!(feed["providers"], json!([]));
    let mut policy = call(
        &router,
        "GET",
        &format!("/Users/{uid}"),
        Some(&admin),
        &[],
        vec![],
    )
    .await
    .2["Policy"]
        .clone();
    policy["BlockedTags"] = json!(["hidden"]);
    assert_eq!(
        json_call(
            &router,
            "POST",
            &format!("/Users/{uid}/Policy"),
            Some(&admin),
            &[],
            policy.clone()
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    programs[0].tags = Some("hidden".into());
    repository.save_items(&[programs[0].clone()]).await.unwrap();
    assert_eq!(
        call(&router, "GET", HOME, Some(&token), &[], vec![])
            .await
            .2["local_broadcasts"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    channel_item.tags = Some("hidden".into());
    repository.save_items(&[channel_item]).await.unwrap();
    assert_eq!(
        call(&router, "GET", HOME, Some(&token), &[], vec![])
            .await
            .2["local_broadcasts"],
        json!([])
    );
    policy["EnableLiveTvAccess"] = json!(false);
    assert_eq!(
        json_call(
            &router,
            "POST",
            &format!("/Users/{uid}/Policy"),
            Some(&admin),
            &[],
            policy
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&router, "GET", HOME, Some(&token), &[], vec![])
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &router,
            "GET",
            &format!("/LiveTv/Channels/{channel}"),
            Some(&token),
            &[],
            vec![]
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    retire(wired).await;
}

/// Gate only the existing manager seam; the actual write remains real SQLite.
struct WriteGate {
    inner: std::sync::Arc<dyn ferrofin_traits::configuration::DisplayPreferencesManager>,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    finished: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl ferrofin_traits::configuration::DisplayPreferencesManager for WriteGate {
    async fn get_display_preferences(
        &self,
        user_id: Uuid,
        item_id: Uuid,
        client: &str,
    ) -> Result<DisplayPreferencesEntity, ServiceError> {
        self.inner
            .get_display_preferences(user_id, item_id, client)
            .await
    }
    async fn get_item_display_preferences(
        &self,
        user_id: Uuid,
        item_id: Uuid,
        client: &str,
    ) -> Result<ItemDisplayPreferencesEntity, ServiceError> {
        self.inner
            .get_item_display_preferences(user_id, item_id, client)
            .await
    }
    async fn list_item_display_preferences(
        &self,
        user_id: Uuid,
        client: &str,
    ) -> Result<Vec<ItemDisplayPreferencesEntity>, ServiceError> {
        self.inner
            .list_item_display_preferences(user_id, client)
            .await
    }
    async fn list_custom_item_display_preferences(
        &self,
        user_id: Uuid,
        item_id: Uuid,
        client: &str,
    ) -> Result<HashMap<String, Option<String>>, ServiceError> {
        self.inner
            .list_custom_item_display_preferences(user_id, item_id, client)
            .await
    }
    async fn set_custom_item_display_preferences(
        &self,
        user_id: Uuid,
        item_id: Uuid,
        client: &str,
        custom_preferences: &HashMap<String, Option<String>>,
    ) -> Result<(), ServiceError> {
        self.inner
            .set_custom_item_display_preferences(user_id, item_id, client, custom_preferences)
            .await
    }
    async fn update_display_preferences(
        &self,
        display_preferences: &DisplayPreferencesEntity,
    ) -> Result<(), ServiceError> {
        self.inner
            .update_display_preferences(display_preferences)
            .await
    }
    async fn update_item_display_preferences(
        &self,
        item_display_preferences: &ItemDisplayPreferencesEntity,
    ) -> Result<(), ServiceError> {
        self.inner
            .update_item_display_preferences(item_display_preferences)
            .await
    }
    async fn list_home_sections(
        &self,
        display_preferences_id: i64,
    ) -> Result<Vec<HomeSectionEntity>, ServiceError> {
        self.inner.list_home_sections(display_preferences_id).await
    }
    async fn set_home_sections(
        &self,
        display_preferences_id: i64,
        sections: &[(i32, i32)],
    ) -> Result<(), ServiceError> {
        self.inner
            .set_home_sections(display_preferences_id, sections)
            .await
    }
    async fn compare_exchange_custom_item_display_preferences(
        &self,
        user_id: Uuid,
        item_id: Uuid,
        client: &str,
        expected: &std::collections::HashMap<String, Option<String>>,
        replacement: &std::collections::HashMap<String, Option<String>>,
    ) -> Result<bool, ServiceError> {
        self.entered.notify_one();
        self.release.notified().await;
        let result = self
            .inner
            .compare_exchange_custom_item_display_preferences(
                user_id,
                item_id,
                client,
                expected,
                replacement,
            )
            .await;
        self.finished.notify_one();
        result
    }
}
#[tokio::test]
async fn cancelled_request_retains_write_until_real_commit() {
    let (_temp, _config, _db, wired, _router, token, _uid, _other) = setup().await;
    let gate = std::sync::Arc::new(WriteGate {
        inner: wired.state.display_preferences.clone(),
        entered: tokio::sync::Notify::default(),
        release: tokio::sync::Notify::default(),
        finished: tokio::sync::Notify::default(),
    });
    let mut inner = (*wired.state).clone();
    inner.display_preferences = gate.clone();
    let router = ferrofin_api::create_router(ferrofin_api::state::AppState::from_inner(inner));
    let request_router = router.clone();
    let request_token = token.clone();
    let request = tokio::spawn(async move {
        json_call(
            &request_router,
            "PUT",
            PREFS,
            Some(&request_token),
            &["\"0\""],
            defaults(),
        )
        .await
    });
    gate.entered.notified().await;
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    gate.release.notify_one();
    gate.finished.notified().await;
    assert_eq!(
        call(&router, "GET", PREFS, Some(&token), &[], vec![])
            .await
            .2["version"],
        1
    );
    retire(wired).await;
}

#[tokio::test]
async fn public_provider_routes_use_normal_account_access_and_validate_before_fetching() {
    let (_temp, _config, _db, wired, router, token, uid, _other) = setup().await;
    let (admin, _) = login(&router, "admin").await;
    assert_eq!(
        json_call(
            &router,
            "POST",
            &format!("/Users/{uid}/Policy"),
            Some(&admin),
            &[],
            json!({"EnableLiveTvAccess":false}),
        )
        .await
        .0,
        StatusCode::NO_CONTENT,
    );
    // Invalid input returns locally without a provider request. Live-TV playback
    // permission is unrelated to an authenticated public-data read.
    for path in [
        "/sports/api/v1/scoreboard?league=internal&date=2026-11-27",
        "/sports/api/v1/scoreboard?league=nhl&date=2026-02-30",
        "/sports/api/v1/scoreboard?league=nhl&date=2026-11-27&url=https://internal",
        "/sports/api/v1/events/nhl/not-an-event",
        "/sports/api/v1/standings/internal",
        "/sports/api/v1/logos/not-a-registered-asset",
    ] {
        assert_eq!(
            call(&router, "GET", path, Some(&token), &[], vec![])
                .await
                .0,
            StatusCode::BAD_REQUEST,
            "{path}",
        );
    }
    assert_eq!(
        call(&router, "GET", HOME, Some(&token), &[], vec![])
            .await
            .0,
        StatusCode::FORBIDDEN,
        "local broadcast access retains the existing Live-TV permission",
    );
    retire(wired).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the fixed primary ESPN endpoints; no user media or production server"]
#[allow(clippy::too_many_lines)] // Keep the full HTTP qualification with its disposable fixture.
async fn provider_feeds_and_logos_use_real_authenticated_http() {
    use std::time::Instant;
    let (_temp, _config, _db, wired, router, token, uid, _other) = setup().await;
    let (admin, _) = login(&router, "admin").await;
    assert_eq!(
        json_call(
            &router,
            "POST",
            &format!("/Users/{uid}/Policy"),
            Some(&admin),
            &[],
            json!({"EnableLiveTvAccess":false}),
        )
        .await
        .0,
        StatusCode::NO_CONTENT,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (stop, stopped) = tokio::sync::oneshot::channel();
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
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .unwrap();
    let authorization = format!(
        r#"MediaBrowser Client="Synthetic",Device="Synthetic",Version="1",DeviceId="sports-test",Token="{token}""#
    );
    assert_eq!(
        client
            .get(format!("{base}/sports/api/v1/standings/nhl"))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::UNAUTHORIZED,
    );
    let mut receipts = Vec::new();
    let mut logo = None;
    for league in ["nhl", "nfl", "nba", "mlb", "wnba", "epl", "mls"] {
        let date = if league == "nhl" {
            "2026-11-25"
        } else {
            "2026-11-26"
        };
        for path in [
            format!("/sports/api/v1/scoreboard?league={league}&date={date}"),
            format!("/sports/api/v1/standings/{league}"),
        ] {
            let start = Instant::now();
            let response = client
                .get(format!("{base}{path}"))
                .header(header::AUTHORIZATION, &authorization)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::OK, "{path}");
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            let bytes = response.bytes().await.unwrap();
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["schema"], 1);
            assert_eq!(value["provider"], "ESPN");
            assert_eq!(value["league"], league);
            assert_eq!(value["stale"], false);
            if path.contains("/standings/") {
                assert!(!value["groups"].as_array().unwrap().is_empty());
                if logo.is_none() {
                    logo = value["groups"][0]["entries"][0]["team"]["logo"]
                        .as_str()
                        .map(str::to_owned);
                }
            } else {
                assert!(value["events"].is_array());
            }
            receipts.push(json!({"path":path,"status":200,"bytes":bytes.len(),
                "elapsedMilliseconds":start.elapsed().as_millis(),"fetchedAt":value["fetchedAt"]}));
        }
    }
    for (league, id) in [("nhl", "401892781"), ("nfl", "401873081")] {
        let path = format!("/sports/api/v1/events/{league}/{id}");
        let start = Instant::now();
        let response = client
            .get(format!("{base}{path}"))
            .header(header::AUTHORIZATION, &authorization)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK, "{path}");
        let value: Value = response.json().await.unwrap();
        assert_eq!(value["event"]["id"], id);
        let plays = value["event"]["plays"].as_array().unwrap();
        let stats = value["event"]["stats"].as_array().unwrap();
        assert!(plays.len() > 100, "{path}");
        assert!(!stats.is_empty(), "{path}");
        receipts.push(
            json!({"path":path,"status":200,"plays":plays.len(),"stats":stats.len(),
            "elapsedMilliseconds":start.elapsed().as_millis()}),
        );
    }
    let logo = logo.expect("at least one primary team logo");
    assert!(logo.starts_with("/sports/api/v1/logos/"));
    let image = client
        .get(format!("{base}{logo}"))
        .header(header::AUTHORIZATION, &authorization)
        .send()
        .await
        .unwrap();
    assert_eq!(image.status(), reqwest::StatusCode::OK);
    assert_eq!(image.headers()[header::CONTENT_TYPE], "image/png");
    let bytes = image.bytes().await.unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(bytes.len() <= 1024 * 1024);
    assert_eq!(
        client
            .get(format!("{base}{logo}"))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "a cached public logo still needs an authenticated app account",
    );
    receipts.push(json!({"path":logo,"status":200,"bytes":bytes.len()}));
    if let Some(root) = std::env::var_os("BLUEFIN_SPORTS_RECEIPT_DIR") {
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            std::path::PathBuf::from(root).join("sports-provider-http-qualification.json"),
            serde_json::to_vec_pretty(&receipts).unwrap(),
        )
        .unwrap();
    }
    stop.send(()).unwrap();
    server.await.unwrap();
    retire(wired).await;
}
