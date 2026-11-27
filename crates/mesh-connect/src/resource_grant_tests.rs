use super::*;
fn subject(n: u8) -> Subject { Subject { account: [n;32], endpoint: [n;32] } }
#[test]
fn pairwise_live_exact_scope_only() {
    let owner = subject(1);
    let peer = subject(2);
    let g = Grant { id: [4;32], issuer: owner, subject: peer, resource: [5;32],
        scope: Scope::StorageCapacity, generation: 1, not_before_ms: 10, expires_ms: 20 };
    let check = |record: Option<&Grant>, who, resource, scope, now, live|
        authorize(record, owner, who, resource, scope, now, live);
    let active = Liveness::Active { generation: 1 };
    assert!(check(Some(&g), peer, [5;32], g.scope, 10, active));
    assert!(!check(None, peer, [5;32], g.scope, 10, active));
    for who in [subject(3), Subject { endpoint: [3;32], ..peer }, Subject { account: [3;32], ..peer }] {
        assert!(!check(Some(&g), who, [5;32], g.scope, 10, active));
    }
    for scope in [Scope::MediaRead, Scope::CatalogRead] {
        assert!(!check(Some(&g), peer, [5;32], scope, 10, active));
    }
    for live in [Liveness::Unknown, Liveness::Revoked, Liveness::Active { generation: 2 }] {
        assert!(!check(Some(&g), peer, [5;32], g.scope, 10, live));
    }
    for now in [9, 20, u64::MAX] { assert!(!check(Some(&g), peer, [5;32], g.scope, now, active)); }
    assert!(!check(Some(&g), peer, [6;32], g.scope, 10, active));
    let reshared = Grant { issuer: peer, subject: subject(3), ..g };
    assert!(!check(Some(&reshared), subject(3), [5;32], g.scope, 10, active));
    let invalid = Grant { id: [0;32], ..g };
    assert!(!check(Some(&invalid), peer, [5;32], g.scope, 10, active));
}
