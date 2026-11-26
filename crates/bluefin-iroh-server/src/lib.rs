//! Built-in direct-only media transport. The caller supplies the production router.
//! Endpoint identity authenticates the transport, never the viewing account.
#[cfg(unix)]
mod identity;
#[cfg(not(unix))]
mod identity {
    pub(crate) fn open(_: &std::path::Path) -> anyhow::Result<(iroh::SecretKey, std::fs::File)> {
        anyhow::bail!("Iroh persistent identity unsupported on this platform")
    }
}
mod protocol;
use anyhow::{Result, ensure};
use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, header},
};
use futures_util::StreamExt;
use iroh::{
    Endpoint, RelayMode, TransportAddr,
    endpoint::{Connection, IncomingAddr, LocalTransportAddr},
};
use std::{fmt::Write as _, fs::File, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    sync::{Semaphore, watch},
    task::JoinSet,
};
use tower::ServiceExt;

/// Native client protocol and fixed foreground-media flow limits.
pub const ALPN: &[u8] = b"bluefin/http/1";
const IDLE: Duration = Duration::from_secs(30);
const TOTAL: Duration = Duration::from_hours(6);
/// Trusted process-start input. Absence means no key or socket is opened.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Existing private 0700 directory and persistent 0600 raw secret file.
    pub secret_key_file: PathBuf,
    /// One concrete operator-chosen local UDP address; no wildcard discovery.
    pub bind_address: SocketAddr,
}
impl Config {
    /// Reads only trusted process input, once at startup. No product API mutates it.
    ///
    /// # Errors
    /// Refuses an unreadable, oversized or malformed explicitly configured file.
    /// No identity or UDP socket is opened by this preflight. The server binds
    /// only after the complete configuration has been accepted.
    pub fn from_env() -> Result<Option<Self>> {
        let config: Option<Self> = std::env::var_os("BLUEFIN_IROH_CONFIG")
            .map(|v| {
                use std::io::Read as _;
                let file = std::fs::File::open(v)
                    .map_err(|_| anyhow::anyhow!("Iroh configuration unavailable"))?;
                ensure!(
                    file.metadata()?.is_file(),
                    "regular Iroh configuration required"
                );
                let mut bytes = Vec::new();
                file.take(16 * 1024 + 1).read_to_end(&mut bytes)?;
                ensure!(bytes.len() <= 16 * 1024, "Iroh configuration bound");
                serde_json::from_slice(&bytes)
                    .map_err(|_| anyhow::anyhow!("invalid Iroh configuration"))
            })
            .transpose()?;
        Ok(config)
    }
}

#[cfg(test)]
mod startup_tests {
    #[test]
    fn process_preflight_parses_opt_in_without_opening_identity() {
        const MODE: &str = "BLUEFIN_TEST_IROH_PREFLIGHT_MODE";
        if let Ok(mode) = std::env::var(MODE) {
            if mode == "disabled" {
                assert!(super::Config::from_env().unwrap().is_none());
            } else {
                assert!(super::Config::from_env().unwrap().is_some());
            }
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let identity = temp.path().join("identity");
        let config = temp.path().join("config.json");
        std::fs::write(
            &config,
            serde_json::to_vec(&serde_json::json!({
                "secret_key_file": identity,
                "bind_address": "127.0.0.1:0"
            }))
            .unwrap(),
        )
        .unwrap();
        for mode in ["disabled", "configured"] {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child.args([
                "--exact",
                "startup_tests::process_preflight_parses_opt_in_without_opening_identity",
            ]);
            child.env(MODE, mode).env_remove("BLUEFIN_IROH_CONFIG");
            if mode == "configured" {
                child.env("BLUEFIN_IROH_CONFIG", &config);
            }
            let result = child.output().unwrap();
            assert!(
                result.status.success(),
                "preflight child failed: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(!identity.exists());
        }
    }
}
/// A server-owned verified peer; only the direct path observed by Iroh supplies it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectPeer {
    /// Observed remote UDP socket.
    pub remote: SocketAddr,
    /// Chosen receiving local UDP socket.
    pub local: SocketAddr,
}
/// A production request's retained current authority, checked even while the peer
/// is backpressured. Implementations must own their source and session context.
pub trait Authority: Send + Sync {
    /// Rechecks fresh account/device/network/source/operation authority.
    ///
    /// # Errors
    /// Refuses output whenever its retained authority is no longer current.
    fn check(&self)
    -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>>;
}
/// Server-private response extension, never serialized into media headers.
#[derive(Clone)]
pub struct ResponseAuthority(pub Arc<dyn Authority>);
/// Opaque closed authority error; transport logs never include private context.
pub type AuthorityError = anyhow::Error;
/// A generic refusal without account, credential, path or policy details.
#[must_use]
pub fn authority_refused() -> AuthorityError {
    anyhow::anyhow!("current media authority refused")
}
/// Joined terminal accounting; success is never inferred from a timeout.
#[derive(Default, Debug)]
pub struct StopReceipt {
    /// Connection tasks actually joined (including failed requests).
    pub joined_connections: usize,
    /// True only after endpoint.close completed and all owned tasks joined.
    pub endpoint_closed: bool,
}
/// Bound lifetime, not detached. Dropping the run future drops/aborts its JoinSet;
/// normal shutdown must use run's stop channel and retain its receipt.
pub struct Server {
    endpoint: Endpoint,
    local: SocketAddr,
    _identity: File,
    active: watch::Sender<usize>,
}
impl Server {
    /// Binds exactly the configured address using the private persistent identity.
    ///
    /// # Errors
    /// Refuses an invalid bind, unsafe/leased identity or failed endpoint setup.
    pub async fn bind(config: &Config) -> Result<Self> {
        ensure!(
            !config.bind_address.ip().to_canonical().is_unspecified()
                && !config.bind_address.ip().to_canonical().is_multicast(),
            "concrete Iroh bind required"
        );
        let (key, lease) = identity::open(&config.secret_key_file)?;
        let transport = iroh::endpoint::QuicTransportConfig::builder()
            .fixed_path()
            .max_concurrent_bidi_streams(1u32.into())
            .max_concurrent_uni_streams(0u32.into())
            .stream_receive_window((8u32 * 1024 * 1024).into())
            .receive_window((16u32 * 1024 * 1024).into())
            .send_window(16 * 1024 * 1024)
            .datagram_receive_buffer_size(None)
            .datagram_send_buffer_size(0)
            .max_idle_timeout(Some(IDLE.try_into()?))
            .build();
        let endpoint = Endpoint::builder(iroh::endpoint::presets::Minimal)
            .clear_ip_transports()
            .clear_relay_transports()
            .clear_address_lookup()
            .bind_addr(config.bind_address)?
            .secret_key(key)
            .relay_mode(RelayMode::Disabled)
            .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
            .transport_config(transport)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await?;
        let sockets = endpoint.bound_sockets();
        if sockets.len() != 1
            || sockets[0].ip() != config.bind_address.ip()
            || (config.bind_address.port() != 0 && sockets[0].port() != config.bind_address.port())
        {
            endpoint.close().await;
            anyhow::bail!("Iroh bound socket does not match operator configuration");
        }
        Ok(Self {
            endpoint,
            local: sockets[0],
            _identity: lease,
            active: watch::channel(0).0,
        })
    }
    /// Public identity and actual bound direct hint. This makes no reachability claim.
    #[must_use]
    pub fn address(&self) -> (String, SocketAddr) {
        (self.endpoint.id().to_string(), self.local)
    }
    /// Event-driven owned connection count, including drain and handshake.
    /// Zero is published after body/source/stream/permit destruction.
    #[must_use]
    pub fn active_connections(&self) -> watch::Receiver<usize> {
        self.active.subscribe()
    }
    /// Serves the fully composed production router; stop cancels and joins every
    /// owned connection before closing the endpoint and returning its receipt.
    ///
    /// # Errors
    /// Reports a connection task panic after cancelling/joining all remaining
    /// tasks and completing endpoint closure.
    pub async fn run(self, app: Router, mut stop: watch::Receiver<bool>) -> Result<StopReceipt> {
        let mut jobs = JoinSet::new();
        let permits = Arc::new(Semaphore::new(4));
        let mut receipt = StopReceipt::default();
        let (cancel, child_stop) = watch::channel(false);
        let mut task_failed = false;
        loop {
            let child_stop = child_stop.clone();
            tokio::select! { biased;
                _=stop.wait_for(|v|*v) => break,
                Some(result)=jobs.join_next(),if !jobs.is_empty()=> {
                    receipt.joined_connections+=1;
                    if result.is_err() { task_failed=true; break; }
                },
                incoming=self.endpoint.accept()=> {
                    let Some(incoming)=incoming else {break};
                    let Ok(permit)=permits.clone().try_acquire_owned() else {incoming.refuse();continue};
                    let app=app.clone();
                    let local=self.local;
                    self.active.send_modify(|n|*n+=1);
                    let active=Active(self.active.clone());
                    jobs.spawn(connection(incoming, app, child_stop, local, permit, active));
                }
            }
        }
        cancel.send_replace(true);
        while let Some(result) = jobs.join_next().await {
            task_failed |= result.is_err();
            receipt.joined_connections += 1;
        }
        self.endpoint.close().await;
        receipt.endpoint_closed = true;
        ensure!(
            !task_failed,
            "Iroh connection task failed after joined shutdown"
        );
        Ok(receipt)
    }
}
async fn connection(
    incoming: iroh::endpoint::Incoming,
    app: Router,
    mut stop: watch::Receiver<bool>,
    local: SocketAddr,
    permit: tokio::sync::OwnedSemaphorePermit,
    active: Active,
) {
    let _active = active;
    let _permit = permit;
    let IncomingAddr::Ip(remote) = incoming.remote_addr() else {
        incoming.refuse();
        return;
    };
    let conn = tokio::select! { biased;
        ()=async {let _=stop.wait_for(|v|*v).await;}=>return,
        result=tokio::time::timeout(Duration::from_secs(5),incoming)=>match result {Ok(Ok(conn))=>conn,_=>return},
    };
    let mut events = conn.path_events(); // Subscribe BEFORE the first snapshot.
    let peer = DirectPeer { remote, local };
    if observed(&conn, peer).is_err() {
        conn.close(1u32.into(), b"path unavailable");
        return;
    }
    tokio::select! { biased;
        ()=async {let _=stop.wait_for(|v|*v).await;}=>{},
        ()=path_changed(&conn,peer,&mut events)=>{},
        _=conn.closed()=>{},
        _=tokio::time::timeout(TOTAL,serve(&conn,app,peer))=>{},
    }
    conn.close(0u32.into(), b"media request ended");
}
async fn path_changed(
    conn: &Connection,
    peer: DirectPeer,
    events: &mut iroh::endpoint::PathEventStream,
) {
    while let Some(event) = events.next().await {
        // The subscription precedes the snapshot. Its initial notification may
        // describe the very same captured path; it supplies no new authority.
        let same = match event {
            iroh::endpoint::PathEvent::Opened {
                remote_addr,
                local_addr,
                ..
            }
            | iroh::endpoint::PathEvent::Selected {
                remote_addr,
                local_addr,
                ..
            } => {
                remote_addr == TransportAddr::Ip(peer.remote)
                    && matches!(local_addr,LocalTransportAddr::Ip(Some(ip)) if ip==peer.local.ip())
                    && observed(conn, peer).is_ok()
            }
            _ => false, // Closed, lagged, unknown or ambiguous observations fail closed.
        };
        if !same {
            conn.close(2u32.into(), b"network context changed");
            return;
        }
    }
    conn.close(2u32.into(), b"network context unavailable");
}
struct Active(watch::Sender<usize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.send_modify(|n| *n -= 1);
    }
}
fn observed(conn: &Connection, peer: DirectPeer) -> Result<()> {
    ensure!(conn.alpn() == ALPN, "ALPN");
    let paths = conn.paths();
    ensure!(paths.len() == 1, "ambiguous path");
    let path = paths
        .iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing path"))?;
    ensure!(
        path.is_selected()
            && path.remote_addr() == &TransportAddr::Ip(peer.remote)
            && matches!(path.local_addr(),LocalTransportAddr::Ip(Some(ip)) if *ip==peer.local.ip()),
        "changed path"
    );
    Ok(())
}
async fn serve(conn: &Connection, app: Router, peer: DirectPeer) -> Result<()> {
    let (mut send, mut recv) =
        tokio::time::timeout(Duration::from_secs(5), conn.accept_bi()).await??;
    let raw = tokio::time::timeout(Duration::from_secs(5), protocol::read(&mut recv)).await??;
    let mut request: Request<Body> = protocol::parse(&raw)?;
    observed(conn, peer)?;
    request.extensions_mut().insert(ConnectInfo(peer.remote));
    request.extensions_mut().insert(peer);
    let head = request.method() == axum::http::Method::HEAD;
    let response = tokio::time::timeout(Duration::from_secs(30), app.oneshot(request)).await??;
    ensure!(
        !response.status().is_redirection(),
        "media redirect refused"
    );
    let authority = response.extensions().get::<ResponseAuthority>().cloned();
    ensure!(
        !response.status().is_success() || authority.is_some(),
        "missing production authority"
    );
    let monitor = async {
        if let Some(authority) = &authority {
            loop {
                tokio::time::timeout(Duration::from_secs(2), authority.0.check()).await??;
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
        std::future::pending::<Result<()>>().await
    };
    tokio::select! { biased; result=monitor=>result, result=write_response(conn,&mut send,&mut recv,response,head,peer)=>result }
}
async fn write_response(
    conn: &Connection,
    send: &mut iroh::endpoint::SendStream,
    recv: &mut iroh::endpoint::RecvStream,
    response: axum::response::Response,
    head: bool,
    peer: DirectPeer,
) -> Result<()> {
    let mut headers = format!(
        "HTTP/1.1 {} Media\r\nConnection: close\r\n",
        response.status().as_u16()
    );
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::ACCEPT_RANGES,
        header::ETAG,
        header::LAST_MODIFIED,
        header::CACHE_CONTROL,
    ] {
        if let Some(value) = response.headers().get(&name) {
            let value = value.to_str()?;
            ensure!(
                value.len() <= 1024 && value.bytes().all(|b| (32..127).contains(&b)),
                "response header"
            );
            write!(headers, "{}: {value}\r\n", name.as_str())?;
        }
    }
    headers.push_str("\r\n");
    tokio::time::timeout(IDLE, send.write_all(headers.as_bytes())).await??;
    let expected = response
        .headers()
        .get(header::CONTENT_LENGTH)
        .map(|v| v.to_str()?.parse::<u64>().map_err(anyhow::Error::from))
        .transpose()?;
    let mut body = response.into_body().into_data_stream();
    let mut written = 0u64;
    if !head {
        while let Some(chunk) = tokio::time::timeout(IDLE, body.next()).await? {
            let chunk = chunk?;
            ensure!(chunk.len() <= 1024 * 1024, "response frame bound");
            for part in chunk.chunks(32768) {
                observed(conn, peer)?;
                written = written
                    .checked_add(part.len() as u64)
                    .ok_or_else(|| anyhow::anyhow!("length overflow"))?;
                ensure!(
                    expected.is_none_or(|n| written <= n),
                    "response length overflow"
                );
                tokio::time::timeout(IDLE, send.write_all(part)).await??;
            }
        }
        ensure!(
            expected.is_none_or(|n| written == n),
            "response length mismatch"
        );
    }
    send.finish()?;
    // Retain the actual body/source through application drain, within the same
    // task. stopped() is transport acknowledgement only, so it is not used here.
    tokio::time::timeout(IDLE, conn.closed()).await?;
    drop(body);
    let _ = recv;
    Ok(())
}
