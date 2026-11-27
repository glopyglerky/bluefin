//! Disconnected authorization predicate for BDP-018 / SYSPLAN-027.
//!
//! No wire credentials, issuance, sign-in, or delegation are implemented here.
//! Caller MUST obtain the record and resource owner from trusted durable state,
//! and bind subject account + endpoint to the existing pinned PQ session. Never
//! populate these inputs from peer claims or a ConnectInvite. Check and execute
//! under the same revocation snapshot/transaction; a decision is not a token.
//! Unknown state denies. Catalog visibility requires its own explicit grant.
//!
//! Upstream study (2026-11-27): iroh-blobs 0.103.1 EventMask fix (#270)
//! requires per-operation checks; hashes/tickets are not user authorization.
//! https://www.iroh.computer/blog/iroh-blobs-push-fix
//! Gossip 0.102 broadcast membership is likewise not a resource entitlement:
//! https://www.iroh.computer/blog/iroh-gossip-road-to-1-0
//! Uses existing 32-byte identity representation; no crypto or transport change.
//! This allocation-free core uses only core language types; the containing crate
//! remains hosted and no no_std target qualification is claimed.

/// Account and authenticated endpoint are distinct, both must match exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Subject {
    pub account: [u8; 32],
    pub endpoint: [u8; 32],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope { StorageCapacity, MediaRead, CatalogRead }

/// Trusted durable owner record, never a bearer capsule. Pairwise only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grant {
    pub id: [u8; 32],
    pub issuer: Subject,
    pub subject: Subject,
    pub resource: [u8; 32],
    pub scope: Scope,
    pub generation: u64,
    pub not_before_ms: u64,
    pub expires_ms: u64,
}
/// Fresh durable lookup result. Missing/unavailable/revoked all deny.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Liveness { Unknown, Revoked, Active { generation: u64 } }

/// Evaluate one operation against one current record. There is no inheritance,
/// wildcard, group traversal, re-share, account login or implicit catalog access.
/// `owner` is the independent resource ownership lookup, never grant.issuer.
/// Time must come from the trusted owner; rollback handling belongs to that owner.
pub fn authorize(
    grant: Option<&Grant>, owner: Subject, peer: Subject,
    resource: [u8; 32], scope: Scope, now_ms: u64, live: Liveness,
) -> bool {
    let Some(g) = grant else { return false; };
    let valid_subject = |s: Subject| s.account != [0; 32] && s.endpoint != [0; 32];
    valid_subject(owner) && valid_subject(peer)
        && g.id != [0; 32] && resource != [0; 32]
        && g.issuer == owner && g.subject == peer
        && g.resource == resource && g.scope == scope
        && g.generation != 0
        && live == (Liveness::Active { generation: g.generation })
        && g.not_before_ms <= now_ms && now_ms < g.expires_ms
}

#[cfg(test)]
#[path = "resource_grant_tests.rs"]
mod tests;
