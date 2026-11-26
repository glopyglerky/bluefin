#![cfg(feature="native")]
use mesh_connect::{native::{DeviceIdentity,PqcEndpoint,RelayPolicy,verify_device_signature},Error};
use serde_json::{json,Value};

#[test]
fn stable_device_identity_signing_and_tamper_refusal(){
    let a=DeviceIdentity::from_seed([7;32]).unwrap();
    let b=DeviceIdentity::from_seed([7;32]).unwrap();
    assert_eq!(a.public_key(),b.public_key());
    assert_eq!(a.certificate_sha256(),b.certificate_sha256());
    let signature=a.sign(b"mesh-connect/test/device-proof/v1").unwrap();
    verify_device_signature(&a.public_key(),b"mesh-connect/test/device-proof/v1",&signature).unwrap();
    assert!(verify_device_signature(&a.public_key(),b"other challenge",&signature).is_err());
    assert!(!format!("{a:?}").contains("7, 7"));
}
#[tokio::test]
async fn actual_iroh_pinned_hybrid_tls_and_live_device_proof_roundtrip(){
    let server_key=DeviceIdentity::from_seed([11;32]).unwrap();
    let client_key=DeviceIdentity::from_seed([12;32]).unwrap();
    let server=PqcEndpoint::server(&server_key,"127.0.0.1:0".parse().unwrap(),vec![],"bluefin/connect/1").await.unwrap();
    let address=server.server_address().unwrap();
    let peer_key=client_key.public_key();
    let accepting=server.clone();
    let task=tokio::spawn(async move{
        let mut connection=accepting.accept().await.unwrap();
        assert_eq!(connection.peer_device().unwrap().public_key(),peer_key);
        let message:Value=connection.receive().await.unwrap();
        assert_eq!(message["binding"],json!(connection.binding()));
        connection.send(&json!({"ok":true,"binding":connection.binding()})).await.unwrap();
        connection
    });
    let client=PqcEndpoint::client(&client_key).await.unwrap();
    let mut connection=client.connect_server(&address).await.unwrap();
    connection.send(&json!({"binding":connection.binding()})).await.unwrap();
    let reply:Value=connection.receive().await.unwrap();
    assert_eq!(reply["ok"],true);assert_eq!(reply["binding"],json!(connection.binding()));
    let mut accepted=task.await.unwrap();
    connection.close().await;accepted.close().await;
    client.close().await;server.close().await;
}
#[tokio::test]
async fn wrong_pin_wrong_service_and_unapproved_relay_never_release_application_channel(){
    let key=DeviceIdentity::from_seed([21;32]).unwrap();
    let server=PqcEndpoint::server(&key,"127.0.0.1:0".parse().unwrap(),vec![],"bluefin/connect/1").await.unwrap();
    let mut address=server.server_address().unwrap();
    let client=PqcEndpoint::client(&DeviceIdentity::from_seed([22;32]).unwrap()).await.unwrap();
    let accepting=server.clone();
    let task=tokio::spawn(async move{accepting.accept().await});
    address.server_certificate_sha256=[99;32];
    assert!(matches!(client.connect_server(&address).await,Err(Error::Authentication)));
    assert!(task.await.unwrap().is_err());
    address=server.server_address().unwrap();
    address.relay_urls=vec!["https://relay.example/".into()];
    assert!(matches!(client.connect_server(&address).await,Err(Error::RelayNotAllowed)));
    assert!(RelayPolicy::allowlist(vec!["https://user@relay.example/".into()]).is_err());
    address.relay_urls.clear();address.service="wrong/connect/1".into();
    assert!(client.connect_server(&address).await.is_err());
    client.close().await;server.close().await;
}
