//! Bluefin managed pairing v1; secrets are body-only capability credentials.
use crate::{error::ServiceError, options::AuthorizationInfo, session::AuthenticationResultData};
use async_trait::async_trait;
use ferrofin_model::secret::Secret;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Durable challenge credential. Never place it in a URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingProof {
    /// Public random challenge identity.
    pub challenge: String,
    /// Sensitive possession authority.
    pub secret: Secret,
}
/// Initiation response. Persist before requesting authorization.
#[derive(Debug, Serialize)]
pub struct PairingChallenge {
    /// Credential for status, exchange and cancellation.
    pub proof: PairingProof,
    /// Exact originating device identity to confirm on the phone.
    pub device_id: String,
    /// Unix expiry for authorization/exchange; cleanup survives it.
    pub expires_at: i64,
}
/// Cleanup proof is ONLY state == cancelled from a successful response.
#[derive(Debug, Serialize)]
pub struct PairingStatus {
    /// pending, minting, authorized or cancelled.
    pub state: String,
}
/// Optional capability, separate from legacy QuickConnect.
#[async_trait]
pub trait ManagedPairing: Send + Sync {
    /// Start with bounded admission.
    async fn initiate(&self, auth: &AuthorizationInfo) -> Result<PairingChallenge, ServiceError>;
    /// Authorize as the actual authenticated caller, checking exact device identity.
    async fn authorize(
        &self,
        proof: &PairingProof,
        device_id: &str,
        user: Uuid,
    ) -> Result<PairingStatus, ServiceError>;
    /// Read retained state (including after expiry/restart).
    async fn status(&self, proof: &PairingProof) -> Result<PairingStatus, ServiceError>;
    /// Exchange idempotently for the single persisted token.
    async fn exchange(
        &self,
        proof: &PairingProof,
    ) -> Result<AuthenticationResultData, ServiceError>;
    /// Durable fence plus revocation, or failure without cleanup acknowledgement.
    async fn cancel(&self, proof: &PairingProof) -> Result<PairingStatus, ServiceError>;
}
