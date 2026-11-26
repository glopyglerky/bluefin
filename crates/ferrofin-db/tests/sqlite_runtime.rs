//! The actual SQLx-linked SQLite security floor and on-disk upgrade contract.

use chrono::{DateTime, Utc};
use ferrofin_db::Database;
use ferrofin_db::store::{datetime_to_db, guid_to_db};
use sqlx::Row;
use uuid::Uuid;

#[tokio::test]
async fn linked_sqlite_meets_security_floor() {
    let db = Database::connect_in_memory().await.expect("connect");
    let (version, source): (String, String) =
        sqlx::query_as("SELECT sqlite_version(), sqlite_source_id()")
            .fetch_one(db.pool())
            .await
            .expect("query actual SQLx driver");
    // SAFETY: this read-only identity function has no arguments or setup precondition.
    let number = unsafe { libsqlite3_sys::sqlite3_libversion_number() };
    let parts: Vec<i32> = version.split('.').map(|v| v.parse().unwrap()).collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(number, parts[0] * 1_000_000 + parts[1] * 1_000 + parts[2]);
    println!("SQLx SQLite {version}; libversion_number={number}; source_id={source}");
    assert!(number >= 3_054_000, "RUSTSEC-2026-0459: {version}");
    db.close().await;
}

fn identity() -> (Uuid, DateTime<Utc>) {
    (
        Uuid::parse_str("dc68cd36-0909-4b02-b129-8b1ba641a16a").unwrap(),
        "2026-08-11T17:46:47.753964700Z".parse().unwrap(),
    )
}

async fn assert_probe(db: &Database) {
    let (id, date) = identity();
    let row = sqlx::query("SELECT *, typeof(native_id) AS id_type, typeof(native_date) AS date_type FROM SQLiteUpgradeProbe")
        .fetch_one(db.pool()).await.expect("read persisted probe");
    assert_eq!(row.get::<Uuid, _>("native_id"), id);
    assert_eq!(row.get::<DateTime<Utc>, _>("native_date"), date);
    assert_eq!(row.get::<String, _>("id_type"), "blob");
    assert_eq!(row.get::<String, _>("date_type"), "text");
    assert_eq!(row.get::<String, _>("stored_id"), guid_to_db(id));
    assert_eq!(row.get::<String, _>("stored_date"), datetime_to_db(date));
    assert_eq!(row.get::<String, _>("payload"), "committed — synthetic");
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(integrity, "ok");
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(db.pool())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn wal_reopen_migrations_and_query_encodings_survive_upgrade() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.db");
    let url = format!("sqlite://{}", path.display());
    let db = Database::connect_sized(&url, Some(2)).await.unwrap();
    db.run_migrations().await.unwrap();
    sqlx::query("CREATE TABLE SQLiteUpgradeProbe (native_id BLOB, native_date TEXT, stored_id TEXT, stored_date TEXT, payload TEXT)")
        .execute(db.writer()).await.unwrap();
    let (id, date) = identity();
    sqlx::query("INSERT INTO SQLiteUpgradeProbe VALUES (?1, ?2, ?3, ?4, ?5)")
        .bind(id)
        .bind(date)
        .bind(guid_to_db(id))
        .bind(datetime_to_db(date))
        .bind("committed — synthetic")
        .execute(db.writer())
        .await
        .unwrap();
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(mode, "wal");
    assert!(path.with_extension("db-wal").metadata().unwrap().len() > 32);
    let mut transaction = db.writer().begin().await.unwrap();
    sqlx::query("UPDATE SQLiteUpgradeProbe SET payload = 'uncommitted'")
        .execute(&mut *transaction)
        .await
        .unwrap();
    // A separate reader sees the committed WAL state, never the dirty writer.
    assert_probe(&db).await;
    transaction.rollback().await.unwrap();
    if let Some(destination) = std::env::var_os("BLUEFIN_SQLITE_SAVE_FIXTURE") {
        db.snapshot_to(std::path::Path::new(&destination))
            .await
            .unwrap();
    }
    db.close().await;
    let reopened = Database::connect_sized(&url, Some(2)).await.unwrap();
    reopened.run_migrations().await.unwrap();
    assert_probe(&reopened).await;
    reopened.close().await;

    // Optional retained pre-upgrade synthetic snapshot, copied before any write.
    // The normal test still exercises the complete contract without this input.
    if let Some(before) = std::env::var_os("BLUEFIN_SQLITE_PREUPGRADE_DB") {
        std::fs::copy(before, &path).unwrap();
        let upgraded = Database::connect_sized(&url, Some(2)).await.unwrap();
        upgraded.run_migrations().await.unwrap();
        assert_probe(&upgraded).await;
        upgraded.close().await;
    }
}
