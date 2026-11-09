//! Bluefin router tests with the production persistent preferences manager.
use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use ferrofin_api::{create_router, state::AppState, test_support::*};
use ferrofin_db::entities::{
    base_items::{BaseItemEntity, PeopleEntity},
    users::UserEntity,
};
use ferrofin_model::{
    data::CollectionType,
    dto::ItemCounts,
    querying::{QueryFiltersLegacy, QueryResult},
    users::UserPolicy,
};
use ferrofin_traits::{
    error::ServiceError,
    library::LibraryManager,
    net::{AuthService, AuthorizationContext, RequestContext},
    options::{AuthorizationInfo, DeleteOptions, InternalItemsQuery, InternalPeopleQuery},
    persistence::ItemWithCounts,
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tower::ServiceExt;
use uuid::Uuid;
const CALLER_ID: Uuid = Uuid::from_u128(42);
const TRACK: Uuid = Uuid::from_u128(70);
struct UserAuth(Uuid, bool);
impl UserAuth {
    fn info(&self) -> AuthorizationInfo {
        AuthorizationInfo {
            is_authenticated: !self.0.is_nil(),
            is_api_key: self.1,
            user: (!self.0.is_nil()).then(|| Arc::new(fake_user_entity(self.0, "viewer"))),
            token: Some("token".into()),
            ..Default::default()
        }
    }
}
#[async_trait]
impl AuthService for UserAuth {
    async fn authenticate(&self, _: &RequestContext) -> Result<AuthorizationInfo, ServiceError> {
        if self.0.is_nil() {
            Err(ServiceError::Unauthorized("missing".into()))
        } else {
            Ok(self.info())
        }
    }
}
#[async_trait]
impl AuthorizationContext for UserAuth {
    async fn get_authorization_info(
        &self,
        _: &RequestContext,
    ) -> Result<AuthorizationInfo, ServiceError> {
        Ok(self.info())
    }
}
macro_rules! fake_library_manager {
    ($fake:ty { $($given:tt)* }) => {
        #[async_trait]
        impl LibraryManager for $fake {
            $($given)*

            async fn get_item_by_id(
                &self,
                _id: Uuid,
            ) -> Result<Option<BaseItemEntity>, ServiceError> {
                unimplemented!("fake")
            }
            async fn query_items(
                &self,
                _query: &InternalItemsQuery,
            ) -> Result<QueryResult<BaseItemEntity>, ServiceError> {
                unimplemented!("fake")
            }
            async fn get_item_ids(
                &self,
                _query: &InternalItemsQuery,
            ) -> Result<Vec<Uuid>, ServiceError> {
                unimplemented!("fake")
            }
            async fn get_item_list(
                &self,
                _query: &InternalItemsQuery,
            ) -> Result<Vec<BaseItemEntity>, ServiceError> {
                unimplemented!("fake")
            }
            async fn get_latest_item_list(
                &self,
                _query: &InternalItemsQuery,
                _collection_type: CollectionType,
            ) -> Result<Vec<BaseItemEntity>, ServiceError> {
                unimplemented!("fake")
            }
            async fn create_items(
                &self,
                _items: &[BaseItemEntity],
                _parent_id: Option<Uuid>,
            ) -> Result<(), ServiceError> {
                unimplemented!("fake")
            }
            async fn update_items(
                &self,
                _items: &[BaseItemEntity],
                _parent_id: Option<Uuid>,
            ) -> Result<(), ServiceError> {
                unimplemented!("fake")
            }
            async fn delete_item(
                &self,
                _id: Uuid,
                _options: &DeleteOptions,
            ) -> Result<(), ServiceError> {
                unimplemented!("fake")
            }
            async fn get_people(
                &self,
                _query: &InternalPeopleQuery,
            ) -> Result<Vec<PeopleEntity>, ServiceError> {
                unimplemented!("fake")
            }
            async fn get_people_names(
                &self,
                _query: &InternalPeopleQuery,
            ) -> Result<Vec<String>, ServiceError> {
                unimplemented!("fake")
            }
            async fn get_count(&self, _query: &InternalItemsQuery) -> Result<i32, ServiceError> {
                unimplemented!("fake")
            }
            async fn get_item_counts(
                &self,
                _query: &InternalItemsQuery,
            ) -> Result<ItemCounts, ServiceError> {
                unimplemented!("fake")
            }
            async fn get_genres(
                &self,
                _query: &InternalItemsQuery,
            ) -> Result<QueryResult<ItemWithCounts>, ServiceError> {
                unimplemented!("fake")
            }
            async fn get_studios(
                &self,
                _query: &InternalItemsQuery,
            ) -> Result<QueryResult<ItemWithCounts>, ServiceError> {
                unimplemented!("fake")
            }
            async fn get_artists(
                &self,
                _query: &InternalItemsQuery,
            ) -> Result<QueryResult<ItemWithCounts>, ServiceError> {
                unimplemented!("fake")
            }
            async fn get_music_genres(
                &self,
                _query: &InternalItemsQuery,
            ) -> Result<QueryResult<ItemWithCounts>, ServiceError> {
                unimplemented!("fake")
            }
            async fn get_album_artists(
                &self,
                _query: &InternalItemsQuery,
            ) -> Result<QueryResult<ItemWithCounts>, ServiceError> {
                unimplemented!("fake")
            }
            async fn get_query_filters_legacy(
                &self,
                _query: &InternalItemsQuery,
            ) -> Result<QueryFiltersLegacy, ServiceError> {
                unimplemented!("fake")
            }
            async fn get_media_stream_languages(
                &self,
                _stream_type: ferrofin_model::entities::MediaStreamType,
                _query: &InternalItemsQuery,
            ) -> Result<Vec<String>, ServiceError> {
                unimplemented!("fake")
            }
        }
    };
}

struct VisibleLibrary(AtomicBool);
fake_library_manager!(VisibleLibrary {
 async fn queue_library_scan(&self) -> Result<(),ServiceError> { Ok(()) }
 async fn get_item_by_id_for_user(&self, id: Uuid, user: Option<&UserEntity>) -> Result<Option<BaseItemEntity>,ServiceError> {
    assert!(user.is_some(), "queue never uses unrestricted lookup");
    Ok(if id == TRACK && self.0.load(Ordering::SeqCst) { Some(minimal_base_item(id,"track","Audio")) }
    else if id == Uuid::from_u128(71) { Some(minimal_base_item(id,"movie","Movie")) } else { None })
 }
});
async fn database(path: &std::path::Path) -> ferrofin_db::Database {
    let db = ferrofin_db::Database::connect_sized(
        &format!("sqlite://{}?mode=rwc", path.display()),
        Some(2),
    )
    .await
    .unwrap();
    db.run_migrations().await.unwrap();
    for id in [CALLER_ID, Uuid::from_u128(43)] {
        sqlx::query(r#"INSERT OR IGNORE INTO "Users" ("Id", "AuthenticationProviderId", "DisplayCollectionsView", "DisplayMissingEpisodes", "EnableAutoLogin", "EnableLocalPassword", "EnableNextEpisodeAutoPlay", "EnableUserPreferenceAccess", "HidePlayedInLatest", "InternalId", "InvalidLoginAttemptCount", "MaxActiveSessions", "MustUpdatePassword", "PasswordResetProviderId", "PlayDefaultAudioTrack", "RememberAudioSelections", "RememberSubtitleSelections", "RowVersion", "SubtitleMode", "SyncPlayAccess", "Username", "NormalizedUsername") VALUES (?1,'',0,0,0,0,0,1,0,0,0,0,0,'',1,1,1,0,0,0,?1,?1)"#).bind(ferrofin_db::store::guid_to_db(id)).execute(db.writer()).await.unwrap();
    }
    db
}
async fn request(state: AppState, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = create_router(state).oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 1_000_000).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
fn update(revision: u64, ids: &[Uuid]) -> Value {
    json!({"protocolVersion":1,"expectedRevision":revision,"queue":{"itemIds":ids,"cursor":if ids.is_empty() { None } else { Some(0usize) },"shuffle":true,"repeat":"all"}})
}
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn router_auth_validation_concurrency_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("queue.db");
    let db = database(&path).await;
    let library = Arc::new(VisibleLibrary(AtomicBool::new(true)));
    let app = state(db.clone(), CALLER_ID, false, library.clone());
    for uri in ["/Bluefin/Capabilities", "/Bluefin/Music/Queue"] {
        assert_eq!(
            request(
                state(db.clone(), Uuid::nil(), false, library.clone()),
                "GET",
                uri,
                Value::Null
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(
                state(db.clone(), CALLER_ID, true, library.clone()),
                "GET",
                uri,
                Value::Null
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    let (s, caps) = request(app.clone(), "GET", "/Bluefin/Capabilities", Value::Null).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(caps["viewer"]["contentDownloading"], false);
    assert!(caps.get("iroh").is_none());
    assert_eq!(
        request(
            app.clone(),
            "PUT",
            "/Bluefin/Music/Queue",
            update(0, &[Uuid::from_u128(999)])
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            app.clone(),
            "PUT",
            "/Bluefin/Music/Queue",
            update(0, &[Uuid::from_u128(71)])
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    for method in ["GET", "PUT"] {
        assert_eq!(
            request(
                app.clone(),
                method,
                "/Bluefin/Music/Queue?userId=00000000-0000-0000-0000-000000000043",
                update(0, &[])
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let mut invalid = update(0, &[]);
    invalid["queue"]["cursor"] = json!(0);
    assert_eq!(
        request(app.clone(), "PUT", "/Bluefin/Music/Queue", invalid)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let mut invalid = update(0, &[]);
    invalid["queue"]["profilePolicy"] = json!({"admin":true});
    assert_eq!(
        request(app.clone(), "PUT", "/Bluefin/Music/Queue", invalid)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut oversized = update(0, &[]);
    oversized["padding"] = json!("x".repeat(70000));
    assert_eq!(
        request(app.clone(), "PUT", "/Bluefin/Music/Queue", oversized)
            .await
            .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let (a, b) = tokio::join!(
        request(
            app.clone(),
            "PUT",
            "/Bluefin/Music/Queue",
            update(0, &[TRACK])
        ),
        request(
            app.clone(),
            "PUT",
            "/Bluefin/Music/Queue",
            update(0, &[TRACK])
        )
    );
    assert!([a.0, b.0].contains(&StatusCode::OK));
    assert!([a.0, b.0].contains(&StatusCode::CONFLICT));
    assert_eq!(
        request(
            state(db.clone(), Uuid::from_u128(43), false, library.clone()),
            "GET",
            "/Bluefin/Music/Queue",
            Value::Null
        )
        .await
        .1["revision"],
        0
    );
    assert_eq!(
        request(
            app.clone(),
            "GET",
            "/DisplayPreferences/queue?client=bluefin-music-queue-v1",
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            app.clone(),
            "POST",
            "/DisplayPreferences/queue?client=bluefin-music-queue-v1",
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            app.clone(),
            "PUT",
            "/Bluefin/Music/Queue",
            update(1, &[TRACK; 513])
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    library.0.store(false, Ordering::SeqCst);
    assert_eq!(
        request(app.clone(), "GET", "/Bluefin/Music/Queue", Value::Null)
            .await
            .1["queue"]["itemIds"],
        json!([])
    );
    assert_eq!(
        request(
            app.clone(),
            "PUT",
            "/Bluefin/Music/Queue",
            update(1, &[TRACK])
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    drop(app);
    db.close().await;
    drop(db);
    let db = database(&path).await;
    library.0.store(true, Ordering::SeqCst);
    let app = state(db.clone(), CALLER_ID, false, library);
    let (_, saved) = request(app, "GET", "/Bluefin/Music/Queue", Value::Null).await;
    assert_eq!(saved["revision"], 1);
    assert_eq!(saved["queue"]["cursor"], 0);
    assert_eq!(saved["queue"]["shuffle"], true);
    assert_eq!(saved["queue"]["repeat"], "all");
    assert_eq!(saved["queue"]["itemIds"], json!([TRACK]));
    db.close().await;
}
fn state(
    db: ferrofin_db::Database,
    id: Uuid,
    api_key: bool,
    library: Arc<VisibleLibrary>,
) -> AppState {
    let auth = Arc::new(UserAuth(id, api_key));
    let users = Arc::new(PolicyUsers(UserPolicy {
        enable_content_downloading: false,
        ..UserPolicy::default()
    }));
    AppState::new(
        library,
        users,
        Arc::new(FakeUserViews),
        Arc::new(FakeUserData),
        Arc::new(FakeMediaSources),
        Arc::new(FakeSessions),
        Arc::new(FakeSystem),
        Arc::new(FakeAppHost),
        Arc::new(FakeConfig),
        Arc::new(FakeProviders),
        Arc::new(FakeMusic),
        Arc::new(FakeSimilarItems),
        Arc::new(FakeSearch),
        Arc::new(FakeDto),
        auth.clone(),
        auth,
        Arc::new(FakeQuickConnect),
        Arc::new(FakePlaylists),
        Arc::new(FakeCollections),
        Arc::new(FakeTvSeries),
        Arc::new(FakeSubtitles),
        Arc::new(FakeLyrics),
        Arc::new(FakeMediaSegments),
        Arc::new(FakeTrickplay),
        Arc::new(FakeDevices),
        Arc::new(FakeClientEventLogger),
        Arc::new(FakeApiKeys),
        Arc::new(FakeLocalization),
        Arc::new(ferrofin_core::FerrofinDisplayPreferencesManager::new(db)),
        Arc::new(FakeActivity),
        Arc::new(FakeFileSystem),
        Arc::new(FakeTasks),
    )
}
