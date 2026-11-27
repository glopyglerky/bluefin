//! Read-only public metadata fetched by this Bluefin instance.
use std::collections::HashSet;

use super::{Client, Duration, OnceLock, Read, Result, Url, Value, err, json, text};

const TV_CATALOG_LIMIT: u64 = 1024 * 1024;

/// Public lookup runs on this instance, without a peer origin or account token.
pub(super) fn public_tv_search(input: &Value) -> Result<Value> {
    let term = text(input, "query")?.trim();
    if term.chars().count() < 2 || term.chars().count() > 160 || term.chars().any(char::is_control) {
        return Err(err("Invalid public catalog query"));
    }
    static CLIENT: OnceLock<Result<Client>> = OnceLock::new();
    let client = CLIENT
        .get_or_init(|| {
            Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("Bluefin public catalog")
                .timeout(Duration::from_secs(6))
                .pool_max_idle_per_host(1)
                .build()
                .map_err(|_| err("Public catalog is unavailable"))
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let response = client
        .get("https://api.tvmaze.com/search/shows")
        .query(&[("q", term)])
        .send()
        .map_err(|_| err("Public TV catalog is unavailable"))?;
    if !response.status().is_success() {
        return Err(err("Public TV catalog is unavailable"));
    }
    let mut bytes = Vec::new();
    response
        .take(TV_CATALOG_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| err("Public TV catalog response could not be read"))?;
    if bytes.len() as u64 > TV_CATALOG_LIMIT {
        return Err(err("Public TV catalog response is too large"));
    }
    let rows: Vec<Value> = serde_json::from_slice(&bytes)
        .map_err(|_| err("Invalid public TV catalog response"))?;
    let mut seen = HashSet::new();
    let results: Vec<_> = rows
        .iter()
        .filter_map(public_tv_title)
        .filter(|title| title["id"].as_u64().is_some_and(|id| seen.insert(id)))
        .take(20)
        .collect();
    Ok(json!({"results": results}))
}

fn public_tv_title(row: &Value) -> Option<Value> {
    let show = row.get("show")?;
    let id = show["id"].as_u64().filter(|id| *id > 0)?;
    let title = show["name"].as_str()?.trim();
    if title.is_empty() || title.chars().count() > 256 || title.chars().any(char::is_control) {
        return None;
    }
    let year = show["premiered"]
        .as_str()
        .and_then(|date| date.get(..4))
        .filter(|year| year.bytes().all(|byte| byte.is_ascii_digit()));
    let artwork = show["image"]["medium"]
        .as_str()
        .filter(|value| value.len() <= 1024)
        .and_then(|value| Url::parse(value).ok())
        .filter(|url| {
            url.scheme() == "https"
                && url.host_str() == Some("static.tvmaze.com")
                && url.port().is_none()
                && url.username().is_empty()
                && url.password().is_none()
                && url.path().starts_with("/uploads/images/medium_portrait/")
                && url.query().is_none()
                && url.fragment().is_none()
        })
        .map(|url| url.to_string());
    Some(json!({
        "id": id,
        "title": title,
        "year": year,
        "artwork": artwork,
        "sourceUrl": format!("https://www.tvmaze.com/shows/{id}")
    }))
}

#[cfg(test)]
mod public_catalog_tests {
    use super::*;

    #[test]
    fn public_title_uses_the_provider_identity_and_rejects_unknown_artwork_origins() {
        let result = public_tv_title(&json!({"show": {"id": 47, "name": "A Stage",
            "url": "https://example.test/title", "premiered": "2025-04-01",
            "image": {"medium": "https://example.test/poster.jpg"}}})).unwrap();
        assert_eq!(result["sourceUrl"], "https://www.tvmaze.com/shows/47");
        assert_eq!(result["year"], "2025");
        assert_eq!(result["artwork"], Value::Null);
        assert!(public_tv_title(&json!({"show": {"id": 0, "name": "Invalid"}})).is_none());
    }
}
