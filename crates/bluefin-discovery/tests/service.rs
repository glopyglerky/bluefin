use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use bluefin_contract::discovery::SubmitAction;
use bluefin_contract::search::SearchScope;
use bluefin_discovery::{Audience, DiscoveryService, Error, Viewer, config::SaveSettings};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use tokio::sync::Mutex;
use uuid::Uuid;

const USER: Uuid = Uuid::from_u128(42);
const KEY: &str = "test-service-credential";
const SESSION: &str = "test-viewer-session";

#[derive(Default)]
struct Fake {
    permissions: AtomicU64,
    auth_calls: AtomicUsize,
    search_calls: AtomicUsize,
    redirect_calls: AtomicUsize,
    redirect: AtomicBool,
    bodies: Mutex<Vec<Value>>,
}
fn scoped(headers: &HeaderMap) -> bool {
    headers.get("x-api-key").is_some_and(|v| v == KEY)
        && headers.get("x-api-user").is_some_and(|v| v == "7")
        && headers.get("authorization").is_none()
}

fn session_scoped(headers: &HeaderMap) -> bool {
    headers
        .get("authorization")
        .is_some_and(|v| v == "MediaBrowser Token=\"test-viewer-session\"")
        && !headers.contains_key("x-api-key")
        && !headers.contains_key("x-api-user")
}
async fn me(State(fake): State<Arc<Fake>>, headers: HeaderMap) -> Response {
    fake.auth_calls.fetch_add(1, Ordering::SeqCst);
    if fake.redirect.load(Ordering::SeqCst) {
        return (StatusCode::FOUND, [("location", "/redirect-target")]).into_response();
    }
    if !scoped(&headers) {
        return Json(json!({"id":1,"permissions":2})).into_response();
    }
    Json(json!({"id":7,"permissions":fake.permissions.load(Ordering::SeqCst),"jellyfinUserId":USER.simple().to_string(),"apiKey":"never-forward-me"})).into_response()
}
async fn search(State(fake): State<Arc<Fake>>, headers: HeaderMap) -> Response {
    assert!(scoped(&headers));
    fake.search_calls.fetch_add(1, Ordering::SeqCst);
    Json(json!({"page":1,"totalPages":2,"results":[
        {"id":123,"mediaType":"movie","title":"Original movie","overview":"Synthetic metadata","releaseDate":"2024-01-01","upstreamUserId":7,"mediaInfo":{"status":1,"requests":[{"userId":7}]}},
        {"id":456,"mediaType":"tv","name":"Original series","firstAirDate":"2025-04-02","mediaInfo":{"status":1}},
        {"id":789,"mediaType":"movie","title":"Available movie","mediaInfo":{"status":5}},
        {"id":11,"mediaType":"person","name":"Not a playable title"}
    ]})).into_response()
}
async fn seasons(headers: HeaderMap) -> Json<Value> {
    assert!(scoped(&headers));
    Json(json!({"seasons":[
        {"seasonNumber":0,"episodeCount":0},{"seasonNumber":1,"episodeCount":8},{"seasonNumber":2,"episodeCount":10}
    ]}))
}
async fn submit(
    State(fake): State<Arc<Fake>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    assert!(scoped(&headers));
    fake.bodies.lock().await.push(body);
    (
        StatusCode::CREATED,
        Json(json!({"id":999,"status":1,"requestedBy":{"id":7}})),
    )
        .into_response()
}
struct Fixture {
    fake: Arc<Fake>,
    service: Arc<DiscoveryService>,
    root: tempfile::TempDir,
    task: tokio::task::JoinHandle<()>,
    url: String,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        let fake = Arc::new(Fake::default());
        fake.permissions.store(32, Ordering::SeqCst);
        let target = Arc::clone(&fake);
        let app = Router::new()
            .route("/api/v1/auth/me", get(me))
            .route("/api/v1/search", get(search))
            .route("/api/v1/tv/{id}", get(seasons))
            .route("/api/v1/request", post(submit))
            .route(
                "/Moonfin/Seerr/Api/auth/me",
                get(|State(fake): State<Arc<Fake>>, headers: HeaderMap| async move {
                    assert!(session_scoped(&headers));
                    fake.auth_calls.fetch_add(1, Ordering::SeqCst);
                    Json(json!({
                        "id": 7, "permissions": fake.permissions.load(Ordering::SeqCst)
                    }))
                }),
            )
            .route(
                "/Moonfin/Seerr/Api/search",
                get(|headers: HeaderMap, Query(query): Query<std::collections::HashMap<String, String>>| async move {
                    assert!(session_scoped(&headers));
                    assert_eq!(query["query"], "stevn spelberg");
                    let mut results = vec![
                        json!({"id": 11, "mediaType": "person", "name": "Steven Spielberg", "knownForDepartment": "Directing"}),
                        json!({"id": 123, "mediaType": "movie", "title": "A related film", "mediaInfo": null}),
                        json!({"id": 456, "mediaType": "tv", "name": "A related series"})
                    ];
                    if let Some(kind) = query.get("type") {
                        results.retain(|row| row["mediaType"] == kind.as_str());
                    }
                    Json(json!({"results": results}))
                }),
            )
            .route(
                "/JellyfinEnhanced/jellyseerr/music/search",
                get(|headers: HeaderMap, Query(query): Query<std::collections::HashMap<String, String>>| async move {
                    assert!(session_scoped(&headers));
                    assert_eq!(query["query"], "Test musician");
                    Json(json!({
                        "artists": [{"name": "Test musician"}],
                        "albums": [{"albumName":"Test album","artistName":"Test musician","catalogManaged":true}],
                        "tracks": [{"title":"Test track","artistName":"Test musician"}]
                    }))
                }),
            )
            .route(
                "/Moonfin/Seerr/Api/request",
                post(|State(fake): State<Arc<Fake>>, headers: HeaderMap, Json(body): Json<Value>| async move {
                    assert!(session_scoped(&headers));
                    fake.bodies.lock().await.push(body);
                    Json(json!({"familyLibrary": {"state": "ready"}}))
                }),
            )
            .route(
                "/redirect-target",
                get(move || {
                    let target = Arc::clone(&target);
                    async move {
                        target.redirect_calls.fetch_add(1, Ordering::SeqCst);
                        StatusCode::OK
                    }
                }),
            )
            .with_state(Arc::clone(&fake));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let root = tempfile::tempdir().unwrap();
        let service = DiscoveryService::open(&root.path().join("discovery.json")).unwrap();
        Self {
            fake,
            service,
            root,
            task,
            url,
        }
    }
    fn update(
        &self,
        revision: u64,
        enabled: bool,
        clients: bool,
        credential: bool,
    ) -> SaveSettings {
        serde_json::from_value(json!({"expectedRevision":revision,"enabled":enabled,"exposeToClients":clients,
            "baseUrl":self.url,"allowLocalHttp":true,"apiKey":if credential { Some(KEY) } else { None },
            "viewers":[{"userId":USER,"upstreamUserId":7,"search":true,"requests":true}]})).unwrap()
    }
    async fn enable(&self) {
        self.service
            .save(self.update(0, true, true, true))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn session_catalog_scopes_are_sent_upstream_and_music_uses_the_current_viewer() {
    let f = Fixture::new().await;
    let mut update = f.update(0, true, true, false);
    update.session_catalog = Some(true);
    f.service.save(update).await.unwrap();
    let current = viewer().with_session_token(SESSION);
    for (scope, kind) in [
        (SearchScope::Movies, "movie"),
        (SearchScope::TvShows, "series"),
    ] {
        let page = f
            .service
            .search_scope(
                current.clone(),
                Audience::Client,
                "stevn spelberg",
                1,
                scope,
            )
            .await
            .unwrap();
        assert_eq!(page.results.len(), 1);
        assert_eq!(page.results[0].kind, kind);
        assert_eq!(page.results[0].actions.len(), 1);
    }
    let page = f
        .service
        .search_scope(
            current,
            Audience::Client,
            "Test musician",
            1,
            SearchScope::Music,
        )
        .await
        .unwrap();
    assert_eq!(
        page.results
            .iter()
            .map(|row| row.kind.as_str())
            .collect::<Vec<_>>(),
        ["artist", "album", "track"]
    );
    assert!(
        page.results
            .iter()
            .all(|row| row.availability == "unknown" && row.actions.is_empty())
    );
}

#[tokio::test]
async fn session_catalog_preserves_ranked_people_and_submits_through_current_family_session() {
    let f = Fixture::new().await;
    let mut update = f.update(0, true, true, false);
    update.session_catalog = Some(true);
    update.base_url = Some("https://catalog.example.invalid".into());
    assert!(matches!(f.service.save(update).await, Err(Error::Invalid)));

    let mut update = f.update(0, true, true, false);
    update.session_catalog = Some(true);
    f.service.save(update).await.unwrap();
    let service = DiscoveryService::open(&f.root.path().join("discovery.json")).unwrap();
    assert!(service.settings().await.session_catalog);
    assert!(service.capabilities(viewer(), Audience::Client).await.is_none());
    assert!(
        service
            .capabilities(viewer().with_session_token(SESSION), Audience::Client)
            .await
            .is_some()
    );
    assert!(matches!(
        service
            .search(viewer(), Audience::Client, "stevn spelberg", 1)
            .await,
        Err(Error::Forbidden)
    ));
    assert_eq!(f.fake.auth_calls.load(Ordering::SeqCst), 0);

    let viewer = viewer().with_session_token(SESSION);
    let page = service
        .search(viewer.clone(), Audience::Client, "stevn spelberg", 1)
        .await
        .unwrap();
    assert_eq!(page.results[0].kind, "person");
    assert_eq!(page.results[0].title, "Steven Spielberg");
    assert!(page.results[0].actions.is_empty());
    let action = page.results[1].actions[0].id;
    service
        .action(viewer.clone(), Audience::Client, action)
        .await
        .unwrap();
    service
        .submit(viewer, Audience::Client, input(action, &[]))
        .await
        .unwrap();
    assert_eq!(
        *f.fake.bodies.lock().await,
        vec![json!({
            "mediaId": 123, "mediaType": "movie", "is4k": false
        })]
    );
}
fn viewer() -> Viewer {
    Viewer::new(USER, false)
}
fn input(id: Uuid, selections: &[&str]) -> SubmitAction {
    SubmitAction {
        action_id: id,
        selections: selections.iter().map(|s| (*s).into()).collect(),
        authorized_use_confirmed: true,
    }
}

#[tokio::test]
async fn default_off_and_web_opt_in_do_not_contact_upstream_for_denied_clients() {
    let f = Fixture::new().await;
    assert!(
        f.service
            .capabilities(viewer(), Audience::Client)
            .await
            .is_none()
    );
    assert!(matches!(
        f.service
            .search(viewer(), Audience::Client, "film", 1)
            .await,
        Err(Error::Unavailable)
    ));
    f.service
        .save(f.update(0, true, false, true))
        .await
        .unwrap();
    assert!(
        f.service
            .capabilities(viewer(), Audience::Client)
            .await
            .is_none()
    );
    assert!(
        f.service
            .capabilities(viewer(), Audience::Web)
            .await
            .unwrap()
            .search
    );
    assert!(matches!(
        f.service
            .search(viewer(), Audience::Client, "film", 1)
            .await,
        Err(Error::Unavailable)
    ));
    assert_eq!(f.fake.auth_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.service
            .search(viewer(), Audience::Web, "film", 1)
            .await
            .unwrap()
            .results
            .len(),
        3
    );
}

#[tokio::test]
async fn mapped_identity_generic_results_explicit_request_and_duplicate_submission() {
    let f = Fixture::new().await;
    f.enable().await;
    let page = f
        .service
        .search(viewer(), Audience::Client, "original", 1)
        .await
        .unwrap();
    let raw = serde_json::to_string(&page).unwrap();
    for secret in [
        KEY,
        "never-forward-me",
        "upstreamUserId",
        "requestedBy",
        "apiKey",
    ] {
        assert!(!raw.contains(secret));
    }
    assert_eq!(page.results.len(), 3);
    assert!(page.results[2].actions.is_empty());
    let action = page.results[0].actions[0].id;
    let form = f
        .service
        .action(viewer(), Audience::Client, action)
        .await
        .unwrap();
    assert!(!form.requires_selection);
    let mut unconfirmed = input(action, &[]);
    unconfirmed.authorized_use_confirmed = false;
    assert!(matches!(
        f.service
            .submit(viewer(), Audience::Client, unconfirmed)
            .await,
        Err(Error::Invalid)
    ));
    assert_eq!(
        f.service
            .submit(viewer(), Audience::Client, input(action, &[]))
            .await
            .unwrap()
            .status,
        "submitted"
    );
    assert_eq!(
        f.service
            .submit(viewer(), Audience::Client, input(action, &[]))
            .await
            .unwrap()
            .status,
        "submitted"
    );
    let bodies = f.fake.bodies.lock().await;
    assert_eq!(bodies.len(), 1);
    assert_eq!(
        bodies[0],
        json!({"mediaType":"movie","mediaId":123,"is4k":false})
    );
}

#[tokio::test]
async fn caller_identity_and_service_key_never_grant_another_users_authority() {
    let f = Fixture::new().await;
    f.enable().await;
    let stranger = Viewer::new(Uuid::from_u128(43), false);
    assert!(matches!(
        f.service
            .search(stranger.clone(), Audience::Client, "original", 1)
            .await,
        Err(Error::Forbidden)
    ));
    assert_eq!(f.fake.auth_calls.load(Ordering::SeqCst), 0);
    let page = f
        .service
        .search(viewer(), Audience::Client, "original", 1)
        .await
        .unwrap();
    let action = page.results[0].actions[0].id;
    assert!(
        f.service
            .action(stranger, Audience::Client, action)
            .await
            .is_err()
    );
    f.fake.permissions.store(0, Ordering::SeqCst);
    assert!(matches!(
        f.service
            .submit(viewer(), Audience::Client, input(action, &[]))
            .await,
        Err(Error::Forbidden)
    ));
    assert!(f.fake.bodies.lock().await.is_empty());
    f.fake.permissions.store(2, Ordering::SeqCst);
    assert!(matches!(
        f.service
            .search(viewer(), Audience::Client, "original", 1)
            .await,
        Err(Error::Forbidden)
    ));
}

#[tokio::test]
async fn disable_invalidates_stale_actions_and_settings_survive_restart_without_secret_readback() {
    let f = Fixture::new().await;
    f.enable().await;
    let page = f
        .service
        .search(viewer(), Audience::Client, "original", 1)
        .await
        .unwrap();
    let action = page.results[0].actions[0].id;
    assert!(matches!(
        f.service.save(f.update(0, true, true, false)).await,
        Err(Error::Conflict)
    ));
    f.service
        .save(f.update(1, true, false, false))
        .await
        .unwrap();
    assert!(matches!(
        f.service
            .submit(viewer(), Audience::Client, input(action, &[]))
            .await,
        Err(Error::Unavailable)
    ));
    let restarted = DiscoveryService::open(&f.root.path().join("discovery.json")).unwrap();
    let snapshot = restarted.settings().await;
    assert_eq!(snapshot.revision, 2);
    assert!(snapshot.credential_configured);
    assert!(!serde_json::to_string(&snapshot).unwrap().contains(KEY));
    assert!(
        restarted
            .capabilities(viewer(), Audience::Client)
            .await
            .is_none()
    );
    assert!(f.fake.bodies.lock().await.is_empty());
}

#[tokio::test]
async fn series_selections_are_server_derived_and_upstream_advanced_fields_are_rejected() {
    let f = Fixture::new().await;
    f.enable().await;
    let page = f
        .service
        .search(viewer(), Audience::Client, "original", 1)
        .await
        .unwrap();
    let action = page.results[1].actions[0].id;
    assert!(matches!(
        f.service
            .submit(viewer(), Audience::Client, input(action, &["1"]))
            .await,
        Err(Error::Invalid)
    ));
    let form = f
        .service
        .action(viewer(), Audience::Client, action)
        .await
        .unwrap();
    assert_eq!(form.choices.len(), 2);
    assert!(matches!(
        f.service
            .submit(viewer(), Audience::Client, input(action, &["99"]))
            .await,
        Err(Error::Invalid)
    ));
    assert!(serde_json::from_value::<SubmitAction>(json!({"actionId":action,"selections":[],"authorizedUseConfirmed":true,"userId":1,"ignoreQuota":true})).is_err());
    f.service
        .submit(viewer(), Audience::Client, input(action, &["2", "1"]))
        .await
        .unwrap();
    assert_eq!(
        f.fake.bodies.lock().await[0],
        json!({"mediaType":"tv","mediaId":456,"is4k":false,"seasons":[1,2]})
    );
}

#[tokio::test]
async fn redirects_and_untrusted_connection_inputs_are_rejected() {
    let f = Fixture::new().await;
    f.enable().await;
    f.fake.redirect.store(true, Ordering::SeqCst);
    assert!(matches!(
        f.service
            .search(viewer(), Audience::Client, "original", 1)
            .await,
        Err(Error::Upstream)
    ));
    assert_eq!(f.fake.redirect_calls.load(Ordering::SeqCst), 0);
    for bad in [
        "http://example.com",
        "http://169.254.169.254",
        "https://user:password@example.com",
        "https://example.com?token=x",
        "https://example.com/a%2fb",
    ] {
        let mut update = f.update(1, true, true, false);
        update.base_url = Some(bad.into());
        assert!(
            matches!(f.service.save(update).await, Err(Error::Invalid)),
            "{bad}"
        );
    }
}
