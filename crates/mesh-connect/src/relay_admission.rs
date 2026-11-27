//! Private Iroh 1.4.1 relay HTTP-callout admission, separate from account auth.
//!
//! The trusted account owner must check its live epoch, policy and exact stored
//! invitation before calling `admit_pairing`, while holding its issuance gate.
//! Decoding a capsule alone is never sufficient. This module does not consume
//! invitations: only the existing PQ/passkey account transaction does that.
//! Leases live in memory, disappear on restart and cannot exceed two minutes.
//! Iroh's HTTP hook checks connections only; expiry/revocation here refuses
//! future connections, and does not disconnect already admitted connections.
use crate::ConnectInvite;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;

pub const MAX_LEASE: Duration = Duration::from_secs(120);
pub const MAX_ENDPOINTS: usize = 1024;
pub const MAX_INVITE_ENDPOINTS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    Invalid,
    Full,
    InvitationLimit,
}

struct Lease {
    invitation: Option<[u8; 32]>,
    deadline: Instant,
}

/// Owned by one configured relay audience and one live account-service epoch.
/// Replace this store when that epoch changes. Never share across audiences.
pub struct RelayAdmissions {
    relay_url: String,
    leases: BTreeMap<[u8; 32], Lease>,
}

impl RelayAdmissions {
    /// Exact canonical HTTPS relay selected independently by its operator.
    pub fn new(relay_url: &str) -> Result<Self, AdmissionError> {
        let relay_url =
            crate::capsule::canonical_relay(relay_url).map_err(|_| AdmissionError::Invalid)?;
        Ok(Self {
            relay_url,
            leases: BTreeMap::new(),
        })
    }
    /// Trusted owner entry point, after checking the *current stored* invitation.
    /// `wall_now_ms` is the owner's time; `now` is a monotonic local clock.
    /// Repeat grants do not extend a live lease or change its invitation.
    pub fn admit_pairing(
        &mut self,
        invite: &ConnectInvite,
        endpoint: [u8; 32],
        wall_now_ms: u64,
        now: Instant,
    ) -> Result<(), AdmissionError> {
        invite
            .validate_at(wall_now_ms)
            .map_err(|_| AdmissionError::Invalid)?;
        if endpoint == [0; 32]
            || endpoint == invite.endpoint_id
            || !invite.relay_urls.contains(&self.relay_url)
        {
            return Err(AdmissionError::Invalid);
        }
        self.prune(now);
        if let Some(lease) = self.leases.get(&endpoint) {
            return if lease.invitation == Some(invite.invitation_id) {
                Ok(())
            } else {
                Err(AdmissionError::Invalid)
            };
        }
        if self
            .leases
            .values()
            .filter(|l| l.invitation == Some(invite.invitation_id))
            .count()
            >= MAX_INVITE_ENDPOINTS
        {
            return Err(AdmissionError::InvitationLimit);
        }
        let duration =
            Duration::from_millis(invite.expires_at_unix_ms - wall_now_ms).min(MAX_LEASE);
        self.insert(endpoint, Some(invite.invitation_id), now + duration)
    }

    /// Trusted listener owner renewal. Never expose this operation publicly.
    /// Listener identity is admitted separately so bootstrap never needs an
    /// already-online relay-backed invitation. Caller must still be live.
    pub fn admit_listener(
        &mut self,
        endpoint: [u8; 32],
        now: Instant,
    ) -> Result<(), AdmissionError> {
        if endpoint == [0; 32] {
            return Err(AdmissionError::Invalid);
        }
        self.prune(now);
        if self
            .leases
            .get(&endpoint)
            .is_some_and(|l| l.invitation.is_some())
        {
            return Err(AdmissionError::Invalid);
        }
        self.insert(endpoint, None, now + MAX_LEASE)
    }
    fn insert(
        &mut self,
        endpoint: [u8; 32],
        invitation: Option<[u8; 32]>,
        deadline: Instant,
    ) -> Result<(), AdmissionError> {
        if !self.leases.contains_key(&endpoint) && self.leases.len() >= MAX_ENDPOINTS {
            return Err(AdmissionError::Full);
        }
        self.leases.insert(
            endpoint,
            Lease {
                invitation,
                deadline,
            },
        );
        Ok(())
    }
    pub fn allows(&mut self, endpoint: &[u8; 32], now: Instant) -> bool {
        self.prune(now);
        self.leases.contains_key(endpoint)
    }
    pub fn revoke_invitation(&mut self, invitation: &[u8; 32]) {
        self.leases
            .retain(|_, l| l.invitation.as_ref() != Some(invitation));
    }
    pub fn clear(&mut self) {
        self.leases.clear();
    }
    fn prune(&mut self, now: Instant) {
        self.leases.retain(|_, l| now < l.deadline);
    }
}

/// Hash-only verifier for a dedicated, randomly generated 32-byte bearer,
/// encoded as 64 lowercase hexadecimal characters. Never serialize or log it.
pub struct CalloutCredential {
    digest: [u8; 32],
}
impl CalloutCredential {
    pub fn new(token: &str) -> Result<Self, AdmissionError> {
        if parse_endpoint(token).is_none() {
            return Err(AdmissionError::Invalid);
        }
        Ok(Self {
            digest: Sha256::digest(token.as_bytes()).into(),
        })
    }
    pub fn verifies(&self, authorization: &str) -> bool {
        let Some(token) = authorization.strip_prefix("Bearer ") else {
            return false;
        };
        if token.len() != 64 {
            return false;
        }
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        bool::from(self.digest.ct_eq(&digest))
    }
}

/// Iroh v1.4.1 EndpointId Display is exactly 64 lowercase hexadecimal bytes.
/// Reject alternative encodings and all-zero IDs at the callout boundary.
pub fn parse_endpoint(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut result = [0; 32];
    fn nibble(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            _ => None,
        }
    }
    for (out, pair) in result.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        *out = nibble(pair[0])? * 16 + nibble(pair[1])?;
    }
    (result != [0; 32]).then_some(result)
}
