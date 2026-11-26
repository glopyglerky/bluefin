//! HTTPS launches carry the existing invitation in the fragment. The web origin
//! supplies browser trust; it does not replace the pinned application identity.
use crate::{ConnectInvite, Error, Result, MAX_CODE_BYTES};
use url::Url;

pub const WEB_FRAGMENT_PREFIX: &str = "bluefin-connect=";
pub const MAX_WEB_LAUNCH_BYTES: usize = MAX_CODE_BYTES + 1024;

/// Application/operator-selected HTTPS page, never taken from a relay hint.
#[derive(Clone)]
pub struct WebLaunchBase(Url);

impl WebLaunchBase {
    pub fn parse(input: &str) -> Result<Self> {
        if input.len() > 1024 { return Err(Error::TooLarge); }
        let url = Url::parse(input).map_err(|_| Error::InvalidFields)?;
        if url.scheme() != "https" || url.domain().is_none()
            || !url.username().is_empty() || url.password().is_some()
            || url.query().is_some() || url.fragment().is_some()
        {
            return Err(Error::InvalidFields);
        }
        Ok(Self(url))
    }

    /// Every representation spends the same capability at the live authority.
    pub fn invitation_url(&self, invite: &ConnectInvite) -> Result<String> {
        let mut url = self.0.clone();
        url.set_fragment(Some(&format!("{WEB_FRAGMENT_PREFIX}{}", invite.to_code()?)));
        let text = url.to_string();
        if text.len() > MAX_WEB_LAUNCH_BYTES { return Err(Error::TooLarge); }
        Ok(text)
    }
}

/// Extract locally without visiting the URL. A QR image or pasted launch URL
/// cannot make the native client contact its web origin.
pub fn parse_web_launch(input: &str) -> Result<ConnectInvite> {
    if input.len() > MAX_WEB_LAUNCH_BYTES { return Err(Error::TooLarge); }
    let mut url = Url::parse(input).map_err(|_| Error::InvalidCode)?;
    let code = url.fragment().and_then(|fragment| fragment.strip_prefix(WEB_FRAGMENT_PREFIX))
        .ok_or(Error::InvalidCode)?.to_owned();
    if code.len() > MAX_CODE_BYTES || code.contains(['%', '&', '#', '=', '?']) {
        return Err(Error::InvalidCode);
    }
    url.set_fragment(None);
    WebLaunchBase::parse(url.as_str())?;
    ConnectInvite::parse(&code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InviteIntent;

    fn invite() -> ConnectInvite {
        ConnectInvite {
            authority_id: [1; 32], authority_generation: 1, authority_epoch: [2; 32],
            endpoint_id: [3; 32], server_certificate_sha256: [4; 32],
            invitation_id: [5; 32], secret: [6; 32], expires_at_unix_ms: 2_000_000_000_000,
            service: "bluefin/connect/1".into(), intent: InviteIntent::Login,
            account_hint: None, relay_urls: vec!["https://relay.example/".into()],
            direct_addresses: vec![],
        }
    }

    #[test]
    fn launch_preserves_one_capability_and_keeps_secrets_out_of_http_request() {
        let original = invite();
        let text = WebLaunchBase::parse("https://app.example/web/bluefin/").unwrap()
            .invitation_url(&original).unwrap();
        let parsed = Url::parse(&text).unwrap();
        assert_eq!(parsed.path(), "/web/bluefin/");
        assert!(parsed.query().is_none());
        assert!(parsed.fragment().unwrap().starts_with(WEB_FRAGMENT_PREFIX));
        assert_eq!(ConnectInvite::parse(&text).unwrap(), original);
        assert_eq!(parse_web_launch(&text).unwrap(), original);
    }

    #[test]
    fn launch_refuses_credential_urls_http_queries_and_ambiguous_fragments() {
        for base in ["http://app.example/", "https://user@app.example/", "https://app.example/?code=x",
            "https://app.example/#x", "https://192.0.2.1/"] {
            assert!(WebLaunchBase::parse(base).is_err());
        }
        let url = WebLaunchBase::parse("https://app.example/").unwrap()
            .invitation_url(&invite()).unwrap();
        for extra in ["&next=https://other.example", "%0a", "#extra", "?x=1"] {
            assert!(parse_web_launch(&(url.clone() + extra)).is_err());
        }
        assert!(parse_web_launch("https://app.example/#account").is_err());
        assert!(parse_web_launch(&("https://app.example/#bluefin-connect=".to_owned()
            + &"A".repeat(MAX_CODE_BYTES + 1))).is_err());
    }
}
