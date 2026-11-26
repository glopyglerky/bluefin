//! Short, consistent source-generation capture. No schema or persistent leases.
use std::fs::File;
use std::sync::Arc;

use tokio::sync::OwnedSemaphorePermit;
use uuid::Uuid;

use crate::entities::base_items::{
    AttachmentStreamInfoEntity, BaseItemEntity, MediaStreamInfoEntity,
};
use crate::{Database, DbError, Result, store::guid_to_db};

/// Maximum simultaneous opened inputs per database owner, shared by its clones.
pub const LOCAL_SOURCE_PIN_LIMIT: usize = 32;

/// A consistent row generation and an owned open input, captured together.
///
/// This is a logical database generation and stable opened file identity, not
/// immutable file contents. Dropping it closes the descriptor and releases the
/// database owner's admission permit synchronously, including after cancellation.
/// No SQLite transaction, database lease row or background cleanup survives capture.
pub struct OpenedSourceGeneration {
    /// The requested item's row at capture.
    pub item: BaseItemEntity,
    /// The selected source's row at capture.
    pub source: BaseItemEntity,
    /// The bounded version component at capture, for the production group rules.
    pub versions: Vec<BaseItemEntity>,
    /// Captured authorization ancestry, bounded to 128 rows.
    pub ancestry: Vec<BaseItemEntity>,
    /// Captured collection-folder declarations, bounded to 128 rows.
    pub libraries: Vec<BaseItemEntity>,
    /// The selected source's stream generation.
    pub streams: Vec<MediaStreamInfoEntity>,
    /// The selected source's attachment generation.
    pub attachments: Vec<AttachmentStreamInfoEntity>,
    input: File,
    database: Database,
    _permit: OwnedSemaphorePermit,
}

impl OpenedSourceGeneration {
    /// Server-process descriptor path. Keep this owner alive through every open
    /// by a direct reader or child encoder. Never return this path to clients.
    ///
    /// # Errors
    /// Refuses platforms without the qualified Linux proc-descriptor route.
    pub fn input_path(&self) -> Result<String> {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            Ok(format!(
                "/proc/{}/fd/{}",
                std::process::id(),
                self.input.as_raw_fd()
            ))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = &self.input;
            Err(unavailable(
                "opened source inputs are unsupported on this platform",
            ))
        }
    }

    /// Refresh the policy/library reads used by the existing visibility evaluator.
    /// Existing caches otherwise permit an old independent writer's revocation
    /// to remain cached for their normal TTL. This takes no database lock.
    pub fn refresh_authorization(&self, user_id: Uuid) {
        self.database.invalidate_user_policy(&guid_to_db(user_id));
        self.database.invalidate_libraries();
    }

    /// The database that owns this input, for current account/schedule checks.
    #[must_use]
    pub fn database(&self) -> &Database {
        &self.database
    }

    /// Compare today's logical generation and opened file identity to this
    /// retained input. This intentionally does not assert byte immutability.
    /// A consumer must hash/copy and rehash the returned current descriptor.
    ///
    /// # Errors
    /// Refuses every changed item/version/stream/attachment or file identity.
    pub async fn current_input(&self) -> Result<File> {
        let item = Uuid::parse_str(&self.item.id).map_err(|_| unavailable("invalid item"))?;
        let source = Uuid::parse_str(&self.source.id).map_err(|_| unavailable("invalid source"))?;
        let now = self
            .database
            .capture_local_source(
                item,
                source,
                self.source
                    .path
                    .as_deref()
                    .ok_or_else(|| unavailable("source path absent"))?,
            )
            .await?;
        if self.item != now.item
            || self.source != now.source
            || !same_rows(&self.versions, &now.versions)
            || !same_rows(&self.ancestry, &now.ancestry)
            || !same_rows(&self.libraries, &now.libraries)
            || self.streams != now.streams
            || self.attachments != now.attachments
        {
            return Err(unavailable("source logical generation changed"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let before = self
                .input
                .metadata()
                .map_err(|_| unavailable("source inspection failed"))?;
            let after = now
                .input
                .metadata()
                .map_err(|_| unavailable("source inspection failed"))?;
            if before.dev() != after.dev() || before.ino() != after.ino() {
                return Err(unavailable("source opened identity changed"));
            }
        }
        now.input
            .try_clone()
            .map_err(|_| unavailable("source descriptor unavailable"))
    }
}

fn same_rows(a: &[BaseItemEntity], b: &[BaseItemEntity]) -> bool {
    let mut a: Vec<_> = a.iter().collect();
    let mut b: Vec<_> = b.iter().collect();
    a.sort_by(|a, b| a.id.cmp(&b.id));
    b.sort_by(|a, b| a.id.cmp(&b.id));
    a == b
}

fn unavailable(message: &'static str) -> DbError {
    DbError::Sqlx(sqlx::Error::Protocol(message.into()))
}

impl Database {
    /// Compare and capture one source in a short admission transaction on this
    /// database, opening its exact local path before finishing the snapshot.
    /// Old writers and independent handles remain compatible. A consumer must
    /// use the captured input, never re-resolve the stored path after capture.
    ///
    /// The capture has a two-second deadline, with no SQLite busy wait, and
    /// admits at most 64 versions, 128 ancestry rows, 128 libraries, 256 streams
    /// and 64 attachments. Cancellation rolls back the admission transaction.
    /// Blocking opens retain admission until their actual syscall finishes.
    /// The media-source owner must verify group membership, local finite media
    /// semantics and fresh authorization before exposing the captured input.
    ///
    /// # Errors
    /// Refuses exhausted admission, a missing/mismatched source, excessive rows,
    /// unsupported platforms, non-regular inputs and database or I/O failures.
    pub async fn capture_local_source(
        &self,
        item_id: Uuid,
        source_id: Uuid,
        expected_path: &str,
    ) -> Result<OpenedSourceGeneration> {
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            self.capture_local_source_inner(item_id, source_id, expected_path),
        )
        .await
        .map_err(|_| unavailable("local source capture timed out"))?
    }

    async fn capture_local_source_inner(
        &self,
        item_id: Uuid,
        source_id: Uuid,
        expected_path: &str,
    ) -> Result<OpenedSourceGeneration> {
        let permit = Arc::clone(&self.source_pins)
            .try_acquire_owned()
            .map_err(|_| unavailable("local source admission is full"))?;
        // Serialize the compare/open point with every SQLite writer, including
        // old binaries and separately opened handles. Release before returning.
        let mut tx = self.source_capture.begin_with("BEGIN IMMEDIATE").await?;
        // UNION deduplicates cycles; LIMIT bounds corrupt or oversized groups.
        let versions: Vec<BaseItemEntity> = sqlx::query_as(
            r#"
            WITH RECURSIVE reach("Id") AS (
                VALUES (?1)
                UNION
                SELECT b."Id" FROM "BaseItems" b JOIN reach r ON b."PrimaryVersionId"=r."Id"
                UNION
                SELECT b."PrimaryVersionId" FROM "BaseItems" b JOIN reach r ON b."Id"=r."Id"
                WHERE b."PrimaryVersionId" IS NOT NULL
                LIMIT 65
            ) SELECT * FROM "BaseItems" WHERE "Id" IN (SELECT "Id" FROM reach) LIMIT 65"#,
        )
        .bind(guid_to_db(item_id))
        .fetch_all(&mut *tx)
        .await?;
        if versions.len() > 64 {
            return Err(unavailable("local source version limit exceeded"));
        }
        let row = |id| {
            versions
                .iter()
                .find(|row| Uuid::parse_str(&row.id).ok() == Some(id))
                .cloned()
        };
        let item = row(item_id).ok_or_else(|| unavailable("local source item is absent"))?;
        let source = row(source_id).ok_or_else(|| unavailable("local source version is absent"))?;
        if source.path.as_deref() != Some(expected_path)
            || !std::path::Path::new(expected_path).is_absolute()
        {
            return Err(unavailable("local source generation changed"));
        }
        let ancestry: Vec<BaseItemEntity> = sqlx::query_as(r#"
            WITH RECURSIVE reach("Id") AS (
                VALUES (?1), (?2)
                UNION
                SELECT ref.value FROM reach JOIN "BaseItems" b ON b."Id"=reach."Id"
                CROSS JOIN json_each(json_array(b."ParentId",b."OwnerId",b."ChannelId",b."SeasonId",b."SeriesId",b."TopParentId")) ref
                WHERE ref.value IS NOT NULL LIMIT 129
            ) SELECT * FROM "BaseItems" WHERE "Id" IN (SELECT "Id" FROM reach) LIMIT 129"#)
            .bind(guid_to_db(item_id)).bind(guid_to_db(source_id)).fetch_all(&mut *tx).await?;
        let libraries: Vec<BaseItemEntity> =
            sqlx::query_as(r#"SELECT * FROM "BaseItems" WHERE "Type"=?1 LIMIT 129"#)
                .bind(
                    ferrofin_model::data::BaseItemKind::CollectionFolder
                        .stored_type_name()
                        .unwrap_or_default(),
                )
                .fetch_all(&mut *tx)
                .await?;
        if ancestry.len() > 128 || libraries.len() > 128 {
            return Err(unavailable(
                "local source authorization scope limit exceeded",
            ));
        }
        let streams: Vec<MediaStreamInfoEntity> = sqlx::query_as(
            r#"SELECT * FROM "MediaStreamInfos" WHERE "ItemId"=?1 ORDER BY "StreamIndex" LIMIT 257"#)
            .bind(guid_to_db(source_id)).fetch_all(&mut *tx).await?;
        let attachments: Vec<AttachmentStreamInfoEntity> = sqlx::query_as(
            r#"SELECT * FROM "AttachmentStreamInfos" WHERE "ItemId"=?1 ORDER BY "Index" LIMIT 65"#,
        )
        .bind(guid_to_db(source_id))
        .fetch_all(&mut *tx)
        .await?;
        if streams.len() > 256 || attachments.len() > 64 {
            return Err(unavailable("local source stream limit exceeded"));
        }
        let path = expected_path.to_owned();
        // A canceled blocking open must still count against admission until
        // the syscall finishes. Otherwise repeated timeouts could queue an
        // unbounded number of abandoned opens. The closure never owns the SQL
        // transaction, so cancellation releases that independently.
        let (input, permit) =
            tokio::task::spawn_blocking(move || open_input(&path).map(|input| (input, permit)))
                .await
                .map_err(|_| unavailable("local source open failed"))??;
        tx.commit().await?;
        Ok(OpenedSourceGeneration {
            item,
            source,
            versions,
            ancestry,
            libraries,
            streams,
            attachments,
            input,
            database: self.clone(),
            _permit: permit,
        })
    }
}

#[cfg(target_os = "linux")]
fn open_input(path: &str) -> Result<File> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    // A FIFO or device must never stall admission. O_NONBLOCK has no effect
    // on normal files; fstat below admits only a nonempty regular file.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| unavailable("local source cannot be opened"))?;
    let metadata = file
        .metadata()
        .map_err(|_| unavailable("local source cannot be inspected"))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(unavailable("local source is not a finite regular file"));
    }
    let transport = format!("/proc/{}/fd/{}", std::process::id(), file.as_raw_fd());
    let reopened = std::fs::metadata(transport)
        .map_err(|_| unavailable("local input descriptor transport is unavailable"))?;
    if metadata.dev() != reopened.dev() || metadata.ino() != reopened.ino() {
        return Err(unavailable("local input descriptor transport changed"));
    }
    Ok(file)
}

#[cfg(not(target_os = "linux"))]
fn open_input(_path: &str) -> Result<File> {
    Err(unavailable(
        "opened source inputs are unsupported on this platform",
    ))
}
