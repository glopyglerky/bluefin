//! Durable, bounded Home signals owned by the existing Users database.
use crate::error::ApiError;
use ferrofin_db::{Database, store::guid_to_db};
use ferrofin_traits::error::ServiceError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub(super) const RETENTION_MS: i64 = 90 * 24 * 60 * 60 * 1000;
pub(super) const RETRY_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;
pub(super) const CLOCK_SKEW_MS: i64 = 5 * 60 * 1000;
pub(super) const MAX_EVENTS: i64 = 64;
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SearchEvent {
    pub(super) event_id: Uuid,
    pub(super) query: String,
    pub(super) selected_item_id: Option<Uuid>,
    pub(super) occurred_at_unix_ms: i64,
    pub(super) generation: i64,
}
impl SearchEvent {
    pub(super) fn validate(&mut self, now: i64) -> Result<(), ApiError> {
        if self.event_id.is_nil()
            || self.generation < 0
            || self.selected_item_id.is_some_and(|id| id.is_nil())
            || self.query.chars().any(char::is_control)
            || self.occurred_at_unix_ms < now.saturating_sub(RETRY_WINDOW_MS)
            || self.occurred_at_unix_ms > now.saturating_add(CLOCK_SKEW_MS)
        {
            return Err(ApiError::BadRequest("invalid search event".into()));
        }
        self.query = self.query.split_whitespace().collect::<Vec<_>>().join(" ");
        if !(2..=128).contains(&self.query.chars().count()) {
            return Err(ApiError::BadRequest(
                "search text must contain 2 to 128 characters".into(),
            ));
        }
        Ok(())
    }
}
#[derive(Serialize, Debug)]
pub(super) struct HistoryState {
    pub(super) generation: i64,
}

#[derive(sqlx::FromRow, Debug)]
#[sqlx(rename_all = "PascalCase")]
pub(super) struct StoredEvent {
    pub(super) event_id: String,
    pub(super) generation: i64,
    pub(super) term: String,
    pub(super) selected_item_id: Option<String>,
    pub(super) occurred_at_unix_ms: i64,
}

/// Bound to the same durable database as Users and UserData.
pub(crate) struct History {
    db: Database,
}
fn storage(_: sqlx::Error) -> ApiError {
    ServiceError::backend("account search history storage unavailable").into()
}
impl History {
    pub(crate) fn new(db: Database) -> Self {
        Self { db }
    }

    pub(super) async fn snapshot(
        &self,
        user: Uuid,
        now: i64,
    ) -> Result<(i64, Vec<StoredEvent>), ApiError> {
        // Physically retire expired rows whenever Home or search is used. The
        // read also excludes expired rows, including while another version runs.
        sqlx::query("DELETE FROM BluefinHomeSearches WHERE OccurredAtUnixMs < ?")
            .bind(now.saturating_sub(RETENTION_MS))
            .execute(self.db.writer())
            .await
            .map_err(storage)?;
        let mut tx = self.db.pool().begin().await.map_err(storage)?;
        let generation: Option<i64> =
            sqlx::query_scalar("SELECT Generation FROM BluefinHomeState WHERE UserId = ?")
                .bind(guid_to_db(user))
                .fetch_optional(&mut *tx)
                .await
                .map_err(storage)?;
        let generation = generation.unwrap_or(0);
        let rows = sqlx::query_as::<_, StoredEvent>(
            "SELECT EventId, Generation, Term, SelectedItemId, OccurredAtUnixMs FROM BluefinHomeSearches \
             WHERE UserId = ? AND Generation = ? AND OccurredAtUnixMs >= ? \
             ORDER BY OccurredAtUnixMs DESC, EventId LIMIT ?")
            .bind(guid_to_db(user)).bind(generation).bind(now.saturating_sub(RETENTION_MS)).bind(MAX_EVENTS)
            .fetch_all(&mut *tx).await.map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok((generation, rows))
    }

    pub(super) async fn record(
        &self,
        user: Uuid,
        mut event: SearchEvent,
        now: i64,
    ) -> Result<HistoryState, ApiError> {
        event.validate(now)?;
        let owner = guid_to_db(user);
        let mut tx = self
            .db
            .writer()
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage)?;
        sqlx::query("INSERT OR IGNORE INTO BluefinHomeState(UserId, Generation) VALUES (?, 0)")
            .bind(&owner)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        let generation: i64 =
            sqlx::query_scalar("SELECT Generation FROM BluefinHomeState WHERE UserId = ?")
                .bind(&owner)
                .fetch_one(&mut *tx)
                .await
                .map_err(storage)?;
        if generation != event.generation {
            return Err(ApiError::Conflict(
                "search history changed; refresh Home".into(),
            ));
        }
        let previous = sqlx::query_as::<_, StoredEvent>(
            "SELECT EventId, Generation, Term, SelectedItemId, OccurredAtUnixMs FROM BluefinHomeSearches WHERE UserId = ? AND EventId = ?")
            .bind(&owner).bind(guid_to_db(event.event_id)).fetch_optional(&mut *tx).await.map_err(storage)?;
        let selected = event.selected_item_id.map(guid_to_db);
        if let Some(previous) = previous {
            if previous.generation != generation
                || previous.term != event.query
                || previous.selected_item_id != selected
                || previous.occurred_at_unix_ms != event.occurred_at_unix_ms
            {
                return Err(ApiError::Conflict(
                    "search event identifier already used".into(),
                ));
            }
        } else {
            sqlx::query("INSERT INTO BluefinHomeSearches(UserId, EventId, Generation, Term, SelectedItemId, OccurredAtUnixMs) VALUES (?, ?, ?, ?, ?, ?)")
                .bind(&owner).bind(guid_to_db(event.event_id)).bind(generation).bind(&event.query)
                .bind(selected).bind(event.occurred_at_unix_ms)
                .execute(&mut *tx).await.map_err(storage)?;
        }
        sqlx::query("DELETE FROM BluefinHomeSearches WHERE OccurredAtUnixMs < ?")
            .bind(now.saturating_sub(RETENTION_MS))
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        sqlx::query("DELETE FROM BluefinHomeSearches WHERE UserId = ? AND EventId NOT IN \
            (SELECT EventId FROM BluefinHomeSearches WHERE UserId = ? ORDER BY OccurredAtUnixMs DESC, EventId LIMIT ?)")
            .bind(&owner).bind(&owner).bind(MAX_EVENTS).execute(&mut *tx).await.map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(HistoryState { generation })
    }

    pub(super) async fn clear(&self, user: Uuid, expected: i64) -> Result<HistoryState, ApiError> {
        if !(0..i64::MAX).contains(&expected) {
            return Err(ApiError::BadRequest("invalid history generation".into()));
        }
        let owner = guid_to_db(user);
        let mut tx = self
            .db
            .writer()
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage)?;
        sqlx::query("INSERT OR IGNORE INTO BluefinHomeState(UserId, Generation) VALUES (?, 0)")
            .bind(&owner)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        let current: i64 =
            sqlx::query_scalar("SELECT Generation FROM BluefinHomeState WHERE UserId = ?")
                .bind(&owner)
                .fetch_one(&mut *tx)
                .await
                .map_err(storage)?;
        // An exact retry acknowledges the completed clear without removing new
        // events another device has written since that clear.
        if current == expected + 1 {
            return Ok(HistoryState {
                generation: current,
            });
        }
        if current != expected {
            return Err(ApiError::Conflict(
                "search history changed; refresh Home".into(),
            ));
        }
        sqlx::query("DELETE FROM BluefinHomeSearches WHERE UserId = ?")
            .bind(&owner)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        sqlx::query("UPDATE BluefinHomeState SET Generation = Generation + 1 WHERE UserId = ?")
            .bind(&owner)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(HistoryState {
            generation: expected + 1,
        })
    }
}
