//! Runs the real archive staging/swap and account authorization paths.
use std::{path::{Path, PathBuf}, sync::Arc};

use axum::{body::Body, http::{Request, StatusCode}};
use bluefin_mesh_connect::{ServerAddress, native::DeviceIdentity};
use ferrofin_core::{authorization_context::FerrofinAuthorizationContext, user_manager::FerrofinUserManager};
use ferrofin_db::{Database, store::guid_to_db};
use ferrofin_traits::library::UserManager as _;
use sqlx::Connection as _;
use tower::ServiceExt as _;

use super::super::{BackupOptions, TreeRoots, apply_pending_restore, write_backup};
use crate::{handlers::bluefin_mesh_accounts::{Service, SERVICE}, state::AppState};

const MESH_TOKEN: &str = "synthetic-restored-mesh-token";
const ORDINARY_TOKEN: &str = "synthetic-unrelated-device-token";

fn address() -> ServerAddress {
    ServerAddress {
        endpoint_id: [1; 32], server_certificate_sha256: [2; 32], service: SERVICE.into(),
        direct_addresses: vec!["127.0.0.1:49215".parse().unwrap()], relay_urls: vec![],
    }
}

fn state(db: &Database) -> AppState {
    let mut inner = (*crate::test_support::fake_state()).clone();
    inner.users = Arc::new(FerrofinUserManager::new(db.clone()));
    let auth = Arc::new(FerrofinAuthorizationContext::new(
        db.clone(), inner.users.clone(), inner.app_host.clone(), inner.config.clone(), "restore-test", "1",
    ));
    inner.auth_context = auth.clone();
    inner.auth_service = auth;
    AppState::from_inner(inner)
}

async fn http_status(state: &AppState, token: &str) -> StatusCode {
    crate::create_router(state.clone())
        .oneshot(Request::builder().uri("/Users/Me")
            .header("authorization", format!("MediaBrowser Token=\"{token}\""))
            .body(Body::empty()).unwrap())
        .await.unwrap().status()
}

async fn seeded() -> (Database, AppState, Arc<Service>, DeviceIdentity, String, String) {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let state = state(&db);
    let user = state.users.create_signup_user("restore-viewer", "A long synthetic password").await.unwrap();
    let user = guid_to_db(uuid::Uuid::parse_str(&user.id).unwrap());
    let service = Service::new(db.clone(), "https://accounts.example", address()).await.unwrap();
    let device = DeviceIdentity::from_seed([61; 32]).unwrap();
    let id = "ab".repeat(32);
    let created = chrono::Utc::now().timestamp_millis();
    sqlx::query("INSERT INTO BluefinMeshSessions(Id,UserId,DevicePublicKey,DeviceLabel,DeviceId,Created,Expires) VALUES(?,?,?,?,?,?,?)")
        .bind(&id).bind(&user).bind(device.public_key()).bind("Restore test").bind(format!("bluefin-mesh:{id}"))
        .bind(created).bind(created + 86_400_000).execute(db.writer()).await.unwrap();
    let stamp = chrono::Utc::now().to_rfc3339();
    for (device_id, token) in [(format!("bluefin-mesh:{id}"), MESH_TOKEN), ("ordinary-device".into(), ORDINARY_TOKEN)] {
        sqlx::query("INSERT INTO Devices(AccessToken,AppName,AppVersion,DateCreated,DateLastActivity,DateModified,DeviceId,DeviceName,IsActive,UserId) VALUES(?,?,?,?,?,?,?,?,1,?)")
            .bind(token).bind("Bluefin").bind("1").bind(&stamp).bind(&stamp).bind(&stamp)
            .bind(device_id).bind("Restore test").bind(&user).execute(db.writer()).await.unwrap();
    }
    (db, state, service, device, id, user)
}

async fn archived(db: &Database, directory: &Path) -> (TreeRoots, PathBuf) {
    let source = directory.join("source");
    std::fs::create_dir_all(source.join("config")).unwrap();
    std::fs::write(source.join("config/retained.json"), b"archived configuration").unwrap();
    let roots = TreeRoots::defaults(&source, &source.join("config"), &source.join("unused.db"));
    let snapshot = source.join("snapshot.db");
    db.snapshot_to(&snapshot).await.unwrap();
    let target = directory.join("target");
    let target_roots = TreeRoots::defaults(&target, &target.join("config"), &target.join("ferrofin.db"));
    let backups = target.join("data/backups");
    let manifest = write_backup(&backups, &roots, BackupOptions::default(), chrono::Utc::now(), Some(&snapshot)).unwrap();
    let archive = PathBuf::from(manifest.path);
    std::fs::write(backups.join("restore-pending"), archive.file_name().unwrap().as_encoded_bytes()).unwrap();
    (target_roots, archive)
}

#[tokio::test]
async fn restored_old_mesh_session_and_http_bearer_cannot_authorize() {
    let directory = tempfile::tempdir().unwrap();
    let (db, before, service, device, session, user) = seeded().await;
    // Public HTTP rejects internal mesh bearer material even before restore.
    assert_eq!(http_status(&before, MESH_TOKEN).await, StatusCode::UNAUTHORIZED);
    assert_eq!(http_status(&before, ORDINARY_TOKEN).await, StatusCode::OK);
    assert!(service.authorization(&before, &device.public_key(), &session, "test").await.is_ok());
    let (roots, archive) = archived(&db, directory.path()).await;
    // This backup precedes the revocation. The live owner applies the same
    // terminal state change used by its revocation operation.
    sqlx::query("UPDATE BluefinMeshSessions SET Revoked=1 WHERE Id=?")
        .bind(&session).execute(db.writer()).await.unwrap();
    assert!(service.authorization(&before, &device.public_key(), &session, "test").await.is_err());
    assert_eq!(http_status(&state(&db), MESH_TOKEN).await, StatusCode::UNAUTHORIZED);
    let target = roots.database.clone();
    db.snapshot_to(&target).await.unwrap();
    let restored_path = target.clone();
    let restored_archive = tokio::task::spawn_blocking(move || apply_pending_restore(&roots)).await.unwrap().unwrap();
    assert_eq!(restored_archive, Some(archive));
    let restored = Database::connect(&format!("sqlite://{}", restored_path.display())).await.unwrap();
    let after = state(&restored);
    assert_eq!(http_status(&after, MESH_TOKEN).await, StatusCode::UNAUTHORIZED);
    assert_eq!(http_status(&after, ORDINARY_TOKEN).await, StatusCode::OK);
    assert!(after.users.get_user_by_id(uuid::Uuid::parse_str(&user).unwrap()).await.unwrap().is_some());
    let restarted = Service::new(restored.clone(), "https://accounts.example", address()).await.unwrap();
    assert!(restarted.authorization(&after, &device.public_key(), &session, "test").await.is_err());
    // Reopening the restored database must not remint the missing bearer.
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM Devices WHERE AccessToken=?")
        .bind(MESH_TOKEN).fetch_one(restored.pool()).await.unwrap(), 0);
    restored.close().await;
    db.close().await;
}

#[tokio::test]
async fn failed_mesh_invalidation_replaces_no_live_file() {
    let directory = tempfile::tempdir().unwrap();
    let (db, _, _, _, _, _) = seeded().await;
    sqlx::query("CREATE TRIGGER RefuseMeshRestore BEFORE DELETE ON BluefinMeshSessions BEGIN SELECT RAISE(ABORT,'test interruption'); END")
        .execute(db.writer()).await.unwrap();
    let (roots, _) = archived(&db, directory.path()).await;
    std::fs::create_dir_all(&roots.config).unwrap();
    std::fs::write(&roots.database, b"untouched current database").unwrap();
    std::fs::write(roots.config.join("retained.json"), b"untouched current configuration").unwrap();
    let database = roots.database.clone();
    let config = roots.config.clone();
    let failure = tokio::task::spawn_blocking(move || apply_pending_restore(&roots)).await.unwrap().unwrap_err();
    assert!(!failure.partially_applied);
    assert_eq!(std::fs::read(database).unwrap(), b"untouched current database");
    assert_eq!(std::fs::read(config.join("retained.json")).unwrap(), b"untouched current configuration");
    db.close().await;
}

#[tokio::test]
async fn database_without_mesh_tables_keeps_its_content() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy.db");
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&path).create_if_missing(true);
    let mut connection = sqlx::SqliteConnection::connect_with(&options).await.unwrap();
    sqlx::query("CREATE TABLE Retained(Value TEXT NOT NULL)").execute(&mut connection).await.unwrap();
    sqlx::query("INSERT INTO Retained VALUES ('legacy content')").execute(&mut connection).await.unwrap();
    connection.close().await.unwrap();
    let staged = path.clone();
    tokio::task::spawn_blocking(move || super::invalidate_staged(&staged)).await.unwrap().unwrap();
    let mut connection = sqlx::SqliteConnection::connect_with(&options).await.unwrap();
    let value: String = sqlx::query_scalar("SELECT Value FROM Retained").fetch_one(&mut connection).await.unwrap();
    assert_eq!(value, "legacy content");
    connection.close().await.unwrap();
}


#[tokio::test]
async fn orphan_mesh_bearers_are_removed_without_session_tables() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("orphan.db");
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&path).create_if_missing(true);
    let mut connection = sqlx::SqliteConnection::connect_with(&options).await.unwrap();
    // SQLite identifiers are case-insensitive; classification must be too.
    sqlx::query("CREATE TABLE devices(DeviceId TEXT NOT NULL, AccessToken TEXT NOT NULL)")
        .execute(&mut connection).await.unwrap();
    sqlx::query("INSERT INTO devices VALUES ('bluefin-mesh:orphan','old-mesh'),('ordinary-device','retained')")
        .execute(&mut connection).await.unwrap();
    connection.close().await.unwrap();
    let staged = path.clone();
    tokio::task::spawn_blocking(move || super::invalidate_staged(&staged)).await.unwrap().unwrap();
    let mut connection = sqlx::SqliteConnection::connect_with(&options).await.unwrap();
    let tokens: Vec<String> = sqlx::query_scalar("SELECT AccessToken FROM devices")
        .fetch_all(&mut connection).await.unwrap();
    assert_eq!(tokens, vec!["retained"]);
    connection.close().await.unwrap();
}

#[tokio::test]
async fn a_devices_table_without_device_identity_cannot_be_restored() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("unclassifiable.db");
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&path).create_if_missing(true);
    let mut connection = sqlx::SqliteConnection::connect_with(&options).await.unwrap();
    sqlx::query("CREATE TABLE Devices(AccessToken TEXT NOT NULL)").execute(&mut connection).await.unwrap();
    sqlx::query("INSERT INTO Devices VALUES ('unclassifiable-token')").execute(&mut connection).await.unwrap();
    connection.close().await.unwrap();
    let staged = path.clone();
    assert!(tokio::task::spawn_blocking(move || super::invalidate_staged(&staged)).await.unwrap().is_err());
}


#[tokio::test]
async fn revocation_table_without_core_mesh_schema_is_not_legacy() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("partial.db");
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&path).create_if_missing(true);
    let mut connection = sqlx::SqliteConnection::connect_with(&options).await.unwrap();
    sqlx::query("CREATE TABLE BluefinMeshSessionRevocations(SessionId TEXT NOT NULL)")
        .execute(&mut connection).await.unwrap();
    connection.close().await.unwrap();
    let staged = path.clone();
    assert!(tokio::task::spawn_blocking(move || super::invalidate_staged(&staged)).await.unwrap().is_err());
}
