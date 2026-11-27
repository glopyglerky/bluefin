//! Browser iroh client with the same strict pinned PQ inner TLS profile.
//! The outer transport is relay-only. No certificate bypass, public discovery,
//! default relay, or HTTP credential fallback is provided.
#[path = "native/identity.rs"]
mod identity;
#[path = "native/tls.rs"]
mod tls;
pub use identity::{DeviceIdentity, verify_signature};

use std::{collections::BTreeSet, sync::Arc, time::Duration};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl};
use n0_future::time::{SystemTime, timeout};
use serde::{Serialize, de::DeserializeOwned};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::{TlsConnector, client::TlsStream};
use crate::{ConnectInvite, Error, Result, ServerAddress, capsule::canonical_relay, protocol};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
const FRAME_TIMEOUT: Duration = Duration::from_secs(30);
type QuicIo = tokio::io::Join<iroh::endpoint::RecvStream, iroh::endpoint::SendStream>;

/// The trusted HTTPS application's independent relay policy. An invitation
/// supplies routing hints only and cannot populate this allowlist.
#[derive(Clone, Debug)]
pub struct RelayPolicy { allowed: BTreeSet<String> }
impl RelayPolicy {
    pub fn allowlist(urls: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut allowed = BTreeSet::new();
        for url in urls {
            allowed.insert(canonical_relay(&url)?);
            if allowed.len() > 16 { return Err(Error::TooLarge); }
        }
        if allowed.is_empty() { return Err(Error::RelayNotAllowed); }
        Ok(Self { allowed })
    }

    fn mode(&self) -> Result<RelayMode> {
        let urls = self.allowed.iter().map(|url| url.parse::<RelayUrl>()
            .map_err(|_| Error::InvalidFields)).collect::<Result<Vec<_>>>()?;
        Ok(RelayMode::custom(urls))
    }

    fn peer(&self, address: &ServerAddress) -> Result<EndpointAddr> {
        address.validate()?;
        let id = EndpointId::from_bytes(&address.endpoint_id).map_err(|_| Error::InvalidIdentity)?;
        let mut peer = EndpointAddr::new(id);
        let mut admitted = 0;
        for url in address.relay_urls.iter().filter(|url| self.allowed.contains(*url)) {
            peer = peer.with_relay_url(url.parse().map_err(|_| Error::InvalidFields)?);
            admitted += 1;
        }
        // Browsers have no direct UDP path. Do not spend the invitation if its
        // only routes are direct or outside the application allowlist.
        if admitted == 0 { return Err(Error::RelayNotAllowed); }
        Ok(peer)
    }

    pub fn permits(&self, address: &ServerAddress) -> Result<()> { self.peer(address).map(|_| ()) }
}

#[derive(Debug)]
struct BrowserClock;
impl rustls::time_provider::TimeProvider for BrowserClock {
    fn current_time(&self) -> Option<rustls::pki_types::UnixTime> {
        let duration = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).ok()?;
        Some(rustls::pki_types::UnixTime::since_unix_epoch(duration))
    }
}

pub fn now_ms() -> Result<u64> {
    let duration = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| Error::Expired)?;
    duration.as_millis().try_into().map_err(|_| Error::Expired)
}

#[derive(Clone)]
pub struct PqcEndpoint {
    endpoint: Endpoint,
    identity: DeviceIdentity,
    relay_policy: RelayPolicy,
}
impl PqcEndpoint {
    pub async fn client_with_policy(identity: &DeviceIdentity, relay_policy: RelayPolicy) -> Result<Self> {
        tls::provider()?;
        let transport = iroh::endpoint::QuicTransportConfig::builder()
            .max_concurrent_bidi_streams(1u32.into())
            .max_concurrent_uni_streams(0u32.into())
            .stream_receive_window((1024 * 1024u32).into())
            .receive_window((1024 * 1024u32).into())
            .send_window(2 * 1024 * 1024)
            .datagram_receive_buffer_size(None).build();
        let endpoint = Endpoint::builder(iroh::endpoint::presets::Minimal)
            .clear_address_lookup().clear_relay_transports()
            .relay_mode(relay_policy.mode()?)
            // Each tab/endpoint has its own relay registration. The stable
            // device identity is authenticated by the inner pinned PQ TLS layer.
            // Reusing the outer key would let restored tabs displace one another.
            .transport_config(transport)
            .bind().await.map_err(|_| Error::Network)?;
        Ok(Self { endpoint, identity: identity.clone(), relay_policy })
    }

    /// Outer routing identity for a private relay bootstrap request. This is
    /// not the inner PQ device identity and grants no account authority.
    pub fn endpoint_id(&self) -> [u8;32] { *self.endpoint.id().as_bytes() }

    pub async fn connect(&self, invite: &ConnectInvite) -> Result<PqcConnection> {
        invite.validate_at(now_ms()?)?;
        self.connect_server(&invite.server_address()).await
    }

    pub async fn connect_server(&self, address: &ServerAddress) -> Result<PqcConnection> {
        let peer = self.relay_policy.peer(address)?;
        let id = EndpointId::from_bytes(&address.endpoint_id).map_err(|_| Error::InvalidIdentity)?;
        let mut config = (*tls::client(&self.identity, address.server_certificate_sha256, &address.service)?).clone();
        config.time_provider = Arc::new(BrowserClock);
        let future = async {
            let outer = self.endpoint.connect(peer, address.service.as_bytes()).await
                .map_err(|_| Error::Network)?;
            if outer.remote_id() != id || outer.alpn() != address.service.as_bytes() {
                outer.close(1u32.into(), b"authentication failed");
                return Err(Error::Authentication);
            }
            let (send, recv) = outer.open_bi().await.map_err(|_| Error::Network)?;
            let name = rustls::pki_types::ServerName::try_from(tls::SERVER_NAME)
                .map_err(|_| Error::InvalidIdentity)?;
            let stream = TlsConnector::from(Arc::new(config)).connect(name, tokio::io::join(recv, send))
                .await.map_err(|_| Error::Authentication)?;
            tls::check_negotiated(stream.get_ref().1, &address.service)?;
            let binding = stream.get_ref().1.export_keying_material([0u8; 32],
                tls::EXPORTER_LABEL, Some(address.service.as_bytes())).map_err(|_| Error::Authentication)?;
            Ok(PqcConnection { stream, outer, _endpoint: self.endpoint.clone(), binding, failed: false })
        };
        timeout(HANDSHAKE_TIMEOUT, future).await.map_err(|_| Error::Timeout)?
    }

    pub async fn close(&self) { self.endpoint.close().await; }
}

#[derive(Clone)]
pub struct CancelHandle { outer: iroh::endpoint::Connection }
impl CancelHandle {
    pub fn cancel(&self) { self.outer.close(0u32.into(), b"canceled"); }
}

pub struct PqcConnection {
    stream: TlsStream<QuicIo>,
    outer: iroh::endpoint::Connection,
    _endpoint: Endpoint,
    binding: [u8; 32],
    failed: bool,
}
impl PqcConnection {
    pub fn binding(&self) -> [u8; 32] { self.binding }
    pub fn cancel_handle(&self) -> CancelHandle { CancelHandle { outer: self.outer.clone() } }
    pub async fn send<T: Serialize>(&mut self, value: &T) -> Result<()> {
        if self.failed { return Err(Error::Closed); }
        let bytes = protocol::encode(value)?;
        let result = timeout(FRAME_TIMEOUT, async {
            self.stream.write_u32(bytes.len() as u32).await.map_err(|_| Error::Network)?;
            self.stream.write_all(&bytes).await.map_err(|_| Error::Network)?;
            self.stream.flush().await.map_err(|_| Error::Network)
        }).await.map_err(|_| Error::Timeout).and_then(|result| result);
        if result.is_err() { self.poison(); }
        result
    }
    pub async fn receive<T: DeserializeOwned>(&mut self) -> Result<T> {
        if self.failed { return Err(Error::Closed); }
        let result = timeout(FRAME_TIMEOUT, async {
            let length = self.stream.read_u32().await.map_err(|_| Error::Network)? as usize;
            if length == 0 || length > protocol::MAX_FRAME_BYTES { return Err(Error::TooLarge); }
            let mut bytes = vec![0; length];
            self.stream.read_exact(&mut bytes).await.map_err(|_| Error::Network)?;
            protocol::decode(&bytes)
        }).await.map_err(|_| Error::Timeout).and_then(|result| result);
        if result.is_err() { self.poison(); }
        result
    }
    fn poison(&mut self) {
        self.failed = true;
        self.outer.close(1u32.into(), b"protocol closed");
    }
    pub async fn close(&mut self) {
        let _ = timeout(Duration::from_secs(2), self.stream.shutdown()).await;
        self.failed = true;
        self.outer.close(0u32.into(), b"closed");
    }
}
impl Drop for PqcConnection {
    fn drop(&mut self) {
        // Aborted browser work must not keep an unobserved stream active.
        self.outer.close(0u32.into(), b"released");
    }
}
