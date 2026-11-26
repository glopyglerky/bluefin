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

#[tokio::test]
async fn stalled_peer_does_not_hold_the_incoming_accept_queue() {
    use std::time::Duration;
    let server = PqcEndpoint::server(&DeviceIdentity::from_seed([41;32]).unwrap(),
        "127.0.0.1:0".parse().unwrap(), vec![], "bluefin/connect/1").await.unwrap();
    let address = server.server_address().unwrap();
    let raw = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .clear_address_lookup().clear_relay_transports().relay_mode(iroh::RelayMode::Disabled)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled).bind().await.unwrap();
    let mut target = iroh::EndpointAddr::new(iroh::EndpointId::from_bytes(&address.endpoint_id).unwrap());
    for address in &address.direct_addresses { target = target.with_ip_addr(*address); }
    let slow_endpoint = raw.clone();
    let connecting = tokio::spawn(async move { slow_endpoint.connect(target, b"bluefin/connect/1").await });
    let pending = tokio::time::timeout(Duration::from_secs(5), server.incoming()).await.unwrap().unwrap();
    let stalled = tokio::spawn(pending.authenticate());
    let outer = connecting.await.unwrap().unwrap();
    // This peer completed outer QUIC but deliberately never starts inner TLS.
    let client = PqcEndpoint::client(&DeviceIdentity::from_seed([42;32]).unwrap()).await.unwrap();
    let healthy_endpoint = client.clone();
    let healthy = tokio::spawn(async move { healthy_endpoint.connect_server(&address).await });
    let second = tokio::time::timeout(Duration::from_secs(5), server.incoming()).await.unwrap().unwrap();
    let mut authenticated = second.authenticate().await.unwrap();
    let mut client_connection = healthy.await.unwrap().unwrap();
    assert!(authenticated.peer_device().is_some());
    outer.close(0u32.into(), b"test complete");
    assert!(tokio::time::timeout(Duration::from_secs(5), stalled).await.unwrap().unwrap().is_err());
    client_connection.close().await; authenticated.close().await;
    raw.close().await; client.close().await; server.close().await;
}
