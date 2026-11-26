//! Validate platform browser continuations against the authenticated ceremony.
//! Opening a URL or receiving a callback grants no authority. Finish is still
//! signed by the original device and accepted by the live account owner.
use super::{err, text, Result};
use serde_json::{json, Value};
use url::Url;

fn identifier(value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(err("Invalid passkey continuation."));
    }
    Ok(())
}

fn browser_url(origin: &str, challenge: &str, value: &str) -> Result<Url> {
    identifier(challenge)?;
    let origin = Url::parse(origin).map_err(|_| err("Invalid passkey origin."))?;
    if origin.scheme() != "https" || origin.domain().is_none()
        || !origin.username().is_empty() || origin.password().is_some()
        || origin.query().is_some() || origin.fragment().is_some() || origin.path() != "/"
    {
        return Err(err("Invalid passkey origin."));
    }
    let target = Url::parse(value).map_err(|_| err("Invalid passkey page."))?;
    if target.origin() != origin.origin()
        || !target.username().is_empty() || target.password().is_some()
        || target.query().is_some() || target.path() != "/Bluefin/Accounts/Mesh/Passkey"
    {
        return Err(err("The passkey page does not belong to the paired server."));
    }
    let fragment = target.fragment().ok_or_else(|| err("Missing passkey continuation."))?;
    let (bound, secret) = fragment.split_once('.').ok_or_else(|| err("Invalid passkey continuation."))?;
    identifier(secret)?;
    if bound != challenge { return Err(err("The passkey page belongs to another connection.")); }
    Ok(target)
}

fn callback(challenge: &str, value: &str) -> Result<()> {
    identifier(challenge)?;
    if value != format!("bluefin://mesh-passkey/{challenge}") {
        return Err(err("The passkey callback belongs to another connection."));
    }
    Ok(())
}

pub(super) fn call(v: &Value, operation: &str) -> Result<Value> {
    let challenge = text(v, "challengeId")?;
    match operation {
        "meshBrowserValidate" => Ok(json!({"url": browser_url(
            text(v, "apiOrigin")?, challenge, text(v, "browserUrl")?)?.to_string()})),
        "meshBrowserCallback" => {
            callback(challenge, text(v, "callback")?)?;
            Ok(Value::Null)
        }
        _ => Err(err("Unsupported passkey continuation.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_page_is_bound_to_https_origin_path_and_challenge() {
        let challenge = "a".repeat(64);
        let secret = "b".repeat(64);
        let good = format!("https://example.test/Bluefin/Accounts/Mesh/Passkey#{challenge}.{secret}");
        assert!(browser_url("https://example.test", &challenge, &good).is_ok());
        for candidate in [
            good.replace("https:", "http:"),
            good.replace("example.test", "other.test"),
            good.replace("/Passkey#", "/Passkey/?x=1#"),
            good.replace("/Mesh/", "/Other/"),
            good.replace(&challenge, &"c".repeat(64)),
            format!("{good}.extra"),
            good.replace("https://", "https://user@"),
            good.replace("example.test/", "example.test:8443/"),
        ] {
            assert!(browser_url("https://example.test", &challenge, &candidate).is_err());
        }
        for origin in ["http://example.test", "https://user@example.test", "https://example.test/root",
            "https://example.test?x=1", "https://example.test#fragment"] {
            assert!(browser_url(origin, &challenge, &good).is_err());
        }
    }
    #[test]
    fn callback_is_an_exact_nonsecret_connection_identifier() {
        let challenge = "a".repeat(64);
        let good = format!("bluefin://mesh-passkey/{challenge}");
        assert!(callback(&challenge, &good).is_ok());
        for candidate in [
            good.replace("bluefin:", "https:"),
            good.replace("mesh-passkey", "connect"),
            format!("{good}?ok=true"),
            format!("{good}#secret"),
            good.replace(&challenge, &"b".repeat(64)),
        ] {
            assert!(callback(&challenge, &candidate).is_err());
        }
    }
}
