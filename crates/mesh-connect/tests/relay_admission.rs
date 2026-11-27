use mesh_connect::{ConnectInvite, relay_admission::*};
use std::time::{Duration, Instant};

fn invite() -> ConnectInvite {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/invite-v1.json")).unwrap();
    serde_json::from_value(fixture["invite"].clone()).unwrap()
}

#[test]
fn expiry_is_monotonic_bounded_and_does_not_renew_on_retry() {
    let invite = invite();
    let now = Instant::now();
    let mut store = RelayAdmissions::new("https://relay.example/").unwrap();
    let wall = invite.expires_at_unix_ms - 3_600_000;
    store.admit_pairing(&invite, [9; 32], wall, now).unwrap();
    assert!(!store.allows(&[8; 32], now));
    assert!(store.allows(&[9; 32], now + MAX_LEASE - Duration::from_millis(1)));
    store
        .admit_pairing(
            &invite,
            [9; 32],
            wall + 119_000,
            now + Duration::from_secs(119),
        )
        .unwrap();
    assert!(!store.allows(&[9; 32], now + MAX_LEASE));
}

#[test]
fn invitation_deadline_wins_and_expired_capsules_are_refused() {
    let invite = invite();
    let now = Instant::now();
    let mut store = RelayAdmissions::new("https://relay.example/").unwrap();
    store
        .admit_pairing(&invite, [9; 32], invite.expires_at_unix_ms - 1, now)
        .unwrap();
    assert!(!store.allows(&[9; 32], now + Duration::from_millis(1)));
    assert_eq!(
        store.admit_pairing(&invite, [9; 32], invite.expires_at_unix_ms, now),
        Err(AdmissionError::Invalid)
    );
}

#[test]
fn invite_fanout_and_endpoint_rebinding_are_bounded() {
    let invite = invite();
    let now = Instant::now();
    let wall = invite.expires_at_unix_ms - 1000;
    let mut store = RelayAdmissions::new("https://relay.example/").unwrap();
    store.admit_pairing(&invite, [9; 32], wall, now).unwrap();
    store.admit_pairing(&invite, [8; 32], wall, now).unwrap();
    assert_eq!(
        store.admit_pairing(&invite, [7; 32], wall, now),
        Err(AdmissionError::InvitationLimit)
    );
    let mut other = invite.clone();
    other.invitation_id = [42; 32];
    assert_eq!(
        store.admit_pairing(&other, [9; 32], wall, now),
        Err(AdmissionError::Invalid)
    );
    assert_eq!(
        store.admit_listener([9; 32], now),
        Err(AdmissionError::Invalid)
    );
    store.revoke_invitation(&invite.invitation_id);
    assert!(!store.allows(&[9; 32], now));
    assert!(!store.allows(&[8; 32], now));
}

#[test]
fn capacity_recovers_after_expiry_and_epoch_reset() {
    let now = Instant::now();
    let mut store = RelayAdmissions::new("https://relay.example/").unwrap();
    for i in 1..=MAX_ENDPOINTS as u64 {
        let mut key = [1; 32];
        key[..8].copy_from_slice(&i.to_be_bytes());
        store.admit_listener(key, now).unwrap();
    }
    assert_eq!(
        store.admit_listener([2; 32], now),
        Err(AdmissionError::Full)
    );
    store.admit_listener([2; 32], now + MAX_LEASE).unwrap();
    assert!(store.allows(&[2; 32], now + MAX_LEASE));
    store.clear();
    assert!(!store.allows(&[2; 32], now + MAX_LEASE));
}

#[test]
fn malformed_ids_and_uncredentialed_callouts_are_refused() {
    let token = "ab".repeat(32); // Synthetic, never an active credential.
    let credential = CalloutCredential::new(&token).unwrap();
    assert!(credential.verifies(&format!("Bearer {token}")));
    for wrong in [
        token.clone(),
        format!("Bearer {}", "cd".repeat(32)),
        format!("Bearer {token} "),
    ] {
        assert!(!credential.verifies(&wrong));
    }
    assert!(CalloutCredential::new("").is_err());
    assert!(CalloutCredential::new(&"00".repeat(32)).is_err());
    for bad in [
        "00".repeat(32),
        "AB".repeat(32),
        "g1".repeat(32),
        "01".repeat(31),
    ] {
        assert!(parse_endpoint(&bad).is_none());
    }
    assert_eq!(parse_endpoint(&"01".repeat(32)), Some([1; 32]));
}

#[test]
fn pairing_cannot_replace_listener_or_admit_invalid_routes() {
    let mut invite = invite();
    let now = Instant::now();
    let wall = invite.expires_at_unix_ms - 1000;
    let mut store = RelayAdmissions::new("https://relay.example/").unwrap();
    let mut other_relay = RelayAdmissions::new("https://other-relay.example/").unwrap();
    assert_eq!(
        other_relay.admit_pairing(&invite, [8; 32], wall, now),
        Err(AdmissionError::Invalid)
    );
    store.admit_listener([9; 32], now).unwrap();
    assert_eq!(
        store.admit_pairing(&invite, [9; 32], wall, now),
        Err(AdmissionError::Invalid)
    );
    assert_eq!(
        store.admit_pairing(&invite, invite.endpoint_id, wall, now),
        Err(AdmissionError::Invalid)
    );
    assert_eq!(
        store.admit_pairing(&invite, [0; 32], wall, now),
        Err(AdmissionError::Invalid)
    );
    invite.relay_urls.clear();
    assert_eq!(
        store.admit_pairing(&invite, [8; 32], wall, now),
        Err(AdmissionError::Invalid)
    );
}
