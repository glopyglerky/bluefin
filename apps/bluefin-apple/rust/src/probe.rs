//! Standalone, pinned-peer QUIC handshake probe. This is not a media tunnel.
use super::*;
use iroh::{endpoint::presets::Minimal, Endpoint, EndpointAddr, EndpointId};
use std::net::SocketAddr;
pub const ALPN: &[u8] = b"bluefin/apple-probe/1";
async fn connect(peer: EndpointId, addr: SocketAddr) -> Result<Value> {
    let endpoint = Endpoint::builder(Minimal)
        .bind()
        .await
        .map_err(|_| err("Iroh endpoint unavailable"))?;
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let conn = endpoint.connect(EndpointAddr::new(peer).with_ip_addr(addr), ALPN).await.map_err(|_|err("Iroh pinned-peer handshake failed"))?;
        // Iroh's TLS handshake authenticates the EndpointId supplied to connect.
        conn.close(0u32.into(), b"probe complete");
        Ok(json!({"connected":true,"peerId":peer.to_string(),"alpn":"bluefin/apple-probe/1","mediaRouteQualified":false}))
    }).await.unwrap_or_else(|_|Err(err("Iroh probe timed out")));
    // Closing drops all probe work. Bound graceful shutdown as well.
    let _ = tokio::time::timeout(Duration::from_secs(1), endpoint.close()).await;
    result
}
pub fn call(v: &Value) -> Result<Value> {
    let peer: EndpointId = text(v, "peerId")?
        .parse()
        .map_err(|_| err("Invalid pinned peerId"))?;
    let addr: SocketAddr = text(v, "peerAddress")?
        .parse()
        .map_err(|_| err("Invalid peerAddress; numeric IP:port required"))?;
    if addr.port() == 0 || addr.ip().is_unspecified() || addr.ip().is_multicast() {
        return Err(err("Invalid peerAddress"));
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .map_err(|_| err("Iroh runtime unavailable"))?;
    rt.block_on(connect(peer, addr))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn real_loopback_handshake_and_wrong_pin() {
        let server = Endpoint::builder(Minimal)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .unwrap();
        let addr = server.addr().ip_addrs().next().copied().unwrap();
        let addr = SocketAddr::new("127.0.0.1".parse().unwrap(), addr.port());
        let accept = tokio::spawn({
            let s = server.clone();
            async move {
                let incoming = s.accept().await.unwrap();
                let c = incoming.await.unwrap();
                let _ = c.closed().await;
            }
        });
        assert_eq!(connect(server.id(), addr).await.unwrap()["connected"], true);
        accept.await.unwrap();
        let other = Endpoint::builder(Minimal).bind().await.unwrap();
        let second_accept = tokio::spawn({
            let s = server.clone();
            async move {
                if let Some(incoming) = s.accept().await {
                    let _ = incoming.await;
                }
            }
        });
        let wrong = connect(other.id(), addr).await;
        assert!(wrong.is_err());
        other.close().await;
        server.close().await;
        second_accept.await.unwrap();
    }
}
