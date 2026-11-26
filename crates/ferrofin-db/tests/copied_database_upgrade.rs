//! Explicit, private-fixture upgrade qualification. This test never opens a live
//! database and is ignored unless the build owner admits its sealed snapshot.
//! The fixture stays outside this repository and outside release artifacts.

use std::io::Read as _;
use std::path::Path;

use ferrofin_db::Database;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(PartialEq, Eq, sqlx::FromRow)]
struct LedgerRow {
    version: i64,
    description: String,
    installed_on: String,
    success: bool,
    checksum: Vec<u8>,
    execution_time: i64,
}

// Private database values must never appear in assertion diagnostics.
fn checked<T, E>(result: Result<T, E>, operation: &'static str) -> T {
    match result {
        Ok(value) => value,
        Err(_) => panic!("{operation} failed"),
    }
}

async fn ledger(pool: &SqlitePool) -> Vec<LedgerRow> {
    checked(
        sqlx::query_as::<_, LedgerRow>(
            "SELECT version, description, installed_on, success, checksum, execution_time
             FROM _sqlx_migrations ORDER BY version",
        )
        .fetch_all(pool)
        .await,
        "read migration ledger",
    )
}

async fn integrity(pool: &SqlitePool) {
    let rows: Vec<String> = checked(
        sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_all(pool)
            .await,
        "database integrity check",
    );
    assert!(rows.len() == 1 && rows[0] == "ok", "database integrity failed");
}

fn identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[tokio::test]
#[ignore = "requires an admitted private snapshot in a read-only fixture input"]
async fn copied_version_51_database_upgrades_without_changing_existing_content() {
    let supplied = std::env::var_os("BLUEFIN_UPGRADE_SNAPSHOT")
        .expect("an explicit private fixture admission is required");
    let fixture = Path::new(&supplied);
    assert!(
        fixture == Path::new("/vendor/qualification/upgrade.sqlite"),
        "fixture must use the declared read-only qualification input"
    );
    let metadata = checked(std::fs::symlink_metadata(fixture), "inspect sealed fixture");
    assert!(metadata.file_type().is_file(), "fixture must be a regular file");
    let max_bytes = 1024 * 1024 * 1024;
    assert!(
        metadata.len() > 0 && metadata.len() <= max_bytes,
        "fixture exceeds the admitted per-file bound"
    );

    // Fresh bounded build storage; no inherited read-only permissions and no
    // mutation of either the sealed input or the running installation.
    let temporary = checked(
        tempfile::Builder::new()
            .prefix("private-database-upgrade-")
            .tempdir_in("/build"),
        "create private build fixture",
    );
    let copy = temporary.path().join("upgrade.sqlite");
    let mut input = checked(std::fs::File::open(fixture), "open sealed fixture")
        .take(max_bytes + 1);
    let mut output = checked(
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&copy),
        "create writable fixture copy",
    );
    let copied = checked(std::io::copy(&mut input, &mut output), "copy sealed fixture");
    assert!(copied == metadata.len(), "fixture changed while being copied");
    checked(output.sync_all(), "sync private fixture copy");
    drop(output);
    drop(input);

    let before = checked(
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&copy)
                    .read_only(true),
            )
            .await,
        "open copied baseline read-only",
    );
    let original_ledger = ledger(&before).await;
    assert!(
        original_ledger.len() == 51
            && original_ledger
                .iter()
                .enumerate()
                .all(|(index, row)| row.version == index as i64 + 1 && row.success),
        "fixture must contain exactly the deployed successful versions 1 through 51"
    );
    integrity(&before).await;
    before.close().await;

    let url = format!("sqlite://{}", copy.display());
    // This is the actual application entry point, including migration preflight,
    // backup, the compiled sqlx migrator and post-migration foreign-key checks.
    let db = checked(
        Database::connect_sized(&url, Some(1)).await,
        "application upgrade of private database copy",
    );
    checked(db.run_migrations().await, "repeat migration after connection");
    let upgraded_ledger = ledger(db.pool()).await;
    assert_eq!(
        upgraded_ledger.len(),
        MIGRATOR.iter().count(),
        "upgraded ledger must cover every compiled migration"
    );
    assert!(
        upgraded_ledger[..51] == original_ledger,
        "an existing migration receipt changed"
    );
    for (row, migration) in upgraded_ledger.iter().zip(MIGRATOR.iter()) {
        assert!(
            row.version == migration.version
                && row.success
                && row.checksum.as_slice() == migration.checksum.as_ref(),
            "applied migration does not match this exact source"
        );
    }
    integrity(db.pool()).await;

    let mut connection = checked(db.pool().acquire().await, "acquire comparison connection");
    checked(
        sqlx::query("ATTACH DATABASE ?1 AS original_snapshot")
            .bind("file:/vendor/qualification/upgrade.sqlite?mode=ro&immutable=1")
            .execute(&mut *connection)
            .await,
        "attach read-only original snapshot",
    );
    let tables: Vec<String> = checked(
        sqlx::query_scalar(
            "SELECT name FROM original_snapshot.sqlite_schema
             WHERE type='table' AND name <> '_sqlx_migrations' ORDER BY name",
        )
        .fetch_all(&mut *connection)
        .await,
        "read original table inventory",
    );
    assert!(!tables.is_empty(), "private fixture has no content tables");
    for table in &tables {
        let name = identifier(table);
        let counts: (i64, i64) = checked(
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT (SELECT count(*) FROM main.{name}),
                        (SELECT count(*) FROM original_snapshot.{name})"
            )))
            .fetch_one(&mut *connection)
            .await,
            "compare existing table counts",
        );
        assert!(counts.0 == counts.1, "an existing table row count changed");
        for (left, right) in [("main", "original_snapshot"), ("original_snapshot", "main")] {
            let changed: i64 = checked(
                sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT EXISTS(SELECT * FROM {left}.{name}
                                   EXCEPT SELECT * FROM {right}.{name})"
                )))
                .fetch_one(&mut *connection)
                .await,
                "compare existing table content",
            );
            assert!(changed == 0, "existing table content changed during upgrade");
        }
    }

    type ForeignKeyFailure = (String, Option<i64>, String, i64);
    let mut previous_failures: Vec<ForeignKeyFailure> = checked(
        sqlx::query_as("PRAGMA original_snapshot.foreign_key_check")
            .fetch_all(&mut *connection)
            .await,
        "check original foreign keys",
    );
    let mut current_failures: Vec<ForeignKeyFailure> = checked(
        sqlx::query_as("PRAGMA main.foreign_key_check")
            .fetch_all(&mut *connection)
            .await,
        "check upgraded foreign keys",
    );
    previous_failures.sort();
    current_failures.sort();
    assert!(
        previous_failures == current_failures,
        "foreign-key integrity changed during upgrade"
    );
    checked(
        sqlx::query("DETACH DATABASE original_snapshot")
            .execute(&mut *connection)
            .await,
        "detach original snapshot",
    );
    drop(connection);
    db.close().await;

    let reopened = checked(
        Database::connect_sized(&url, Some(1)).await,
        "reopen upgraded private copy",
    );
    checked(reopened.run_migrations().await, "repeat upgraded migration");
    assert!(
        ledger(reopened.pool()).await == upgraded_ledger,
        "reopening changed migration receipts"
    );
    reopened.close().await;
    checked(temporary.close(), "remove private writable fixture");
}
