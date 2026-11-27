//! Private POST adapter for Iroh 1.4.1 `access.http`.
//!
//! Mount only on a private listener with a dedicated runtime-injected bearer.
//! Do not put this bearer in relay TOML, invitation capsules or request logs.
//! The account owner supplies leases only after checking its current ledger.
//! There is deliberately no public grant route here.
use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use bluefin_mesh_connect::relay_admission::{CalloutCredential, RelayAdmissions, parse_endpoint};
use std::{sync::Arc, time::Instant};
use tokio::sync::Mutex;

pub struct RelayCallout {
    pub admissions: Mutex<RelayAdmissions>,
    credential: CalloutCredential,
}
impl RelayCallout {
    /// The caller generates and supplies a dedicated 256-bit runtime bearer.
    pub fn new(
        relay_url: &str,
        token: &str,
    ) -> Result<Arc<Self>, bluefin_mesh_connect::relay_admission::AdmissionError> {
        Ok(Arc::new(Self {
            admissions: Mutex::new(RelayAdmissions::new(relay_url)?),
            credential: CalloutCredential::new(token)?,
        }))
    }
    pub fn router(self: Arc<Self>) -> Router {
        Router::new()
            .route("/relay/admission", post(check))
            .with_state(self)
    }
}

fn single<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    Some(value)
}

async fn check(
    State(state): State<Arc<RelayCallout>>,
    headers: HeaderMap,
) -> (StatusCode, &'static str) {
    let deny = (StatusCode::FORBIDDEN, "false");
    let Some(auth) = single(&headers, "authorization") else {
        return deny;
    };
    if !state.credential.verifies(auth) {
        return deny;
    }
    let Some(id) = single(&headers, "x-iroh-endpoint-id") else {
        return deny;
    };
    let Some(endpoint) = parse_endpoint(id) else {
        return deny;
    };
    // v1.4.1 sends both names. Never fall back to the deprecated alias, and
    // reject it if present but ambiguous or inconsistent.
    if headers.contains_key("x-iroh-nodeid") && single(&headers, "x-iroh-nodeid") != Some(id) {
        return deny;
    }
    if state
        .admissions
        .lock()
        .await
        .allows(&endpoint, Instant::now())
    {
        (StatusCode::OK, "true")
    } else {
        deny
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn pinned_http_callout_denies_unregistered_ambiguous_and_unauthenticated_peers() {
        let token = "ab".repeat(32);
        let state = RelayCallout::new("https://relay.example/", &token).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
        headers.insert("x-iroh-endpoint-id", "01".repeat(32).parse().unwrap());
        assert_eq!(
            check(State(state.clone()), headers.clone()).await,
            (StatusCode::FORBIDDEN, "false")
        );
        state
            .admissions
            .lock()
            .await
            .admit_listener([1; 32], Instant::now())
            .unwrap();
        assert_eq!(
            check(State(state.clone()), headers.clone()).await,
            (StatusCode::OK, "true")
        );
        headers.append("x-iroh-endpoint-id", "02".repeat(32).parse().unwrap());
        assert_eq!(
            check(State(state.clone()), headers.clone()).await.0,
            StatusCode::FORBIDDEN
        );
        headers.remove("x-iroh-endpoint-id");
        headers.insert("x-iroh-endpoint-id", "01".repeat(32).parse().unwrap());
        headers.insert("x-iroh-nodeid", "02".repeat(32).parse().unwrap());
        assert_eq!(
            check(State(state.clone()), headers.clone()).await.0,
            StatusCode::FORBIDDEN
        );
        headers.remove("x-iroh-nodeid");
        headers.remove("authorization");
        assert_eq!(
            check(State(state.clone()), headers.clone()).await.0,
            StatusCode::FORBIDDEN
        );
        headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
        state.admissions.lock().await.clear();
        assert_eq!(check(State(state), headers).await.0, StatusCode::FORBIDDEN);
    }
}
