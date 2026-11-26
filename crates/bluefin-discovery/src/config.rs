use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, io::Write, path::Path};
use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewerGrant {
    pub user_id: Uuid,
    pub upstream_user_id: u32,
    #[serde(default)]
    pub search: bool,
    #[serde(default)]
    pub requests: bool,
}

/// Administrator input. Credentials are write-only and never returned.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveSettings {
    pub expected_revision: u64,
    pub enabled: bool,
    pub expose_to_clients: bool,
    pub base_url: Option<String>,
    pub allow_local_http: bool,
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_credential: bool,
    pub viewers: Vec<ViewerGrant>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    pub revision: u64,
    pub enabled: bool,
    pub expose_to_clients: bool,
    pub base_url: Option<String>,
    pub allow_local_http: bool,
    pub credential_configured: bool,
    pub viewers: Vec<ViewerGrant>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StoredSettings {
    pub revision: u64,
    pub enabled: bool,
    pub expose_to_clients: bool,
    pub base_url: Option<String>,
    pub allow_local_http: bool,
    pub api_key: Option<String>,
    pub viewers: Vec<ViewerGrant>,
}

impl StoredSettings {
    pub fn snapshot(&self) -> SettingsSnapshot {
        SettingsSnapshot {
            revision: self.revision,
            enabled: self.enabled,
            expose_to_clients: self.expose_to_clients,
            base_url: self.base_url.clone(),
            allow_local_http: self.allow_local_http,
            credential_configured: self.api_key.is_some(),
            viewers: self.viewers.clone(),
        }
    }

    pub fn replace(&self, input: SaveSettings) -> Result<Self> {
        if self.revision != input.expected_revision {
            return Err(Error::Conflict);
        }
        if input.clear_credential && input.api_key.is_some() {
            return Err(Error::Invalid);
        }
        let key = if input.clear_credential {
            None
        } else {
            input.api_key.or_else(|| self.api_key.clone())
        };
        let next = Self {
            revision: self.revision.checked_add(1).ok_or(Error::Conflict)?,
            enabled: input.enabled,
            expose_to_clients: input.expose_to_clients,
            base_url: input.base_url,
            allow_local_http: input.allow_local_http,
            api_key: key,
            viewers: input.viewers,
        };
        next.validate()?;
        Ok(next)
    }

    pub fn validate(&self) -> Result<()> {
        if self.viewers.len() > 1024 {
            return Err(Error::Invalid);
        }
        let mut identities = HashSet::new();
        for grant in &self.viewers {
            if grant.user_id.is_nil()
                || grant.upstream_user_id == 0
                || grant.upstream_user_id > i32::MAX as u32
                || !identities.insert(grant.user_id)
                || (grant.requests && !grant.search)
            {
                return Err(Error::Invalid);
            }
        }
        if let Some(key) = &self.api_key
            && (!(8..=512).contains(&key.len()) || !key.bytes().all(|b| b.is_ascii_graphic()))
        {
            return Err(Error::Invalid);
        }
        if let Some(base) = &self.base_url {
            validate_url(base, self.allow_local_http)?;
        }
        if self.enabled && (self.base_url.is_none() || self.api_key.is_none()) {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

pub(crate) fn validate_url(raw: &str, allow_local_http: bool) -> Result<Url> {
    if raw.len() > 2048 || raw.contains('\\') {
        return Err(Error::Invalid);
    }
    let url = Url::parse(raw).map_err(|_| Error::Invalid)?;
    let local = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback() || ip.is_private(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback() || ip.is_unique_local(),
        _ => false,
    };
    if url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "https" || (allow_local_http && local && url.scheme() == "http"))
        || url.path_segments().is_none_or(|mut parts| {
            parts.any(|p| {
                !p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            })
        })
    {
        return Err(Error::Invalid);
    }
    Ok(url)
}

pub(crate) fn load(path: &Path) -> Result<StoredSettings> {
    match fs::symlink_metadata(path) {
        Ok(info) if info.is_file() && info.len() <= 131_072 => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if info.permissions().mode() & 0o077 != 0 {
                    return Err(Error::Storage);
                }
            }
            let config: StoredSettings =
                serde_json::from_slice(&fs::read(path).map_err(|_| Error::Storage)?)
                    .map_err(|_| Error::Storage)?;
            config.validate().map_err(|_| Error::Storage)?;
            Ok(config)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(StoredSettings::default()),
        _ => Err(Error::Storage),
    }
}

pub(crate) fn persist(path: &Path, config: &StoredSettings) -> Result<bool> {
    let parent = path.parent().ok_or(Error::Storage)?;
    fs::create_dir_all(parent).map_err(|_| Error::Storage)?;
    if fs::symlink_metadata(path).is_ok_and(|m| !m.is_file()) {
        return Err(Error::Storage);
    }
    let temp = parent.join(format!(".bluefin-discovery-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp).map_err(|_| Error::Storage)?;
        let body = serde_json::to_vec(config).map_err(|_| Error::Storage)?;
        if body.len() > 131_072 {
            return Err(Error::Invalid);
        }
        file.write_all(&body)
            .and_then(|()| file.sync_all())
            .map_err(|_| Error::Storage)?;
        fs::rename(&temp, path).map_err(|_| Error::Storage)?;
        // Rename has committed the new file. Even if directory fsync fails,
        // publish it in memory before reporting uncertain durability to HTTP.
        #[cfg(unix)]
        let directory_synced = fs::File::open(parent).and_then(|f| f.sync_all()).is_ok();
        #[cfg(not(unix))]
        let directory_synced = true;
        Ok(directory_synced)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
