//! Native app-level iroh transport with a mandatory inner PQ mutual TLS session.
//! This module never opens or configures a system VPN.
mod identity;
mod tls;
pub use identity::{DeviceIdentity, verify_signature};
pub use identity::verify_signature as verify_device_signature;

use std::{collections::BTreeSet, net::SocketAddr, sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl};
use serde::{Serialize,de::DeserializeOwned};
use sha2::{Digest,Sha256};
use tokio::io::{AsyncReadExt,AsyncWriteExt};
use tokio_rustls::{TlsAcceptor,TlsConnector,TlsStream};
use crate::{ConnectInvite,Error,Result,ServerAddress,capsule::{canonical_relay,validate_service},protocol};

const HANDSHAKE_TIMEOUT:Duration=Duration::from_secs(20);
const FRAME_TIMEOUT:Duration=Duration::from_secs(30);
type QuicIo=tokio::io::Join<iroh::endpoint::RecvStream,iroh::endpoint::SendStream>;

/// Administrative/application relay policy, never constructed from an imported
/// invitation. Empty by default: public discovery and default relays stay off.
#[derive(Clone,Debug,Default)]
pub struct RelayPolicy{allowed:BTreeSet<String>}
impl RelayPolicy{
    pub fn allowlist(urls:impl IntoIterator<Item=String>)->Result<Self>{
        let mut allowed=BTreeSet::new();
        for url in urls{allowed.insert(canonical_relay(&url)?);}
        if allowed.len()>16{return Err(Error::TooLarge);}
        Ok(Self{allowed})
    }
    pub fn direct_only()->Self{Self::default()}
    fn require(&self,address:&ServerAddress)->Result<()>{
        if address.relay_urls.iter().any(|u|!self.allowed.contains(u)){return Err(Error::RelayNotAllowed);}
        Ok(())
    }
    fn mode(&self)->Result<RelayMode>{
        if self.allowed.is_empty(){return Ok(RelayMode::Disabled);}
        let urls=self.allowed.iter().map(|u|u.parse::<RelayUrl>().map_err(|_|Error::InvalidFields))
            .collect::<Result<Vec<_>>>()?;
        Ok(RelayMode::custom(urls))
    }
}

/// Only a completed strict inner handshake constructs this value.
/// It proves a live peer's key possession, not an account authorization.
pub struct AuthenticatedDevice{
    public_key:Vec<u8>,
    certificate_sha256:[u8;32],
    connection_binding:[u8;32],
}
impl std::fmt::Debug for AuthenticatedDevice{
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{
        f.debug_struct("AuthenticatedDevice").finish_non_exhaustive()
    }
}
impl AuthenticatedDevice{
    pub fn public_key(&self)->&[u8]{&self.public_key}
    pub fn certificate_sha256(&self)->[u8;32]{self.certificate_sha256}
    pub fn connection_binding(&self)->[u8;32]{self.connection_binding}
}

/// Shared per-application endpoint. Reuse it across requests rather than
/// opening a new UDP endpoint for every HTTP operation.
#[derive(Clone)]
pub struct PqcEndpoint{
    endpoint:Endpoint,
    identity:DeviceIdentity,
    relay_policy:RelayPolicy,
    service:Option<String>,
    server_config:Option<Arc<rustls::ServerConfig>>,
}
impl PqcEndpoint{
    pub async fn client(identity:&DeviceIdentity)->Result<Self>{
        Self::client_with_policy(identity,RelayPolicy::direct_only()).await
    }
    pub async fn client_with_policy(identity:&DeviceIdentity,relay_policy:RelayPolicy)->Result<Self>{
        tls::provider()?;
        let endpoint=Endpoint::builder(iroh::endpoint::presets::Minimal)
            .clear_address_lookup().clear_relay_transports()
            .relay_mode(relay_policy.mode()?)
            .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
            .bind().await.map_err(|_|Error::Network)?;
        Ok(Self{endpoint,identity:identity.clone(),relay_policy,service:None,server_config:None})
    }
    pub async fn server(identity:&DeviceIdentity,bind:SocketAddr,relays:Vec<String>,service:&str)->Result<Self>{
        validate_service(service)?;
        if relays.len()>2{return Err(Error::InvalidFields);}
        let relay_policy=RelayPolicy::allowlist(relays)?;
        let server_config=tls::server(identity,service)?;
        let endpoint=Endpoint::builder(iroh::endpoint::presets::Minimal)
            .clear_address_lookup().clear_relay_transports().clear_ip_transports()
            .secret_key(identity.iroh_secret()?)
            .bind_addr(bind).map_err(|_|Error::InvalidFields)?
            .relay_mode(relay_policy.mode()?)
            .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
            .alpns(vec![service.as_bytes().to_vec()])
            .bind().await.map_err(|_|Error::Network)?;
        // Relay-backed invitations must not publish an unreachable home relay.
        if !relay_policy.allowed.is_empty(){
            tokio::time::timeout(HANDSHAKE_TIMEOUT,endpoint.online()).await.map_err(|_|Error::Timeout)?;
        }
        Ok(Self{endpoint,identity:identity.clone(),relay_policy,service:Some(service.to_owned()),
            server_config:Some(server_config)})
    }
    pub fn endpoint_id(&self)->[u8;32]{*self.endpoint.id().as_bytes()}
    pub fn certificate_sha256(&self)->[u8;32]{self.identity.certificate_sha256()}
    pub fn direct_addresses(&self)->Vec<SocketAddr>{
        let mut addresses:Vec<_>=self.endpoint.addr().ip_addrs().copied()
            .filter(|a|!a.ip().is_unspecified()&&!a.ip().is_multicast()&&a.port()!=0).collect();
        // An explicitly bound loopback endpoint is useful for local callers
        // and conformance. Do not manufacture a loopback origin for a relay.
        for bound in self.endpoint.bound_sockets(){
            if !bound.ip().is_unspecified() && bound.port()!=0{addresses.push(bound);}
        }
        addresses.sort();addresses.dedup();addresses.truncate(4);addresses
    }
    pub fn server_address(&self)->Result<ServerAddress>{
        let service=self.service.clone().ok_or(Error::ServiceMismatch)?;
        let mut relay_urls:Vec<_>=self.endpoint.addr().relay_urls().map(ToString::to_string)
            .filter(|url|self.relay_policy.allowed.contains(url)).collect();
        relay_urls.sort();relay_urls.dedup();relay_urls.truncate(2);
        let address=ServerAddress{endpoint_id:self.endpoint_id(),
            server_certificate_sha256:self.certificate_sha256(),service,relay_urls,
            direct_addresses:self.direct_addresses()};
        address.validate()?;Ok(address)
    }
    pub async fn connect(&self,invite:&ConnectInvite)->Result<PqcConnection>{
        let now=SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_|Error::Expired)?;
        invite.validate_at(now.as_millis().try_into().map_err(|_|Error::Expired)?)?;
        self.connect_server(&invite.server_address()).await
    }
    pub async fn connect_server(&self,address:&ServerAddress)->Result<PqcConnection>{
        address.validate()?;self.relay_policy.require(address)?;
        let id=EndpointId::from_bytes(&address.endpoint_id).map_err(|_|Error::InvalidIdentity)?;
        let mut peer=EndpointAddr::new(id);
        for addr in &address.direct_addresses{peer=peer.with_ip_addr(*addr);}
        for url in &address.relay_urls{
            peer=peer.with_relay_url(url.parse().map_err(|_|Error::InvalidFields)?);
        }
        let config=tls::client(&self.identity,address.server_certificate_sha256,&address.service)?;
        let future=async{
            let outer=self.endpoint.connect(peer,address.service.as_bytes()).await.map_err(|_|Error::Network)?;
            if outer.remote_id()!=id || outer.alpn()!=address.service.as_bytes(){return Err(Error::Authentication);}
            let (send,recv)=outer.open_bi().await.map_err(|_|Error::Network)?;
            let io=tokio::io::join(recv,send);
            let name=rustls::pki_types::ServerName::try_from(tls::SERVER_NAME).map_err(|_|Error::InvalidIdentity)?;
            let stream=TlsConnector::from(config).connect(name,io).await.map_err(|_|Error::Authentication)?;
            tls::check_negotiated(stream.get_ref().1,&address.service)?;
            let binding=stream.get_ref().1.export_keying_material([0u8;32],tls::EXPORTER_LABEL,Some(address.service.as_bytes()))
                .map_err(|_|Error::Authentication)?;
            Ok(PqcConnection{stream:TlsStream::Client(stream),outer,_endpoint:self.endpoint.clone(),
                binding,peer_device:None,failed:false})
        };
        tokio::time::timeout(HANDSHAKE_TIMEOUT,future).await.map_err(|_|Error::Timeout)?
    }
    pub async fn accept(&self)->Result<PqcConnection>{
        let config=self.server_config.clone().ok_or(Error::ServiceMismatch)?;
        let service=self.service.as_ref().ok_or(Error::ServiceMismatch)?;
        // Waiting for a new connection is intentionally cancelable, with no
        // idle timeout. Each accepted handshake has its own deadline.
        let incoming=self.endpoint.accept().await.ok_or(Error::Closed)?;
        let future=async{
            let outer=incoming.await.map_err(|_|Error::Network)?;
            if outer.alpn()!=service.as_bytes(){return Err(Error::ServiceMismatch);}
            let (send,recv)=outer.accept_bi().await.map_err(|_|Error::Network)?;
            let stream=TlsAcceptor::from(config).accept(tokio::io::join(recv,send)).await.map_err(|_|Error::Authentication)?;
            tls::check_negotiated(stream.get_ref().1,service)?;
            let binding=stream.get_ref().1.export_keying_material([0u8;32],tls::EXPORTER_LABEL,Some(service.as_bytes()))
                .map_err(|_|Error::Authentication)?;
            let certificates=stream.get_ref().1.peer_certificates().ok_or(Error::Authentication)?;
            if certificates.len()!=1{return Err(Error::Authentication);}
            let public_key=tls::certificate_key(&certificates[0]).map_err(|_|Error::Authentication)?;
            let certificate_sha256=Sha256::digest(certificates[0].as_ref()).into();
            let peer_device=Some(AuthenticatedDevice{public_key,certificate_sha256,connection_binding:binding});
            Ok(PqcConnection{stream:TlsStream::Server(stream),outer,_endpoint:self.endpoint.clone(),
                binding,peer_device,failed:false})
        };
        tokio::time::timeout(HANDSHAKE_TIMEOUT,future).await.map_err(|_|Error::Timeout)?
    }
    /// Closes active connections and unblocks accept. The listener owner joins
    /// its own spawned tasks and drops endpoint clones during shutdown.
    pub async fn close(&self){self.endpoint.close().await;}
}

/// Framed application channel. Its fields and constructor are private so
/// callers cannot substitute a boolean "PQC verified" assertion.
pub struct PqcConnection{
    stream:TlsStream<QuicIo>,
    outer:iroh::endpoint::Connection,
    _endpoint:Endpoint,
    binding:[u8;32],
    peer_device:Option<AuthenticatedDevice>,
    failed:bool,
}
impl PqcConnection{
    pub fn binding(&self)->[u8;32]{self.binding}
    pub fn peer_device(&self)->Option<&AuthenticatedDevice>{self.peer_device.as_ref()}
    /// Untrusted routing label, suitable for rate-limiter keys; never an IP
    /// address or local-network proof. Auth decisions use peer_device instead.
    pub fn remote_endpoint(&self)->String{self.outer.remote_id().to_string()}
    pub async fn send<T:Serialize>(&mut self,value:&T)->Result<()>{
        if self.failed{return Err(Error::Closed);}
        let bytes=protocol::encode(value)?;
        let result=tokio::time::timeout(FRAME_TIMEOUT,async{
            self.stream.write_u32(bytes.len() as u32).await.map_err(|_|Error::Network)?;
            self.stream.write_all(&bytes).await.map_err(|_|Error::Network)?;
            self.stream.flush().await.map_err(|_|Error::Network)
        }).await.map_err(|_|Error::Timeout).and_then(|r|r);
        if result.is_err(){self.poison();}
        result
    }
    pub async fn receive<T:DeserializeOwned>(&mut self)->Result<T>{
        if self.failed{return Err(Error::Closed);}
        let result=tokio::time::timeout(FRAME_TIMEOUT,async{
            let len=self.stream.read_u32().await.map_err(|_|Error::Network)? as usize;
            if len==0 || len>protocol::MAX_FRAME_BYTES{return Err(Error::TooLarge);}
            let mut bytes=vec![0;len];
            self.stream.read_exact(&mut bytes).await.map_err(|_|Error::Network)?;
            protocol::decode(&bytes)
        }).await.map_err(|_|Error::Timeout).and_then(|r|r);
        if result.is_err(){self.poison();}
        result
    }
    fn poison(&mut self){self.failed=true;self.outer.close(1u32.into(),b"protocol closed");}
    pub async fn close(&mut self){
        let _=tokio::time::timeout(Duration::from_secs(2),self.stream.shutdown()).await;
        self.failed=true;self.outer.close(0u32.into(),b"closed");
    }
}
