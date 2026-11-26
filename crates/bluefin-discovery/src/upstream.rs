//! Server-only service adapter. Raw provider responses never cross the API.
use crate::{
    Error, Result,
    config::{StoredSettings, ViewerGrant},
};
use reqwest::{Client, Method, header::HeaderValue};
use serde_json::{Value, json};
use std::time::Duration;
use url::Url;
use uuid::Uuid;

pub(crate) struct Upstream {
    client: Client,
}

impl Upstream {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(8))
                .connect_timeout(Duration::from_secs(3))
                .user_agent("Bluefin discovery/1")
                .build()
                .map_err(|_| Error::Upstream)?,
        })
    }

    async fn request(
        &self,
        config: &StoredSettings,
        grant: &ViewerGrant,
        method: Method,
        path: &[&str],
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Value> {
        let mut url = Url::parse(config.base_url.as_deref().ok_or(Error::Unavailable)?)
            .map_err(|_| Error::Unavailable)?;
        url.path_segments_mut()
            .map_err(|()| Error::Unavailable)?
            .pop_if_empty()
            .extend(["api", "v1"])
            .extend(path.iter().copied());
        let mut key = HeaderValue::from_str(config.api_key.as_deref().ok_or(Error::Unavailable)?)
            .map_err(|_| Error::Unavailable)?;
        key.set_sensitive(true);
        // Always select the server-mapped account. Omitting this header would
        // make the service key act as the original administrator.
        let mut user = HeaderValue::from_str(&grant.upstream_user_id.to_string())
            .map_err(|_| Error::Forbidden)?;
        user.set_sensitive(true);
        let mut request = self
            .client
            .request(method, url)
            .header("X-Api-Key", key)
            .header("X-Api-User", user)
            .query(query);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request.send().await.map_err(|_| Error::Upstream)?;
        match response.status().as_u16() {
            200..=299 => {}
            401 | 403 => return Err(Error::Forbidden),
            409 => return Err(Error::AlreadyRequested),
            _ => return Err(Error::Upstream),
        }
        const LIMIT: usize = 2 * 1024 * 1024;
        if response.content_length().is_some_and(|n| n > LIMIT as u64) {
            return Err(Error::Upstream);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Upstream)? {
            if chunk.len() > LIMIT.saturating_sub(bytes.len()) {
                return Err(Error::Upstream);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| Error::Upstream)
    }

    pub async fn identity(
        &self,
        config: &StoredSettings,
        grant: &ViewerGrant,
        viewer: Uuid,
        administrator: bool,
    ) -> Result<u64> {
        let user = self
            .request(config, grant, Method::GET, &["auth", "me"], &[], None)
            .await?;
        if user["id"].as_u64() != Some(u64::from(grant.upstream_user_id)) {
            return Err(Error::Forbidden);
        }
        if let Some(id) = user["jellyfinUserId"].as_str().filter(|id| !id.is_empty())
            && Uuid::parse_str(id).ok() != Some(viewer)
        {
            return Err(Error::Forbidden);
        }
        let permissions = user["permissions"].as_u64().ok_or(Error::Forbidden)?;
        if permissions & 2 != 0 && !administrator {
            return Err(Error::Forbidden);
        }
        Ok(permissions)
    }

    pub async fn search(
        &self,
        config: &StoredSettings,
        grant: &ViewerGrant,
        query: &str,
        page: u32,
    ) -> Result<Value> {
        self.request(
            config,
            grant,
            Method::GET,
            &["search"],
            &[("query", query.to_owned()), ("page", page.to_string())],
            None,
        )
        .await
    }

    pub async fn seasons(
        &self,
        config: &StoredSettings,
        grant: &ViewerGrant,
        id: u64,
    ) -> Result<Vec<u32>> {
        let details = self
            .request(
                config,
                grant,
                Method::GET,
                &["tv", &id.to_string()],
                &[],
                None,
            )
            .await?;
        let entries = details["seasons"].as_array().ok_or(Error::Upstream)?;
        if entries.len() > 256 {
            return Err(Error::Upstream);
        }
        let mut seasons: Vec<_> = entries
            .iter()
            .filter(|s| s["episodeCount"].as_u64().is_some_and(|n| n > 0))
            .filter_map(|s| {
                s["seasonNumber"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
            })
            .collect();
        seasons.sort_unstable();
        seasons.dedup();
        Ok(seasons)
    }

    pub async fn submit(
        &self,
        config: &StoredSettings,
        grant: &ViewerGrant,
        id: u64,
        movie: bool,
        seasons: Vec<u32>,
    ) -> Result<Value> {
        let mut body =
            json!({"mediaType": if movie { "movie" } else { "tv" }, "mediaId":id, "is4k":false});
        if !movie {
            body["seasons"] = json!(seasons);
        }
        // No userId, quota override, service, path or profile is accepted from clients.
        self.request(config, grant, Method::POST, &["request"], &[], Some(body))
            .await
    }
}

pub(crate) fn can_request(permissions: u64, movie: bool) -> bool {
    permissions & (2 | 32 | if movie { 262_144 } else { 524_288 }) != 0
}
