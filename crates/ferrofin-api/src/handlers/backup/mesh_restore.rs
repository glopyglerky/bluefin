//! Restored mesh grants require a new owner-approved pairing.
//!
//! This runs on the backup owner's staged file before any live file is replaced.
//! It is deliberately not a startup epoch: an ordinary restart keeps sessions.
//! Filesystem rollback outside this restore path still needs a surviving recovery
//! authority and must not be represented as qualified by this hook.

use std::io::Read as _;
use std::path::Path;

use sqlx::Connection as _;
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqliteSynchronous};

/// Removes mesh authority from a staged backup, before the backup owner swaps it in.
///
/// The caller already runs on the blocking restore lane. SQLx owns its SQLite
/// worker; this small local runtime waits for that worker and closes it before
/// returning. No live database handle or authority store is opened here.
pub(super) fn invalidate_staged(path: &Path) -> std::io::Result<()> {
    let mut header = [0; 16];
    match std::fs::File::open(path)?.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
        Err(error) => return Err(error),
    }
    // Non-SQLite corruption remains the existing database opener's concern.
    // It cannot contain usable SQLite grants.
    if &header != b"SQLite format 3\0" {
        return Ok(());
    }
    // The backup owner has just replaced this staged file's bytes. Sidecars
    // from an interrupted older staging attempt must not replay over them.
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        match std::fs::remove_file(name) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(invalidate_sqlite(path))?;
    std::fs::File::open(path)?.sync_all()
}

async fn invalidate_sqlite(path: &Path) -> std::io::Result<()> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Delete)
        .synchronous(SqliteSynchronous::Full);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(std::io::Error::other)?;
    let result = clear_mesh_grants(&mut connection).await;
    // A failed transaction is rolled back when its connection is closed.
    // Close before the staged file is discarded or moved by the backup owner.
    let closed = connection.close().await.map_err(std::io::Error::other);
    result?;
    closed
}

async fn clear_mesh_grants(connection: &mut SqliteConnection) -> std::io::Result<()> {
    const TABLES: [&str; 5] = [
        "BluefinMeshAuthority",
        "BluefinMeshInvites",
        "BluefinMeshSignups",
        "BluefinMeshSessions",
        "BluefinMeshReceipts",
    ];
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type='table' AND name IN \
         ('BluefinMeshAuthority','BluefinMeshInvites','BluefinMeshSignups',\
          'BluefinMeshSessions','BluefinMeshReceipts')",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(std::io::Error::other)?;
    if tables.is_empty() {
        return Ok(());
    }
    if TABLES.iter().any(|name| !tables.iter().any(|found| found.as_str() == *name)) {
        return Err(std::io::Error::other(
            "restored mesh authority has an incomplete schema",
        ));
    }
    let mut transaction = connection
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(std::io::Error::other)?;
    // Remove portable HTTP bearers as well as mesh session IDs. Do this while
    // the restored session linkage still exists. Unrelated device tokens and
    // account/passkey rows retain the existing backup contract.
    for statement in [
        "DELETE FROM Devices WHERE substr(DeviceId,1,13)='bluefin-mesh:'",
        "DELETE FROM BluefinMeshSignups",
        "DELETE FROM BluefinMeshReceipts",
        "DELETE FROM BluefinMeshSessions",
        "DELETE FROM BluefinMeshInvites",
        "DELETE FROM BluefinMeshAuthority",
    ] {
        sqlx::query(statement)
            .execute(&mut *transaction)
            .await
            .map_err(std::io::Error::other)?;
    }
    transaction.commit().await.map_err(std::io::Error::other)
}

#[cfg(test)]
#[path = "mesh_restore_tests.rs"]
mod tests;
