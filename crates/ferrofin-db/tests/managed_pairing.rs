//! Synthetic SQLite proof of the actual shipped mint/cancel triggers. No server,
//! account, network, or live database is used.
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
async fn fixture(url: &str) -> SqlitePool {
    let db = SqlitePoolOptions::new()
        .max_connections(2)
        .connect(url)
        .await
        .unwrap();
    sqlx::raw_sql(
        "CREATE TABLE Devices(Id INTEGER PRIMARY KEY,DeviceId TEXT,UserId TEXT,AccessToken TEXT);",
    )
    .execute(&db)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!("../migrations/0050_managed_pairing.sql"))
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO FerrofinManagedPairing VALUES ('a','hash','bluefin-managed:a','watch','app','1','Watch','loopback',unixepoch(),unixepoch()+300,'minting','user')").execute(&db).await.unwrap();
    db
}
async fn mint(db: &SqlitePool) -> Result<sqlx::sqlite::SqliteQueryResult, sqlx::Error> {
    sqlx::query("INSERT INTO Devices(DeviceId,UserId,AccessToken) VALUES ('bluefin-managed:a','user','synthetic-token')").execute(db).await
}
async fn cancel(db: &SqlitePool) -> Result<sqlx::sqlite::SqliteQueryResult, sqlx::Error> {
    sqlx::query("UPDATE FerrofinManagedPairing SET State='cancelled' WHERE Id='a'")
        .execute(db)
        .await
}
async fn state(db: &SqlitePool) -> String {
    sqlx::query_scalar("SELECT State FROM FerrofinManagedPairing WHERE Id='a'")
        .fetch_one(db)
        .await
        .unwrap()
}
async fn tokens(db: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM Devices")
        .fetch_one(db)
        .await
        .unwrap()
}
#[tokio::test]
async fn cancel_before_late_mint_is_unmintable() {
    let db = fixture("sqlite::memory:").await;
    cancel(&db).await.unwrap();
    assert!(mint(&db).await.is_err());
    cancel(&db).await.unwrap();
    assert_eq!(state(&db).await, "cancelled");
    assert_eq!(tokens(&db).await, 0);
}
#[tokio::test]
async fn lost_mint_reply_restart_and_cancel() {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        tmp.path().join("fixture.db").display()
    );
    let db = fixture(&url).await;
    mint(&db).await.unwrap();
    assert_eq!(state(&db).await, "authorized");
    db.close().await;
    let reopened = SqlitePool::connect(&url).await.unwrap();
    assert_eq!(tokens(&reopened).await, 1);
    cancel(&reopened).await.unwrap();
    assert_eq!(tokens(&reopened).await, 0);
    assert!(mint(&reopened).await.is_err());
}
#[tokio::test]
async fn failed_cleanup_cannot_commit_receipt() {
    let db = fixture("sqlite::memory:").await;
    mint(&db).await.unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_cleanup BEFORE DELETE ON Devices BEGIN SELECT RAISE(ABORT,'synthetic cleanup failure'); END;").execute(&db).await.unwrap();
    assert!(cancel(&db).await.is_err());
    assert_eq!(state(&db).await, "authorized");
    assert_eq!(tokens(&db).await, 1);
    sqlx::query("DROP TRIGGER fail_cleanup")
        .execute(&db)
        .await
        .unwrap();
    cancel(&db).await.unwrap();
    assert_eq!(tokens(&db).await, 0);
}
#[tokio::test]
async fn receipt_store_failure_rolls_back_token_mint() {
    let db = fixture("sqlite::memory:").await;
    sqlx::raw_sql("CREATE TRIGGER fail_receipt BEFORE UPDATE ON FerrofinManagedPairing WHEN NEW.State='authorized' BEGIN SELECT RAISE(ABORT,'synthetic store failure'); END;").execute(&db).await.unwrap();
    assert!(mint(&db).await.is_err());
    assert_eq!(tokens(&db).await, 0);
    assert_eq!(state(&db).await, "minting");
    cancel(&db).await.unwrap();
}
#[tokio::test]
async fn expired_and_wrong_user_mints_are_refused() {
    let db = fixture("sqlite::memory:").await;
    assert!(sqlx::query("INSERT INTO Devices(DeviceId,UserId,AccessToken) VALUES ('bluefin-managed:a','other','other-token')").execute(&db).await.is_err());
    sqlx::query("UPDATE FerrofinManagedPairing SET Expires=0")
        .execute(&db)
        .await
        .unwrap();
    assert!(mint(&db).await.is_err());
}
#[tokio::test]
async fn concurrent_mint_and_cancel_leave_no_token() {
    for _ in 0..32 {
        let db = fixture("sqlite::memory:").await;
        let (m, c) = tokio::join!(mint(&db), cancel(&db));
        let _ = m;
        c.unwrap();
        assert_eq!(state(&db).await, "cancelled");
        assert_eq!(tokens(&db).await, 0);
        assert!(mint(&db).await.is_err());
    }
}
