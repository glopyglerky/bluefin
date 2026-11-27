//! Bounded public metadata reads shared by the configured provider clients.
use std::{sync::OnceLock, time::Duration};

use crate::rate_limit::{BodyError, CappedBody, RequestError};

const MAX_DOCUMENT_BYTES: u64 = 2 * 1024 * 1024;

/// A source failure is different from a successful search with no matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CatalogFetchError {
    #[error("catalog source is unavailable")]
    Unavailable,
    #[error("catalog source is rate limited")]
    RateLimited,
    #[error("catalog source denied access")]
    Denied,
    #[error("catalog identity was not found")]
    NotFound,
    #[error("catalog source returned invalid metadata")]
    InvalidResponse,
    #[error("unsupported catalog identity")]
    UnsupportedIdentity,
}

impl From<RequestError> for CatalogFetchError {
    fn from(value: RequestError) -> Self {
        match value {
            RequestError::Cooldown => Self::RateLimited,
            RequestError::Http(_) => Self::Unavailable,
            RequestError::NotReplayable => Self::InvalidResponse,
        }
    }
}

impl From<BodyError> for CatalogFetchError {
    fn from(value: BodyError) -> Self {
        match value {
            BodyError::Read(_) => Self::Unavailable,
            BodyError::TooLarge => Self::InvalidResponse,
        }
    }
}

/// Public catalog traffic shares configured credentials and quotas. Its transport
/// refuses redirects and ambient proxies so source origins stay explicit.
pub(crate) fn client() -> Result<&'static reqwest::Client, CatalogFetchError> {
    static CLIENT: OnceLock<Result<reqwest::Client, reqwest::Error>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(3))
                .timeout(Duration::from_secs(8))
                .pool_max_idle_per_host(1)
                .build()
        })
        .as_ref()
        .map_err(|_| CatalogFetchError::Unavailable)
}

pub(crate) async fn json(
    response: reqwest::Response,
) -> Result<serde_json::Value, CatalogFetchError> {
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            401 | 403 => CatalogFetchError::Denied,
            404 => CatalogFetchError::NotFound,
            429 => CatalogFetchError::RateLimited,
            300..=399 | 400 | 422 => CatalogFetchError::InvalidResponse,
            _ => CatalogFetchError::Unavailable,
        });
    }
    let bytes = CappedBody::new(response, MAX_DOCUMENT_BYTES)?
        .read_to_end(Vec::new())
        .await?;
    serde_json::from_slice(&bytes).map_err(|_| {
        crate::rate_limit::note_request_failure();
        CatalogFetchError::InvalidResponse
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock_http::MockServer;

    #[tokio::test]
    async fn public_catalog_redirects_are_rejected_without_contacting_the_destination() {
        let destination = MockServer::start_raw(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
        ]).await;
        let redirect = MockServer::start_raw(vec![
            format!("HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\n\r\n", destination.base_url).into_bytes(),
        ]).await;
        let response = client().unwrap().get(&redirect.base_url).send().await.unwrap();
        assert_eq!(json(response).await, Err(CatalogFetchError::InvalidResponse));
        assert_eq!(redirect.requests(), 1);
        assert_eq!(destination.requests(), 0);
    }

    #[tokio::test]
    async fn public_catalog_document_limit_covers_declared_and_chunked_bodies() {
        let oversized = "x".repeat(MAX_DOCUMENT_BYTES as usize + 1);
        let responses = vec![
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", oversized.len()).into_bytes(),
            format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{oversized}\r\n0\r\n\r\n", oversized.len()).into_bytes(),
        ];
        let server = MockServer::start_raw(responses).await;
        for _ in 0..2 {
            let response = client().unwrap().get(&server.base_url).send().await.unwrap();
            assert_eq!(json(response).await, Err(CatalogFetchError::InvalidResponse));
        }
        assert_eq!(server.requests(), 2);
    }
}
