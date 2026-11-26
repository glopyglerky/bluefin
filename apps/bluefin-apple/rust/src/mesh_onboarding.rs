//! Local invitation decoding and presentation. The capsule never grants account authority.
//! All input forms produce the same canonical invite, which the live authority redeems.
use super::{err, text, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use bluefin_mesh_connect::{ConnectInvite, InviteIntent, QrTheme, decode_qr_image, qr_png, qr_svg};
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn now_ms() -> Result<u64> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|_| err("The device clock is unavailable."))?.as_millis();
    u64::try_from(millis).map_err(|_| err("The device clock is outside the supported range."))
}

pub(super) fn parse(code: &str) -> Result<ConnectInvite> {
    let invite = ConnectInvite::parse(code)
        .map_err(|_| err("This is not a valid Bluefin connection code."))?;
    if invite.service != "bluefin/connect/1" {
        return Err(err("This invitation is for another application."));
    }
    invite.validate_at(now_ms()?)
        .map_err(|_| err("This connection code has expired. Ask for a new one."))?;
    Ok(invite)
}

fn describe(invite: &ConnectInvite) -> Result<Value> {
    let code = invite.to_code().map_err(|_| err("This connection code is invalid."))?;
    Ok(json!({
        "code": code,
        "link": format!("bluefin://connect/{}", code),
        "expiresAtUnixMs": invite.expires_at_unix_ms,
        "intent": match invite.intent { InviteIntent::Login => "login", InviteIntent::Signup => "signup" },
        "accountLinked": invite.account_hint.is_some(),
        "singleUse": true,
        "service": "bluefin/connect/1"
    }))
}

pub(super) fn call(v: &Value, operation: &str) -> Result<Value> {
    let invite = parse(text(v, "code")?)?;
    match operation {
        "meshInviteDecode" => describe(&invite),
        "meshInviteQr" => {
            let theme = QrTheme::bluefin();
            let png = qr_png(&invite, &theme)
                .map_err(|_| err("The connection image could not be created."))?;
            let svg = qr_svg(&invite, &theme)
                .map_err(|_| err("The connection image could not be created."))?;
            Ok(json!({"invite":describe(&invite)?,"pngBase64":STANDARD.encode(png),"svg":svg}))
        }
        _ => Err(err("Unsupported connection operation.")),
    }
}

pub(super) fn image(bytes: &[u8]) -> Result<Value> {
    let invite = decode_qr_image(bytes)
        .map_err(|_| err("Choose an image containing exactly one valid Bluefin connection QR code."))?;
    let code = invite.to_code().map_err(|_| err("This connection code is invalid."))?;
    let invite = parse(&code)?;
    describe(&invite)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoding_does_not_accept_a_server_url_or_unvalidated_payload() {
        for code in ["https://example.invalid", "192.0.2.1", "bluefin://connect/invalid", "{}"] {
            assert!(parse(code).is_err());
        }
    }
    #[test]
    fn invalid_image_is_not_a_connection() {
        assert!(image(b"not an image").is_err());
    }
}
