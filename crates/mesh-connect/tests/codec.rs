use mesh_connect::{ConnectInvite,Error,InviteIntent,protocol};
use serde_json::Value;

fn fixture()->(ConnectInvite,Value){
    let f:Value=serde_json::from_str(include_str!("fixtures/invite-v1.json")).unwrap();
    (serde_json::from_value(f["invite"].clone()).unwrap(),f)
}
#[test]
fn independent_binary_fixture_and_all_text_entry_forms_agree(){
    let(invite,f)=fixture();
    let bytes=invite.to_bytes().unwrap();
    let hex:String=bytes.iter().map(|b|format!("{b:02x}")).collect();
    assert_eq!(hex,f["binary_hex"]);
    assert_eq!(invite.to_code().unwrap(),f["code"]);
    assert_eq!(invite.to_uri().unwrap(),f["uri"]);
    for text in [
        f["code"].as_str().unwrap().to_owned(),
        f["code"].as_str().unwrap().to_lowercase().replace('-', " \n"),
        f["uri"].as_str().unwrap().to_owned(),
        format!("bluefin://connect/{}",invite.to_code().unwrap()),
        invite.to_code().unwrap().replace('-',""),
    ]{
        assert_eq!(ConnectInvite::parse_at(&text,1_900_000_000_000).unwrap(),invite);
    }
}
#[test]
fn malformed_unknown_and_expired_codes_fail_closed_without_echoing_secret(){
    let(invite,_)=fixture();
    assert!(matches!(invite.validate_at(invite.expires_at_unix_ms),Err(Error::Expired)));
    let mut bytes=invite.to_bytes().unwrap();bytes[70]^=1;
    assert!(matches!(ConnectInvite::from_bytes(&bytes),Err(Error::Checksum)));
    bytes[2]=2;
    assert!(matches!(ConnectInvite::from_bytes(&bytes),Err(Error::UnsupportedVersion)));
    let code=invite.to_code().unwrap();
    for bad in [format!("{code}0"),format!("{code}="),format!("https://example.com/{code}"),
        format!("{code}☃"),"X".repeat(2049)]{
        assert!(ConnectInvite::parse(&bad).is_err());
    }
    let debug=format!("{invite:?}");
    assert!(debug.contains("REDACTED"));
    assert!(!debug.contains(&code));
    assert!(!debug.contains("6, 6, 6"));
}
#[test]
fn untrusted_relay_fields_and_ambiguous_address_forms_are_rejected(){
    let(mut invite,_)=fixture();
    for bad in ["http://relay.example/","https://user@relay.example/","https://relay.example/?token=x",
        "https://relay.example/#x","https://relay.example/path","https://relay.example","https://RELAY.example/"]{
        invite.relay_urls=vec![bad.into()];assert!(invite.to_code().is_err());
    }
    let(mut invite,_)=fixture();
    invite.direct_addresses.push(invite.direct_addresses[0]);assert!(invite.validate().is_err());
    invite.direct_addresses=vec!["[fe80::1%5]:4433".parse().unwrap()];assert!(invite.validate().is_err());
}
#[test]
fn persistent_server_address_does_not_keep_one_use_capability(){
    let(invite,_)=fixture();
    let address=serde_json::to_string(&invite.server_address()).unwrap();
    assert!(!address.contains("secret"));assert!(!address.contains("invitation_id"));
    assert!(!address.contains("expires_at"));assert!(!address.contains("account_hint"));
}
#[test]
fn login_cannot_supply_signup_account_and_frames_are_bounded(){
    let(invite,_)=fixture();
    let request=protocol::Request::Begin{request_id:[7;32],invitation:invite,
        device_public_key:vec![1;1952],device_label:"Phone".into(),account_name:Some("admin".into())};
    assert!(request.validate().is_err());
    assert!(protocol::decode::<Value>(&vec![b' ';65537]).is_err());
    assert!(protocol::decode::<protocol::Envelope<protocol::Request>>(br#"{"version":1,"body":{"type":"future"}}"#).is_err());
    let(mut invite,_)=fixture();invite.intent=InviteIntent::Signup;
    assert!(protocol::Request::Begin{request_id:[7;32],invitation:invite,
        device_public_key:vec![1;1952],device_label:"Phone".into(),account_name:Some("new-user".into())}.validate().is_ok());
}
