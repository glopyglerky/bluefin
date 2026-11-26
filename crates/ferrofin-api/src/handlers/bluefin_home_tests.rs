use super::repository::{CLOCK_SKEW_MS, RETENTION_MS, RETRY_WINDOW_MS};
use super::*;
use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use ferrofin_core::{
    item_count_service::FerrofinItemCountService,
    item_persistence_service::FerrofinItemPersistenceService,
    item_repository::FerrofinItemRepository, item_type_lookup::ItemTypeLookup,
    library_manager::FerrofinLibraryManager, people_repository::FerrofinPeopleRepository,
    similar_items_manager::FerrofinSimilarItemsManager, user_manager::FerrofinUserManager,
};
use ferrofin_db::Database;
use ferrofin_db::entities::users::UserEntity;
use ferrofin_model::users::UserPolicy;
use ferrofin_traits::error::ServiceError;
use ferrofin_traits::{
    dto::DtoService,
    library::UserManager,
    net::{AuthService, RequestContext},
};
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    db: Database,
    a: Arc<UserEntity>,
    b: Arc<UserEntity>,
}
impl Fixture {
    async fn new() -> Self {
        let db = Database::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        Self::with_db(db).await
    }
    async fn with_db(db: Database) -> Self {
        let users = FerrofinUserManager::new(db.clone());
        let a = Arc::new(users.create_user("home-a").await.unwrap());
        let b = Arc::new(users.create_user("home-b").await.unwrap());
        Self { db, a, b }
    }
    fn history(&self) -> History {
        History::new(self.db.clone())
    }
    fn a_id(&self) -> Uuid {
        Uuid::parse_str(&self.a.id).unwrap()
    }
    fn b_id(&self) -> Uuid {
        Uuid::parse_str(&self.b.id).unwrap()
    }
    fn state(&self, user: Arc<UserEntity>, api_key: bool) -> AppState {
        let mut inner = (*crate::test_support::fake_state()).clone();
        let items = Arc::new(FerrofinItemRepository::new(
            self.db.clone(),
            Arc::new(ItemTypeLookup::new()),
        ));
        inner.library = Arc::new(FerrofinLibraryManager::new(
            items.clone(),
            Arc::new(FerrofinItemCountService::new(self.db.clone())),
            Arc::new(FerrofinItemPersistenceService::new(self.db.clone())),
            Arc::new(FerrofinPeopleRepository::new(self.db.clone())),
        ));
        inner.similar_items = Arc::new(FerrofinSimilarItemsManager::new(self.db.clone(), items));
        inner.users = Arc::new(FerrofinUserManager::new(self.db.clone()));
        inner.dto = Arc::new(TestDto);
        inner.auth_service = Arc::new(StaticAuth { user, api_key });
        AppState::from_inner(inner).with_bluefin_home(self.db.clone())
    }
}
fn event(term: &str, generation: i64, now: i64) -> SearchEvent {
    SearchEvent {
        event_id: Uuid::new_v4(),
        query: term.into(),
        selected_item_id: None,
        occurred_at_unix_ms: now,
        generation,
    }
}
struct StaticAuth {
    user: Arc<UserEntity>,
    api_key: bool,
}
#[async_trait]
impl AuthService for StaticAuth {
    async fn authenticate(&self, _: &RequestContext) -> Result<AuthorizationInfo, ServiceError> {
        Ok(AuthorizationInfo {
            user: Some(self.user.clone()),
            is_authenticated: true,
            is_api_key: self.api_key,
            user_policy: Some(Arc::new(UserPolicy {
                enable_remote_access: true,
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}
struct TestDto;
fn dto(item: &BaseItemEntity) -> BaseItemDto {
    BaseItemDto {
        id: Uuid::parse_str(&item.id).unwrap(),
        name: item.name.clone(),
        ..Default::default()
    }
}
#[async_trait]
impl DtoService for TestDto {
    async fn get_primary_image_aspect_ratio(&self, _: Uuid) -> Result<Option<f64>, ServiceError> {
        Ok(None)
    }
    async fn get_base_item_dto(
        &self,
        item: &BaseItemEntity,
        _: &DtoOptions,
        _: Option<&UserEntity>,
        _: Option<Uuid>,
    ) -> Result<BaseItemDto, ServiceError> {
        Ok(dto(item))
    }
    async fn get_base_item_dtos(
        &self,
        items: &[BaseItemEntity],
        _: &DtoOptions,
        user: Option<&UserEntity>,
        _: Option<Uuid>,
        _: bool,
    ) -> Result<Vec<BaseItemDto>, ServiceError> {
        assert!(
            user.is_some(),
            "Home DTOs must retain their authenticated owner"
        );
        Ok(items.iter().map(dto).collect())
    }
    async fn get_item_by_name_dto(
        &self,
        item: &BaseItemEntity,
        _: &DtoOptions,
        _: Option<&[Uuid]>,
        _: Option<&UserEntity>,
    ) -> Result<BaseItemDto, ServiceError> {
        Ok(dto(item))
    }
}
async fn request(state: AppState, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = register(Router::new())
        .with_state(state)
        .oneshot(req)
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store",
        "Account Home data must not be cached by a shared HTTP intermediary");
    let bytes = to_bytes(response.into_body(), 100_000).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
fn body(event: &SearchEvent) -> Value {
    json!({"eventId":event.event_id, "query":event.query, "selectedItemId":event.selected_item_id,
        "occurredAtUnixMs":event.occurred_at_unix_ms, "generation":event.generation})
}

#[tokio::test]
async fn owner_isolation_idempotent_retry_and_clear_generation() {
    let f = Fixture::new().await;
    let h = f.history();
    let now = Utc::now().timestamp_millis();
    let first = event("  ocean   documentaries  ", 0, now);
    let (a, retry) = tokio::join!(
        h.record(f.a_id(), first.clone(), now),
        h.record(f.a_id(), first.clone(), now)
    );
    assert_eq!(a.unwrap().generation, retry.unwrap().generation);
    let (_, rows) = h.snapshot(f.a_id(), now).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].term, "ocean documentaries");
    assert!(h.snapshot(f.b_id(), now).await.unwrap().1.is_empty());

    let mut changed = first.clone();
    changed.query = "changed payload".into();
    assert!(matches!(
        h.record(f.a_id(), changed, now).await,
        Err(ApiError::Conflict(_))
    ));
    // The same event identifier in another account has no authority over A.
    let mut second = first.clone();
    second.query = "jazz".into();
    h.record(f.b_id(), second, now).await.unwrap();
    assert_eq!(h.snapshot(f.b_id(), now).await.unwrap().1[0].term, "jazz");

    assert_eq!(h.clear(f.a_id(), 0).await.unwrap().generation, 1);
    assert!(matches!(
        h.record(f.a_id(), first, now).await,
        Err(ApiError::Conflict(_))
    ));
    let fresh = event("space", 1, now + 1);
    h.record(f.a_id(), fresh, now + 1).await.unwrap();
    assert_eq!(
        h.clear(f.a_id(), 0).await.unwrap().generation,
        1,
        "exact clear retry is idempotent"
    );
    assert_eq!(
        h.snapshot(f.a_id(), now + 1).await.unwrap().1[0].term,
        "space",
        "late clear retry must retain post-clear searches"
    );
    assert_eq!(
        h.snapshot(f.b_id(), now).await.unwrap().1[0].term,
        "jazz",
        "one user cannot clear another user's searches"
    );
}

#[tokio::test]
async fn history_is_bounded_expires_and_rejects_old_or_invalid_events() {
    let f = Fixture::new().await;
    let h = f.history();
    let now = Utc::now().timestamp_millis();
    for i in 0..70 {
        h.record(f.a_id(), event(&format!("search {i}"), 0, now + i), now + i)
            .await
            .unwrap();
    }
    let (_, rows) = h.snapshot(f.a_id(), now + 70).await.unwrap();
    assert_eq!(rows.len(), 64);
    assert_eq!(rows[0].term, "search 69");
    assert_eq!(rows.last().unwrap().term, "search 6");
    assert!(
        h.record(
            f.a_id(),
            event("old replay", 0, now - RETRY_WINDOW_MS - 1),
            now
        )
        .await
        .is_err()
    );
    assert!(
        h.record(f.a_id(), event("future", 0, now + CLOCK_SKEW_MS + 1), now)
            .await
            .is_err()
    );
    for term in [
        "x".to_owned(),
        " ".into(),
        "\nprivate".into(),
        "a".repeat(129),
    ] {
        assert!(h.record(f.a_id(), event(&term, 0, now), now).await.is_err());
    }
    assert!(
        h.snapshot(f.a_id(), now + RETENTION_MS + 100)
            .await
            .unwrap()
            .1
            .is_empty()
    );
}

#[tokio::test]
async fn search_history_survives_owner_database_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("home.db").display()
    );
    let db = Database::connect_sized(&url, Some(2)).await.unwrap();
    db.run_migrations().await.unwrap();
    let f = Fixture::with_db(db).await;
    let user = f.a_id();
    let now = Utc::now().timestamp_millis();
    f.history()
        .record(user, event("documentaries", 0, now), now)
        .await
        .unwrap();
    f.db.close().await;
    let db = Database::connect_sized(&url, Some(2)).await.unwrap();
    db.run_migrations().await.unwrap();
    let history = History::new(db);
    assert_eq!(
        history.snapshot(user, now).await.unwrap().1[0].term,
        "documentaries"
    );
}

#[tokio::test]
async fn home_routes_reject_user_selection_api_keys_and_invisible_items() {
    let f = Fixture::new().await;
    let state = f.state(f.a.clone(), false);
    let now = Utc::now().timestamp_millis();
    let first = event("ocean", 0, now);
    let mut spoof = body(&first);
    spoof["userId"] = json!(f.b_id());
    assert_eq!(
        request(state.clone(), "POST", "/Bluefin/Home/Searches", spoof)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        request(
            state.clone(),
            "POST",
            &format!("/Bluefin/Home/Searches?userId={}", f.b_id()),
            body(&first)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            f.state(f.a.clone(), true),
            "POST",
            "/Bluefin/Home/Searches",
            body(&first)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut hidden = first.clone();
    hidden.selected_item_id = Some(Uuid::new_v4());
    assert_eq!(
        request(
            state.clone(),
            "POST",
            "/Bluefin/Home/Searches",
            body(&hidden)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert!(
        f.history()
            .snapshot(f.a_id(), now)
            .await
            .unwrap()
            .1
            .is_empty()
    );
    assert_eq!(
        request(
            state.clone(),
            "POST",
            "/Bluefin/Home/Searches",
            body(&first)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        f.history().snapshot(f.a_id(), now).await.unwrap().1.len(),
        1
    );
    assert!(
        f.history()
            .snapshot(f.b_id(), now)
            .await
            .unwrap()
            .1
            .is_empty()
    );
    assert_eq!(
        request(
            state,
            "DELETE",
            "/Bluefin/Home/Searches",
            json!({"generation":0})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(
        f.history()
            .snapshot(f.a_id(), now)
            .await
            .unwrap()
            .1
            .is_empty()
    );
}

#[tokio::test]
async fn cold_start_is_empty_and_does_not_invent_viewing_or_search_history() {
    let f = Fixture::new().await;
    let (status, home) = request(
        f.state(f.a.clone(), false),
        "GET",
        "/Bluefin/Home/Recommendations",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(home["recommendations"], json!([]));
    assert_eq!(home["searchHistoryGeneration"], 0);
    assert_eq!(home["searchHistorySupported"], true);
    assert!(
        f.history()
            .snapshot(f.a_id(), Utc::now().timestamp_millis())
            .await
            .unwrap()
            .1
            .is_empty()
    );
}

#[test]
fn recommendation_bound_preserves_rank_and_deduplicates_between_shelves() {
    let items = |start, end| {
        (start..end)
            .map(|id| {
                crate::test_support::minimal_base_item(Uuid::from_u128(id), "fixture", "Movie")
            })
            .collect()
    };
    let rows = bound_shelves(vec![
        ShelfRows {
            id: "first".into(),
            title: "First".into(),
            reason: "watched",
            items: items(1, 21),
        },
        ShelfRows {
            id: "second".into(),
            title: "Second".into(),
            reason: "search",
            items: items(1, 25),
        },
    ]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].items.len(), 12);
    assert_eq!(rows[1].items.len(), 12);
    let ids: Vec<_> = rows
        .iter()
        .flat_map(|s| &s.items)
        .map(|i| Uuid::parse_str(&i.id).unwrap())
        .collect();
    assert_eq!(ids, (1..25).map(Uuid::from_u128).collect::<Vec<_>>());
}
