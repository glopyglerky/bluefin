//! Administrator device-removal journeys use real durable mesh grants and
//! device storage. Only request identity and playback cleanup are test doubles.
use super::*;
use axum::{body::Body, http::{Request as HttpRequest, StatusCode}};
use ferrofin_core::device_manager::FerrofinDeviceManager;
use tower::ServiceExt;

async fn delete(f: &Fixture, owner: Option<Arc<Service>>, administrator: bool, ids: &str) -> StatusCode {
    let mut inner = (*f.state).clone();
    inner.devices = Arc::new(FerrofinDeviceManager::new(f.db.clone()));
    inner.auth_service = if administrator {
        Arc::new(crate::test_support::ApiKeyAuthService)
    } else {
        Arc::new(crate::test_support::AuthedAuthService)
    };
    let state = AppState::from_inner(inner);
    let state = match owner {
        Some(owner) => state.with_mesh_accounts(owner),
        None => state,
    };
    let response = crate::handlers::devices::register(Router::new()).with_state(state)
        .oneshot(HttpRequest::builder().method("DELETE").uri(format!("/Devices?id={ids}"))
            .header("X-Emby-Token", "test-administrator-only").body(Body::empty()).unwrap())
        .await.unwrap();
    response.status()
}

async fn session_device(f: &Fixture, session: &str) -> (Uuid, String) {
    let row = sqlx::query("SELECT UserId,DeviceId FROM BluefinMeshSessions WHERE Id=?")
        .bind(session).fetch_one(f.db.pool()).await.unwrap();
    (Uuid::parse_str(&row.get::<String, _>("UserId")).unwrap(), row.get("DeviceId"))
}

async fn bearer(f: &Fixture, user: Uuid, device_id: &str, token: &str) {
    let stamp = chrono::Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO Devices(AccessToken,AppName,AppVersion,DateCreated,DateLastActivity,DateModified,DeviceId,DeviceName,IsActive,UserId) VALUES(?,?,?,?,?,?,?,?,1,?)")
        .bind(token).bind("Bluefin").bind("1").bind(&stamp).bind(&stamp).bind(&stamp)
        .bind(device_id).bind("Device removal test").bind(guid_to_db(user))
        .execute(f.db.writer()).await.unwrap();
}

async fn revoked(f: &Fixture, session: &str) -> i64 {
    sqlx::query_scalar("SELECT Revoked FROM BluefinMeshSessions WHERE Id=?")
        .bind(session).fetch_one(f.db.pool()).await.unwrap()
}

async fn bearer_count(f: &Fixture, device_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM Devices WHERE DeviceId=?")
        .bind(device_id).fetch_one(f.db.pool()).await.unwrap()
}

async fn assert_denied(f: &Fixture, owner: &Service, device: &DeviceIdentity, session: &str) {
    assert!(matches!(owner.authorization(&f.state, &device.public_key(), session, "test").await,
        Err(ApiError::Unauthorized(_))), "revoked grant must fail at the authority check");
    assert_eq!(f.sessions.mint_attempts(), 0, "revocation must not reach bearer minting");
}

#[tokio::test]
async fn admin_device_delete_revokes_unminted_grant_and_preserves_unrelated_devices() {
    let f = Fixture::new().await;
    let target = DeviceIdentity::from_seed([101; 32]).unwrap();
    let session = signed_in_mesh_session(&f, &target).await;
    let (user, device_id) = session_device(&f, &session).await;
    assert_eq!(bearer_count(&f, &device_id).await, 0);

    let other = DeviceIdentity::from_seed([102; 32]).unwrap();
    let other_session = hex(&[103; 32]);
    let mut tx = f.db.writer().begin().await.unwrap();
    f.service.insert_session(&mut tx, &other_session, user, &other.public_key(), "Other device", now())
        .await.unwrap();
    tx.commit().await.unwrap();
    let (_, other_id) = session_device(&f, &other_session).await;
    bearer(&f, user, &other_id, "unrelated-mesh-bearer").await;
    bearer(&f, user, "ordinary-device", "unrelated-ordinary-bearer").await;
    f.sessions.add("other-device-playback", user, &other_id);
    f.sessions.add("other-account-playback", Uuid::new_v4(), &device_id);

    assert_eq!(delete(&f, Some(f.service.clone()), true, &device_id).await, StatusCode::NO_CONTENT);
    assert_eq!(revoked(&f, &session).await, 1);
    assert_denied(&f, &f.service, &target, &session).await;
    assert!(!f.sessions.contains("target-playback"));
    assert!(f.sessions.contains("other-device-playback"));
    assert!(f.sessions.contains("other-account-playback"));
    assert_eq!(revoked(&f, &other_session).await, 0);
    assert_eq!(bearer_count(&f, &other_id).await, 1);
    assert_eq!(bearer_count(&f, "ordinary-device").await, 1);
    assert!(f.service.authorization(&f.state, &other.public_key(), &other_session, "test").await.is_ok());
    // The durable grant supplies idempotency even though there never was a bearer.
    assert_eq!(delete(&f, Some(f.service.clone()), true, &device_id).await, StatusCode::NO_CONTENT);
    assert_eq!(f.sessions.close_attempts(), ["target-playback"]);
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM BluefinMeshSessionRevocations")
        .fetch_one(f.db.pool()).await.unwrap(), 0, "administrator must not forge a device receipt");
}

#[tokio::test]
async fn admin_device_delete_is_terminal_before_cleanup_and_retries_after_restart() {
    let f = Fixture::new().await;
    let target = DeviceIdentity::from_seed([104; 32]).unwrap();
    let session = signed_in_mesh_session(&f, &target).await;
    let (_, device_id) = session_device(&f, &session).await;
    mesh_test_bearer(&f, &session).await;
    assert!(f.service.authorization(&f.state, &target.public_key(), &session, "test").await.is_ok());

    f.sessions.fail_next_close();
    assert_eq!(delete(&f, Some(f.service.clone()), true, &device_id).await, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(revoked(&f, &session).await, 1);
    assert_eq!(bearer_count(&f, &device_id).await, 0);
    assert_denied(&f, &f.service, &target, &session).await;
    assert!(f.sessions.contains("target-playback"));

    let restarted = Service::new(f.db.clone(), "https://accounts.example", address()).await.unwrap();
    assert_eq!(delete(&f, Some(f.service.clone()), true, &device_id).await, StatusCode::CONFLICT);
    assert_eq!(delete(&f, Some(restarted.clone()), true, &device_id).await, StatusCode::NO_CONTENT);
    assert!(!f.sessions.contains("target-playback"));
    assert_eq!(delete(&f, Some(restarted.clone()), true, &device_id).await, StatusCode::NO_CONTENT);
    assert_eq!(f.sessions.close_attempts(), ["target-playback", "target-playback"]);
    assert_denied(&f, &restarted, &target, &session).await;
    // Device-key self-revocation remains a distinct, exact-retry protocol.
    let receipt = restarted.revoke_session(&f.state, &target.public_key(), &session, [105; 32]).await.unwrap();
    assert_eq!(restarted.revoke_session(&f.state, &target.public_key(), &session, [105; 32]).await.unwrap(), receipt);
}

#[tokio::test]
async fn admin_device_delete_validates_all_ids_and_requires_administrator() {
    let f = Fixture::new().await;
    let target = DeviceIdentity::from_seed([106; 32]).unwrap();
    let session = signed_in_mesh_session(&f, &target).await;
    let (user, device_id) = session_device(&f, &session).await;
    mesh_test_bearer(&f, &session).await;
    bearer(&f, user, "ordinary-device", "unrelated-ordinary-bearer").await;
    let unknown = format!("bluefin-mesh:{}", hex(&[107; 32]));

    for ids in [
        format!("{device_id},ordinary-device,{unknown}"),
        format!("{device_id},missing-ordinary-device"),
        format!("{device_id},bluefin-mesh:invalid"),
    ] {
        assert_eq!(delete(&f, Some(f.service.clone()), true, &ids).await, StatusCode::BAD_REQUEST);
        assert_eq!(revoked(&f, &session).await, 0);
        assert_eq!(bearer_count(&f, &device_id).await, 1);
        assert_eq!(bearer_count(&f, "ordinary-device").await, 1);
        assert!(f.sessions.close_attempts().is_empty());
    }
    assert_eq!(delete(&f, Some(f.service.clone()), false, &device_id).await, StatusCode::FORBIDDEN);
    assert_eq!(delete(&f, None, true, &device_id).await, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(revoked(&f, &session).await, 0);
    assert_eq!(bearer_count(&f, &device_id).await, 1);
    assert!(f.sessions.contains("target-playback"));
}

#[tokio::test]
async fn admin_device_delete_fences_stale_owner_before_revocation() {
    let f = Fixture::new().await;
    let target = DeviceIdentity::from_seed([108; 32]).unwrap();
    let session = signed_in_mesh_session(&f, &target).await;
    let (_, device_id) = session_device(&f, &session).await;
    mesh_test_bearer(&f, &session).await;
    let restarted = Service::new(f.db.clone(), "https://accounts.example", address()).await.unwrap();

    assert_eq!(delete(&f, Some(f.service.clone()), true, &device_id).await, StatusCode::CONFLICT);
    assert_eq!(revoked(&f, &session).await, 0);
    assert_eq!(bearer_count(&f, &device_id).await, 1);
    assert!(f.sessions.close_attempts().is_empty());
    assert_eq!(delete(&f, Some(restarted.clone()), true, &format!("{device_id},{device_id}")).await,
        StatusCode::NO_CONTENT);
    assert_eq!(f.sessions.close_attempts(), ["target-playback"]);
    assert_denied(&f, &restarted, &target, &session).await;
}
