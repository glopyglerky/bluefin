//! Private hosted adapter configuration; these values never enter catalog responses.
use crate::{Error, Result};
use bluefin_catalog::{Credentials, MovieOfTheNightCredential, TmdbCredential};
use serde::Deserialize;
use std::{fs, path::Path};
use url::{Host, Url};

/// Server-owned optional upstream credentials and personal per-title page access.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub(super) struct Settings {
    pub seerr_url: Option<String>,
    pub seerr_api_key: Option<String>,
    pub allow_local_http: bool,
    pub tmdb_api_key: Option<String>,
    pub tmdb_bearer: Option<String>,
    pub movie_of_the_night_api_key: Option<String>,
    pub rapid_api_key: Option<String>,
    pub disable_public_watch_pages: bool,
}

impl Settings {
    pub fn load(path: &Path) -> Result<Self> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(_) => return Err(Error::Storage),
        };
        if !metadata.is_file() || metadata.len() > 16_384 {
            return Err(Error::Storage);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(Error::Storage);
            }
        }
        let bytes = fs::read(path).map_err(|_| Error::Storage)?;
        let settings: Self = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
        if settings.tmdb_api_key.is_some() && settings.tmdb_bearer.is_some()
            || settings.movie_of_the_night_api_key.is_some() && settings.rapid_api_key.is_some()
            || settings.seerr_url.is_some() != settings.seerr_api_key.is_some()
        {
            return Err(Error::Invalid);
        }
        for key in [
            &settings.seerr_api_key, &settings.tmdb_api_key, &settings.tmdb_bearer,
            &settings.movie_of_the_night_api_key, &settings.rapid_api_key,
        ].into_iter().flatten() {
            if key.is_empty() || key.len() > 4096 || !key.bytes().all(|byte| byte.is_ascii_graphic()) {
                return Err(Error::Invalid);
            }
        }
        if let Some(value) = &settings.seerr_url {
            validate_service_url(value, settings.allow_local_http)?;
        }
        Ok(settings)
    }

    pub fn credentials(&self) -> Credentials {
        Credentials {
            tmdb: self.tmdb_bearer.clone().map(TmdbCredential::Bearer)
                .or_else(|| self.tmdb_api_key.clone().map(TmdbCredential::ApiKey)),
            movie_of_the_night: self.movie_of_the_night_api_key.clone()
                .map(MovieOfTheNightCredential::DirectApiKey)
                .or_else(|| self.rapid_api_key.clone().map(MovieOfTheNightCredential::RapidApiKey)),
        }
    }
}

pub(super) fn validate_service_url(value: &str, allow_local_http: bool) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| Error::Invalid)?;
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some()
        || url.fragment().is_some() || url.host().is_none()
        || url.path_segments().is_some_and(|mut parts| parts.any(|part|
            !part.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')))
    {
        return Err(Error::Invalid);
    }
    let local_http = allow_local_http && url.scheme() == "http" && match url.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback() || ip.is_private(),
        Some(Host::Ipv6(ip)) => ip.is_loopback() || ip.is_unique_local(),
        _ => false,
    };
    if url.scheme() != "https" && !local_http {
        return Err(Error::Invalid);
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_service_http_is_explicit_and_cannot_include_secrets_or_query() {
        assert!(validate_service_url("http://127.0.0.1:5055", false).is_err());
        assert!(validate_service_url("http://127.0.0.1:5055", true).is_ok());
        assert!(validate_service_url("http://example.com", true).is_err());
        assert!(validate_service_url("https://key@example.com", false).is_err());
        assert!(validate_service_url("https://example.com/?key=secret", false).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn readable_credentials_are_rejected_and_missing_configuration_is_optional() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("catalog.json");
        assert!(Settings::load(&path).is_ok());
        fs::write(&path, "{}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Settings::load(&path).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(Settings::load(&path).is_ok());
    }
}
