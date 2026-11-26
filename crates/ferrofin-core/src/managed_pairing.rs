//! Durable pairing. Database triggers own revocation even when minting is
//! interrupted after INSERT, or cancellation races a late device insertion.
use async_trait::async_trait;
use chrono::Utc;
use ferrofin_db::Database;
use ferrofin_model::secret::Secret;
use ferrofin_traits::{
    error::ServiceError,
    managed_pairing::{ManagedPairing, PairingChallenge, PairingProof, PairingStatus},
    options::AuthorizationInfo,
    session::{AuthenticationRequest, AuthenticationResultData, SessionManager},
};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

/// Bounded durable store; admission refuses when receipts fill it. No eviction
/// can turn an old cancelled challenge back into a valid challenge.
pub struct DurablePairing {
    db: Database,
    sessions: Arc<dyn SessionManager>,
    admission: tokio::sync::Mutex<()>,
    operations: tokio::sync::Semaphore,
    rate: std::sync::Mutex<(std::time::Instant, u32)>,
}
fn db_error(_: sqlx::Error) -> ServiceError {
    ServiceError::Backend("managed pairing persistence failed".into())
}
fn hash(secret: &str) -> String {
    use std::fmt::Write;
    Sha256::digest(secret.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}
fn busy() -> ServiceError {
    ServiceError::Busy {
        message: "managed pairing capacity reached".into(),
        retry_after_secs: 2,
    }
}
impl DurablePairing {
    /// Uses the same database and session manager as ordinary authentication.
    #[must_use]
    pub fn new(db: Database, sessions: Arc<dyn SessionManager>) -> Self {
        Self {
            db,
            sessions,
            admission: tokio::sync::Mutex::new(()),
            operations: tokio::sync::Semaphore::new(16),
            rate: std::sync::Mutex::new((std::time::Instant::now(), 0)),
        }
    }
    fn admit(&self) -> Result<tokio::sync::SemaphorePermit<'_>, ServiceError> {
        let mut rate = self.rate.try_lock().map_err(|_| busy())?;
        if rate.0.elapsed() >= std::time::Duration::from_secs(1) {
            *rate = (std::time::Instant::now(), 0);
        }
        if rate.1 >= 64 {
            return Err(busy());
        }
        rate.1 += 1;
        self.operations.try_acquire().map_err(|_| busy())
    }
    async fn row(&self, proof: &PairingProof) -> Result<sqlx::sqlite::SqliteRow, ServiceError> {
        if proof.challenge.len() != 32 || proof.secret.expose().len() != 64 {
            return Err(ServiceError::not_found("managed challenge"));
        }
        sqlx::query("SELECT * FROM FerrofinManagedPairing WHERE Id=? AND SecretHash=?")
            .bind(&proof.challenge)
            .bind(hash(proof.secret.expose()))
            .fetch_optional(self.db.pool())
            .await
            .map_err(db_error)?
            .ok_or_else(|| ServiceError::not_found("managed challenge"))
    }
    async fn expire(&self) -> Result<(), ServiceError> {
        sqlx::query("UPDATE FerrofinManagedPairing SET State='cancelled' WHERE Expires<=unixepoch() AND State IN ('pending','minting','authorized')")
            .execute(self.db.writer()).await.map_err(db_error)?;
        Ok(())
    }
    async fn cancel_row(&self, id: &str) -> Result<PairingStatus, ServiceError> {
        // Trigger deletion is in this same transaction/statement. Failed DELETE
        // rolls back the tombstone too; never return a false receipt.
        sqlx::query("UPDATE FerrofinManagedPairing SET State='cancelled' WHERE Id=?")
            .bind(id)
            .execute(self.db.writer())
            .await
            .map_err(db_error)?;
        Ok(PairingStatus {
            state: "cancelled".into(),
        })
    }
}
#[async_trait]
impl ManagedPairing for DurablePairing {
    async fn initiate(&self, auth: &AuthorizationInfo) -> Result<PairingChallenge, ServiceError> {
        let _permit = self.admit()?;
        let _gate = self.admission.try_lock().map_err(|_| busy())?;
        let field = |v: &Option<String>| {
            v.as_ref()
                .filter(|s| !s.is_empty() && s.len() <= 128)
                .cloned()
                .ok_or_else(|| {
                    ServiceError::InvalidInput("bounded device identity required".into())
                })
        };
        let device_id = field(&auth.device_id)?;
        let app = field(&auth.client)?;
        let version = field(&auth.version)?;
        let name = field(&auth.device)?;
        let ip = auth.remote_endpoint.as_deref().unwrap_or("unknown");
        self.expire().await?;
        let id = Uuid::new_v4().simple().to_string();
        let secret = Secret::from(format!(
            "{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        ));
        let now = Utc::now().timestamp();
        // Atomic bounded admission across instances, including retained receipts.
        let n=sqlx::query("INSERT INTO FerrofinManagedPairing(Id,SecretHash,DeviceId,ClientDeviceId,App,Version,DeviceName,RemoteIp,Created,Expires,State) SELECT ?,?,?,?,?,?,?,?,?,?,'pending' WHERE (SELECT COUNT(*) FROM FerrofinManagedPairing)<4096 AND (SELECT COUNT(*) FROM FerrofinManagedPairing WHERE RemoteIp=?)<64 AND NOT EXISTS(SELECT 1 FROM FerrofinManagedPairing WHERE RemoteIp=? AND Created>?)")
            .bind(&id).bind(hash(secret.expose())).bind(format!("bluefin-managed:{id}"))
            .bind(&device_id).bind(app).bind(version).bind(name).bind(ip).bind(now).bind(now+300)
            .bind(ip).bind(ip).bind(now-2).execute(self.db.writer()).await.map_err(db_error)?.rows_affected();
        if n != 1 {
            return Err(busy());
        }
        Ok(PairingChallenge {
            proof: PairingProof {
                challenge: id,
                secret,
            },
            device_id,
            expires_at: now + 300,
        })
    }
    async fn authorize(
        &self,
        proof: &PairingProof,
        device_id: &str,
        user: Uuid,
    ) -> Result<PairingStatus, ServiceError> {
        let _permit = self.admit()?;
        self.expire().await?;
        let row = self.row(proof).await?;
        if user.is_nil() || row.get::<String, _>("ClientDeviceId") != device_id {
            return Err(ServiceError::Forbidden("pairing identity mismatch".into()));
        }
        let changed=sqlx::query("UPDATE FerrofinManagedPairing SET State='minting',UserId=? WHERE Id=? AND State='pending' AND Expires>unixepoch()")
            .bind(user.to_string().to_uppercase()).bind(&proof.challenge).execute(self.db.writer()).await.map_err(db_error)?.rows_affected();
        if changed != 1 {
            return Err(ServiceError::Conflict("challenge cannot authorize".into()));
        }
        let result = self
            .sessions
            .authenticate_managed(&AuthenticationRequest {
                user_id: Some(user),
                device_id: Some(row.get("DeviceId")),
                app: Some(row.get("App")),
                app_version: Some(row.get("Version")),
                device_name: Some(row.get("DeviceName")),
                remote_endpoint: Some(row.get("RemoteIp")),
                ..Default::default()
            })
            .await;
        if let Err(error) = result {
            self.cancel_row(&proof.challenge).await?;
            return Err(error);
        }
        // A lost/dropped future is safe: the DB minted receipt survives; a
        // cancellation that won during the await remains terminal.
        let state = self.row(proof).await?.get("State");
        Ok(PairingStatus { state })
    }
    async fn status(&self, proof: &PairingProof) -> Result<PairingStatus, ServiceError> {
        let _permit = self.admit()?;
        self.expire().await?;
        Ok(PairingStatus {
            state: self.row(proof).await?.get("State"),
        })
    }
    async fn cancel(&self, proof: &PairingProof) -> Result<PairingStatus, ServiceError> {
        let _permit = self.admit()?;
        self.row(proof).await?;
        self.cancel_row(&proof.challenge).await
    }
    async fn exchange(
        &self,
        proof: &PairingProof,
    ) -> Result<AuthenticationResultData, ServiceError> {
        let _permit = self.admit()?;
        self.expire().await?;
        let row = self.row(proof).await?;
        if !matches!(
            row.get::<String, _>("State").as_str(),
            "authorized" | "exchanged"
        ) {
            return Err(ServiceError::Conflict("challenge not authorized".into()));
        }
        let changed = sqlx::query("UPDATE FerrofinManagedPairing SET State='exchanged' WHERE Id=? AND (State='exchanged' OR (State='authorized' AND Expires>unixepoch()))")
            .bind(&proof.challenge).execute(self.db.writer()).await.map_err(db_error)?.rows_affected();
        if changed != 1 {
            return Err(ServiceError::Conflict("challenge cannot exchange".into()));
        }
        let token: Option<String> =
            sqlx::query_scalar("SELECT AccessToken FROM Devices WHERE DeviceId=?")
                .bind(row.get::<String, _>("DeviceId"))
                .fetch_optional(self.db.pool())
                .await
                .map_err(db_error)?;
        let token = Secret::from(token.ok_or_else(|| {
            ServiceError::Conflict("pairing session unavailable; cancel challenge".into())
        })?);
        let session = self
            .sessions
            .get_session_by_authentication_token(
                token.expose(),
                &row.get::<String, _>("DeviceId"),
                &row.get::<String, _>("RemoteIp"),
            )
            .await?;
        Ok(AuthenticationResultData {
            session,
            access_token: token,
        })
    }
}
