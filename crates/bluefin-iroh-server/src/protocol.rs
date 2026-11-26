//! Bounded one-request HTTP/1.1 framing; no forwarding or query credentials.
use anyhow::{Result, ensure};
use axum::{
    body::Body,
    http::{Request, header},
};
use std::collections::HashSet;
use tokio::io::{AsyncRead, AsyncReadExt};
pub(crate) const HEAD: usize = 16 * 1024;

pub(crate) async fn read<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(1024);
    loop {
        ensure!(bytes.len() < HEAD, "header bound");
        bytes.push(reader.read_u8().await?);
        if bytes.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    // Exactly one request and client FIN. Never reinterpret a body as another request.
    ensure!(
        reader.read(&mut [0]).await? == 0,
        "request body or pipeline"
    );
    Ok(bytes)
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Request<Body>> {
    let mut slots = [httparse::EMPTY_HEADER; 32];
    let mut raw = httparse::Request::new(&mut slots);
    ensure!(
        raw.parse(bytes)?.is_complete() && raw.version == Some(1),
        "HTTP version"
    );
    let method = raw.method.unwrap_or_default();
    ensure!(matches!(method, "GET" | "HEAD"), "media method");
    let path = raw.path.unwrap_or_default();
    ensure!(
        path.starts_with('/')
            && !path.starts_with("//")
            && path.len() <= 8192
            && path
                .bytes()
                .all(|b| (33..127).contains(&b) && !b"%\\#".contains(&b)),
        "media path"
    );
    ensure!(
        !path
            .split('?')
            .next()
            .unwrap_or_default()
            .split('/')
            .any(|s| matches!(s, "." | "..")),
        "path traversal"
    );
    if let Some((_, query)) = path.split_once('?') {
        ensure!(
            query.split('&').all(|pair| {
                let key = pair
                    .split('=')
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                ![
                    "api_key",
                    "apikey",
                    "access_token",
                    "token",
                    "authorization",
                    "x-emby-token",
                    "x-mediabrowser-token",
                ]
                .contains(&key.as_str())
            }),
            "query credentials"
        );
    }
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())?;
    let mut seen = HashSet::new();
    for h in raw.headers {
        let name = h.name.to_ascii_lowercase();
        ensure!(seen.insert(name.clone()), "duplicate header");
        let value = std::str::from_utf8(h.value)?;
        ensure!(
            value.len() <= 4096 && value.bytes().all(|b| (32..127).contains(&b)),
            "header value"
        );
        match name.as_str() {
            "host" => ensure!(value == "bluefin", "host"),
            "authorization" => {
                ensure!(value.starts_with("MediaBrowser "), "viewer authorization");
                request
                    .headers_mut()
                    .insert(header::AUTHORIZATION, value.parse()?);
            }
            "range" | "if-range" | "if-modified-since" | "if-none-match" | "accept" => {
                request.headers_mut().insert(
                    axum::http::HeaderName::from_bytes(name.as_bytes())?,
                    value.parse()?,
                );
            }
            "content-length" => ensure!(value == "0", "body"),
            "connection" => ensure!(value.eq_ignore_ascii_case("close"), "connection"),
            _ => anyhow::bail!("header is not allowed"),
        }
    }
    ensure!(seen.contains("host"), "host missing");
    request.headers_mut().insert(
        header::HOST,
        axum::http::HeaderValue::from_static("bluefin"),
    );
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_credential_and_forwarding_ambiguity() {
        for tail in [
            "X-Forwarded-For: 127.0.0.1\r\n",
            "X-Emby-Token: token\r\n",
            "Authorization: Bearer token\r\n",
            "Host: bluefin\r\n",
            "Transfer-Encoding: chunked\r\n",
        ] {
            assert!(
                parse(
                    format!("GET /Audio/a/stream HTTP/1.1\r\nHost: bluefin\r\n{tail}\r\n")
                        .as_bytes()
                )
                .is_err()
            );
        }
        for path in [
            "//host/stream",
            "/Audio/a/../stream",
            "/Audio/%61/stream",
            "/Audio/a/stream?ApiKey=secret",
            "/Audio/a/stream?api_key=secret",
        ] {
            assert!(
                parse(format!("GET {path} HTTP/1.1\r\nHost: bluefin\r\n\r\n").as_bytes()).is_err()
            );
        }
    }
}
