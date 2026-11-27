//! Server-only service adapter. Raw provider responses never cross the API.
use crate::{
    Error, Result, Viewer,
    config::{StoredSettings, ViewerGrant},
};
use bluefin_contract::search::SearchScope;
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
        viewer: &Viewer,
        path: &[&str],
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Value> {
        let mut url = Url::parse(config.base_url.as_deref().ok_or(Error::Unavailable)?)
            .map_err(|_| Error::Unavailable)?;
        {
            let mut segments = url.path_segments_mut().map_err(|()| Error::Unavailable)?;
            segments.pop_if_empty();
            if config.session_catalog && path == ["music", "search"] {
                segments.extend(["JellyfinEnhanced", "jellyseerr"]);
            } else if config.session_catalog {
                segments.extend(["Moonfin", "Seerr", "Api"]);
            } else {
                segments.extend(["api", "v1"]);
            }
            segments.extend(path.iter().copied());
        }
        let method = if body.is_some() {
            Method::POST
        } else {
            Method::GET
        };
        let request = self.client.request(method, url).query(query);
        let mut request = if config.session_catalog {
            request.header(
                reqwest::header::AUTHORIZATION,
                viewer
                    .session_authorization
                    .clone()
                    .ok_or(Error::Forbidden)?,
            )
        } else {
            let mut key =
                HeaderValue::from_str(config.api_key.as_deref().ok_or(Error::Unavailable)?)
                    .map_err(|_| Error::Unavailable)?;
            key.set_sensitive(true);
            // Always select the server-mapped account. Omitting this header
            // would make the service key act as the original administrator.
            let mut user = HeaderValue::from_str(&grant.upstream_user_id.to_string())
                .map_err(|_| Error::Forbidden)?;
            user.set_sensitive(true);
            request.header("X-Api-Key", key).header("X-Api-User", user)
        };
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
        viewer: &Viewer,
    ) -> Result<u64> {
        let user = self
            .request(config, grant, viewer, &["auth", "me"], &[], None)
            .await?;
        if user["id"].as_u64() != Some(u64::from(grant.upstream_user_id)) {
            return Err(Error::Forbidden);
        }
        if let Some(id) = user["jellyfinUserId"].as_str().filter(|id| !id.is_empty())
            && Uuid::parse_str(id).ok() != Some(viewer.user_id)
        {
            return Err(Error::Forbidden);
        }
        let permissions = user["permissions"].as_u64().ok_or(Error::Forbidden)?;
        if permissions & 2 != 0 && !viewer.administrator {
            return Err(Error::Forbidden);
        }
        Ok(permissions)
    }

    pub async fn search(
        &self,
        config: &StoredSettings,
        grant: &ViewerGrant,
        viewer: &Viewer,
        query: &str,
        page: u32,
        scope: SearchScope,
    ) -> Result<Value> {
        let mut params = vec![("query", query.to_owned()), ("page", page.to_string())];
        if config.session_catalog {
            match scope {
                SearchScope::Movies => params.push(("type", "movie".into())),
                SearchScope::TvShows => params.push(("type", "tv".into())),
                _ => {}
            }
        }
        self.request(config, grant, viewer, &["search"], &params, None)
            .await
    }

    pub async fn seasons(
        &self,
        config: &StoredSettings,
        grant: &ViewerGrant,
        viewer: &Viewer,
        id: u64,
    ) -> Result<Vec<u32>> {
        let details = self
            .request(config, grant, viewer, &["tv", &id.to_string()], &[], None)
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

    pub async fn music(
        &self,
        config: &StoredSettings,
        grant: &ViewerGrant,
        viewer: &Viewer,
        query: &str,
    ) -> Result<Value> {
        self.request(
            config,
            grant,
            viewer,
            &["music", "search"],
            &[("query", query.to_owned())],
            None,
        )
        .await
    }

    pub async fn submit(
        &self,
        config: &StoredSettings,
        grant: &ViewerGrant,
        viewer: &Viewer,
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
        self.request(config, grant, viewer, &["request"], &[], Some(body))
            .await
    }
}

pub(crate) fn can_request(permissions: u64, movie: bool) -> bool {
    permissions & (2 | 32 | if movie { 262_144 } else { 524_288 }) != 0
}
