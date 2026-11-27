use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use bluefin_contract::public_catalog::{CatalogAccess, CatalogKind, CatalogOffer, CatalogOffers};
use reqwest::{Client, Url, header::HeaderValue};
use serde_json::Value;
use tokio::sync::Mutex;

const MAX_REPLY: usize = 2 * 1024 * 1024;
const MAX_CACHED: usize = 512;
const MAX_AGE: Duration = Duration::from_secs(1800);

pub(crate) struct StreamingAvailability {
    http: Client,
    key: HeaderValue,
    cache: Mutex<HashMap<(String, String), (Instant, CatalogOffers)>>,
}

impl StreamingAvailability {
    pub(crate) fn from_config() -> Option<Self> {
        let key = std::env::var("BLUEFIN_STREAMING_AVAILABILITY_KEY").ok()?;
        if key.trim().is_empty() {
            return None;
        }
        let mut key = HeaderValue::from_str(&key).ok()?;
        key.set_sensitive(true);
        let http = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(7))
            .build()
            .ok()?;
        Some(Self {
            http,
            key,
            cache: Mutex::new(HashMap::new()),
        })
    }

    pub(crate) async fn offers(
        &self,
        kind: CatalogKind,
        id: u64,
        country: &str,
    ) -> Option<CatalogOffers> {
        let kind = match kind {
            CatalogKind::Movie => "movie",
            CatalogKind::Series => "tv",
            _ => return None,
        };
        let key = (format!("{kind}/{id}"), country.to_owned());
        let mut cache = self.cache.lock().await;
        if let Some((_, result)) = cache.get(&key).filter(|(time, _)| time.elapsed() < MAX_AGE) {
            return Some(result.clone());
        }
        let mut url = Url::parse("https://api.movieofthenight.com/v4/shows/").ok()?;
        url.path_segments_mut().ok()?.pop_if_empty().push(&key.0);
        let mut response = self
            .http
            .get(url)
            .header("X-API-Key", self.key.clone())
            .query(&[("country", country), ("series_granularity", "show")])
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            if bytes.len().saturating_add(chunk.len()) > MAX_REPLY {
                return None;
            }
            bytes.extend(&chunk);
        }
        let data: Value = serde_json::from_slice(&bytes).ok()?;
        if data["tmdbId"].as_str() != Some(key.0.as_str()) {
            return None;
        }
        let rows = data["streamingOptions"].as_object()?;
        let offers = rows
            .get(country)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(256)
            .filter_map(offer)
            .collect();
        let result = CatalogOffers {
            country: country.to_owned(),
            offers,
        };
        cache.retain(|_, (observed, _)| observed.elapsed() < MAX_AGE);
        if cache.len() >= MAX_CACHED {
            if let Some(old) = cache
                .iter()
                .min_by_key(|(_, (time, _))| *time)
                .map(|(key, _)| key.clone())
            {
                cache.remove(&old);
            }
        }
        cache.insert(key, (Instant::now(), result.clone()));
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_offer_retains_its_exact_player_and_rejects_hops() {
        let mut record = json!({"service":{"name":"Netflix","homePage":"https://www.netflix.com"},
            "type":"subscription","link":"https://www.netflix.com/title/80057281",
            "videoLink":"https://www.netflix.com/watch/80057281"});
        let direct = offer(&record).unwrap();
        assert_eq!(
            direct.player_url.as_deref(),
            Some("https://www.netflix.com/watch/80057281")
        );
        record["videoLink"] = json!("https://www.justwatch.com/us/tv-show/stranger-things");
        assert_eq!(offer(&record).unwrap().player_url, None);
        record["link"] = json!("https://www.netflix.com/search?q=Stranger+Things");
        assert!(offer(&record).is_none());
    }
}

fn title_url(value: &Value, home: &Url) -> Option<String> {
    let link = value.as_str().filter(|url| url.len() <= 2048)?;
    let url = Url::parse(link).ok()?;
    let host = url.host_str()?;
    let provider = home
        .host_str()?
        .strip_prefix("www.")
        .unwrap_or(home.host_str()?);
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || matches!(
            provider,
            "tv.apple.com" | "justwatch.com" | "themoviedb.org" | "watchmode.com"
        )
        || !(host == provider
            || host
                .strip_suffix(provider)
                .is_some_and(|prefix| prefix.ends_with('.')))
        || url.path().trim_matches('/').is_empty()
        || url.path().trim_end_matches('/') == home.path().trim_end_matches('/')
        || url
            .path_segments()?
            .any(|section| matches!(section, "search" | "redirect" | "login"))
        || url.query_pairs().any(|(key, _)| {
            matches!(
                key.as_ref(),
                "redirect" | "redirect_uri" | "redirect_url" | "returnUrl"
            )
        })
    {
        return None;
    }
    Some(url.into())
}

fn offer(row: &Value) -> Option<CatalogOffer> {
    let service = &row["service"];
    let home = Url::parse(service["homePage"].as_str()?).ok()?;
    let provider = service["name"].as_str()?.trim();
    if provider.is_empty() || provider.len() > 120 || provider.chars().any(char::is_control) {
        return None;
    }
    let access = match row["type"].as_str()? {
        "free" => CatalogAccess::Free,
        "subscription" => CatalogAccess::Subscription,
        "buy" => CatalogAccess::Buy,
        "rent" => CatalogAccess::Rent,
        "addon" => CatalogAccess::Addon,
        _ => return None,
    };
    let title_url = title_url(&row["link"], &home)?;
    let player_url = self::title_url(&row["videoLink"], &home).filter(|url| *url != title_url);
    let price = row["price"]["formatted"]
        .as_str()
        .filter(|price| price.len() <= 48 && !price.chars().any(char::is_control))
        .map(str::to_owned);
    Some(CatalogOffer {
        provider: provider.to_owned(),
        access,
        title_url,
        player_url,
        price,
    })
}
