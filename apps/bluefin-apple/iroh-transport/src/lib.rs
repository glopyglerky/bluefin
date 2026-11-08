//! Direct pinned Iroh HTTP streaming. No discovery, relays or persistent credentials.
use anyhow::{Result, anyhow, bail, ensure};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode};
use serde::Deserialize;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{Semaphore, oneshot},
    task::JoinSet,
};
pub const ALPN: &[u8] = b"bluefin/http/1";
const HEAD: usize = 16 * 1024;
const IDLE: Duration = Duration::from_secs(30);
const TOTAL: Duration = Duration::from_secs(6 * 60 * 60);
pub async fn endpoint() -> Result<Endpoint> {
    let mut transport = iroh::endpoint::TransportConfig::default();
    transport
        .max_concurrent_bidi_streams(1u32.into())
        .max_concurrent_uni_streams(0u32.into())
        .stream_receive_window((64u32 * 1024).into())
        .receive_window((256u32 * 1024).into())
        .send_window(256 * 1024);
    Ok(Endpoint::builder()
        .transport_config(transport)
        .clear_discovery()
        .relay_mode(RelayMode::Disabled)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await?)
}
async fn headers<R: AsyncRead + Unpin>(r: &mut R) -> Result<Vec<u8>> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut v = Vec::with_capacity(1024);
        loop {
            ensure!(v.len() < HEAD, "headers too large");
            v.push(r.read_u8().await?);
            if v.ends_with(b"\r\n\r\n") {
                return Ok(v);
            }
        }
    })
    .await
    .map_err(|_| anyhow!("header timeout"))?
}
#[derive(Clone)]
struct Request {
    method: String,
    path: String,
    range: Option<String>,
    auth: Option<String>,
}
fn path_valid(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && path.len() <= 8192
        && path
            .bytes()
            .all(|c| (33..127).contains(&c) && !b"%\\#".contains(&c))
        && !path
            .split('?')
            .next()
            .unwrap_or("")
            .split('/')
            .any(|s| s == "." || s == "..")
        && !path.split_once('?').is_some_and(|(_, q)| {
            q.split('&').any(|v| {
                let k = v.split('=').next().unwrap_or("").to_ascii_lowercase();
                [
                    "api_key",
                    "apikey",
                    "access_token",
                    "token",
                    "authorization",
                    "x-emby-token",
                ]
                .contains(&k.as_str())
            })
        })
}
fn auth_valid(v: &str) -> bool {
    !v.is_empty() && v.len() <= 4096 && v.bytes().all(|b| (32..127).contains(&b))
}
fn range_valid(v: &str) -> bool {
    let Some(v) = v.strip_prefix("bytes=") else {
        return false;
    };
    let Some((a, b)) = v.split_once('-') else {
        return false;
    };
    (!a.is_empty() || !b.is_empty())
        && [a, b]
            .iter()
            .all(|s| s.is_empty() || s.parse::<u64>().is_ok())
        && (a.is_empty() || b.is_empty() || a.parse::<u64>().unwrap() <= b.parse::<u64>().unwrap())
}
fn parse(raw: &[u8], host: &str) -> Result<Request> {
    let mut slots = [httparse::EMPTY_HEADER; 32];
    let mut p = httparse::Request::new(&mut slots);
    ensure!(p.parse(raw)?.is_complete(), "incomplete request");
    ensure!(p.version == Some(1), "HTTP/1.1 required");
    let method = p.method.unwrap_or("");
    ensure!(["GET", "HEAD"].contains(&method), "method");
    let path = p.path.unwrap_or("");
    ensure!(path_valid(path), "path");
    let mut seen = std::collections::HashSet::new();
    let mut auth = None;
    let mut range = None;
    let mut got_host = false;
    for h in p.headers {
        let name = h.name.to_ascii_lowercase();
        ensure!(seen.insert(name.clone()), "duplicate header");
        let value = std::str::from_utf8(h.value)?;
        ensure!(
            value.bytes().all(|b| (32..127).contains(&b)),
            "header value"
        );
        match name.as_str() {
            "host" => {
                ensure!(value == host, "host");
                got_host = true;
            }
            "authorization" => {
                ensure!(auth_valid(value), "auth");
                auth = Some(value.to_owned());
            }
            "range" => {
                ensure!(range_valid(value), "range");
                range = Some(value.to_owned());
            }
            "content-length" => ensure!(value == "0", "body"),
            "transfer-encoding" | "expect" | "upgrade" | "proxy-authorization" => {
                bail!("unsupported header")
            }
            _ => {}
        }
    }
    ensure!(got_host, "host missing");
    Ok(Request {
        method: method.to_owned(),
        path: path.to_owned(),
        range,
        auth,
    })
}
fn encode(r: &Request) -> Vec<u8> {
    let mut s = format!(
        "{} {} HTTP/1.1\r\nHost: bluefin\r\nConnection: close\r\n",
        r.method, r.path
    );
    if let Some(a) = &r.auth {
        s.push_str(&format!("Authorization: {a}\r\n"));
    }
    if let Some(a) = &r.range {
        s.push_str(&format!("Range: {a}\r\n"));
    }
    s.push_str("\r\n");
    s.into_bytes()
}
/// Exactly one trusted HTTPS origin and an item-specific media path prefix.
/// There is no redirect, proxy environment, arbitrary URL or credential query support.
#[derive(Clone)]
pub struct Origin {
    base: reqwest::Url,
    prefix: String,
    http: reqwest::Client,
}
impl Origin {
    pub fn new(origin: &str, prefix: &str) -> Result<Self> {
        Self::with_client(
            origin,
            prefix,
            reqwest::Client::builder()
                .https_only(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .read_timeout(IDLE)
                .build()?,
        )
    }
    fn with_client(origin: &str, prefix: &str, http: reqwest::Client) -> Result<Self> {
        let base = reqwest::Url::parse(origin)?;
        ensure!(
            base.scheme() == "https"
                && base.host_str().is_some()
                && base.username().is_empty()
                && base.password().is_none()
                && base.query().is_none()
                && base.fragment().is_none()
                && base.path() == "/",
            "HTTPS origin only"
        );
        ensure!(
            path_valid(prefix)
                && prefix.ends_with('/')
                && !prefix.contains('?')
                && (prefix.starts_with("/Videos/") || prefix.starts_with("/Audio/"))
                && prefix.split('/').filter(|s| !s.is_empty()).count() >= 2,
            "item media prefix required"
        );
        Ok(Self {
            base,
            prefix: prefix.to_owned(),
            http,
        })
    }
    async fn serve(
        &self,
        send: &mut iroh::endpoint::SendStream,
        recv: &mut iroh::endpoint::RecvStream,
    ) -> Result<()> {
        let raw = headers(recv).await?;
        let r = parse(&raw, "bluefin")?;
        // One request per stream. The peer must finish its request side.
        let mut extra = [0u8; 1];
        ensure!(
            tokio::time::timeout(Duration::from_secs(1), recv.read(&mut extra))
                .await??
                .is_none(),
            "request body"
        );
        ensure!(
            r.path
                .split('?')
                .next()
                .unwrap_or("")
                .starts_with(&self.prefix),
            "media scope"
        );
        let auth = r.auth.ok_or_else(|| anyhow!("authorization required"))?;
        let url = self.base.join(&r.path)?;
        ensure!(url.origin() == self.base.origin(), "origin");
        let mut req = self
            .http
            .request(reqwest::Method::from_bytes(r.method.as_bytes())?, url)
            .header("authorization", auth);
        if let Some(range) = r.range {
            req = req.header("range", range);
        }
        let mut response = req.send().await?;
        ensure!(
            [200, 206, 401, 403, 404, 416].contains(&response.status().as_u16()),
            "upstream status"
        );
        let mut head = format!(
            "HTTP/1.1 {} Media\r\nConnection: close\r\n",
            response.status().as_u16()
        );
        for name in [
            "content-type",
            "content-length",
            "content-range",
            "accept-ranges",
        ] {
            if let Some(v) = response.headers().get(name) {
                let v = v.to_str()?;
                ensure!(
                    v.len() <= 1024 && v.bytes().all(|b| (32..127).contains(&b)),
                    "response header"
                );
                head.push_str(&format!("{name}: {v}\r\n"));
            }
        }
        head.push_str("\r\n");
        tokio::time::timeout(IDLE, send.write_all(head.as_bytes())).await??;
        if r.method != "HEAD" {
            while let Some(chunk) = response.chunk().await? {
                for part in chunk.chunks(32768) {
                    tokio::time::timeout(IDLE, send.write_all(part)).await??;
                }
            }
        }
        send.finish()?;
        Ok(())
    }
}
/// Ephemeral adapter lifetime. Caller owns the endpoint identity and the shutdown signal.
/// Caller must retain the returned endpoint id and pin it in the client configuration.
pub async fn run_adapter(ep: Endpoint, origin: Origin, mut stop: oneshot::Receiver<()>) {
    let limit = Arc::new(Semaphore::new(4));
    let mut jobs = JoinSet::new();
    loop {
        tokio::select! {
            _=&mut stop => break,
            Some(_)=jobs.join_next(), if !jobs.is_empty()=>{},
            incoming=ep.accept()=> {let Some(incoming)=incoming else {break};
                let Ok(permit)=limit.clone().try_acquire_owned() else {incoming.refuse();continue};
                let origin=origin.clone();jobs.spawn(async move {
                    let _permit=permit;
                    let _=tokio::time::timeout(TOTAL,async {
                        let conn=tokio::time::timeout(Duration::from_secs(5),incoming).await??;
                        let (mut send,mut recv)=tokio::time::timeout(Duration::from_secs(5),conn.accept_bi()).await??;
                        let served = tokio::select! {_ = conn.closed()=>false, result = origin.serve(&mut send,&mut recv)=>result.is_ok()};
                        // Keep the response alive until delivery is acknowledged or reset.
                        if served { let _=tokio::time::timeout(IDLE,send.stopped()).await; }
                        conn.close(0u32.into(),b"complete");Ok::<_,anyhow::Error>(())
                    }).await;
                });
            }
        }
    }
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
    let _ = tokio::time::timeout(Duration::from_secs(1), ep.close()).await;
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientConfig {
    pub peer_id: String,
    pub peer_address: SocketAddr,
    pub authorization: String,
}
async fn forward(
    ep: Endpoint,
    peer: EndpointId,
    addr: SocketAddr,
    mut local: tokio::net::TcpStream,
    host: String,
    cap: String,
    auth: Arc<String>,
) -> Result<()> {
    let raw = headers(&mut local).await?;
    let mut r = parse(&raw, &host)?;
    let prefix = format!("/{cap}");
    ensure!(r.path.starts_with(&(prefix.clone() + "/")), "capability");
    ensure!(
        r.auth.is_none(),
        "authorization comes from viewer configuration"
    );
    r.path = r.path[prefix.len()..].to_owned();
    r.auth = Some((*auth).clone());
    let conn = tokio::time::timeout(
        Duration::from_secs(5),
        ep.connect(EndpointAddr::new(peer).with_ip_addr(addr), ALPN),
    )
    .await??;
    let result = async {
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(&encode(&r)).await?;
        send.finish()?;
        let mut buf = [0u8; 32768];
        loop {
            let mut probe = [0u8; 1];
            let n = tokio::select! { v = tokio::time::timeout(IDLE, AsyncReadExt::read(&mut recv, &mut buf)) => v??, _ = local.peek(&mut probe) => bail!("local closed or pipelined") };
            if n == 0 {
                break;
            }
            tokio::time::timeout(IDLE, local.write_all(&buf[..n])).await??;
        }
        Ok(())
    }
    .await;
    conn.close(0u32.into(), b"complete");
    result
}
struct Tunnel {
    stop: oneshot::Sender<()>,
    join: std::thread::JoinHandle<()>,
}
static TUNNELS: OnceLock<Mutex<HashMap<u64, Tunnel>>> = OnceLock::new();
fn tunnels() -> &'static Mutex<HashMap<u64, Tunnel>> {
    TUNNELS.get_or_init(Default::default)
}
fn start(config: ClientConfig) -> Result<(u64, String)> {
    ensure!(auth_valid(&config.authorization), "authorization");
    let peer: EndpointId = config.peer_id.parse()?;
    let addr = config.peer_address;
    ensure!(
        addr.port() != 0 && !addr.ip().is_unspecified() && !addr.ip().is_multicast(),
        "address"
    );
    let mut registry = tunnels().lock().map_err(|_| anyhow!("registry"))?;
    ensure!(registry.len() < 2, "busy");
    let cap = rand::random::<[u8; 32]>()
        .iter()
        .fold(String::new(), |mut s, b| {
            use std::fmt::Write;
            write!(&mut s, "{b:02x}").unwrap();
            s
        });
    let mut id = rand::random::<u64>();
    while id == 0 || registry.contains_key(&id) {
        id = rand::random();
    }
    let (stop, mut rx) = oneshot::channel();
    let (ready, wait) = std::sync::mpsc::sync_channel(1);
    let join = std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
        else {
            return;
        };
        rt.block_on(async move {
            let Ok(ep)=endpoint().await else {return};let Ok(listener)=TcpListener::bind("127.0.0.1:0").await else {let _ = tokio::time::timeout(Duration::from_secs(1), ep.close()).await;return};
            let host=listener.local_addr().unwrap().to_string();let url=format!("http://{host}/{cap}");
            if ready.send(url).is_err(){let _ = tokio::time::timeout(Duration::from_secs(1), ep.close()).await;return;}
            static LIMIT:OnceLock<Arc<Semaphore>>=OnceLock::new();let limit=LIMIT.get_or_init(||Arc::new(Semaphore::new(4))).clone();let auth=Arc::new(config.authorization);let mut jobs=JoinSet::new();
            loop {tokio::select! {
                _=&mut rx => break,
                Some(_)=jobs.join_next(), if !jobs.is_empty()=>{},
                accepted=listener.accept()=>{let Ok((local,_))=accepted else {break};
                    let Ok(permit)=limit.clone().try_acquire_owned() else {continue};
                    let (ep,host,cap,auth)=(ep.clone(),host.clone(),cap.clone(),auth.clone());
                    jobs.spawn(async move {let _permit=permit;let _=tokio::time::timeout(TOTAL,forward(ep,peer,addr,local,host,cap,auth)).await;});
                }
            }}jobs.abort_all();while jobs.join_next().await.is_some(){} let _ = tokio::time::timeout(Duration::from_secs(1), ep.close()).await;
        });
    });
    match wait.recv_timeout(Duration::from_secs(10)) {
        Ok(url) => {
            registry.insert(id, Tunnel { stop, join });
            Ok((id, url))
        }
        Err(_) => {
            let _ = stop.send(());
            let _ = join.join();
            bail!("startup failed")
        }
    }
}
fn stop(id: u64) {
    let tunnel = tunnels().lock().ok().and_then(|mut t| t.remove(&id));
    if let Some(t) = tunnel {
        let _ = t.stop.send(());
        let _ = t.join.join();
    }
}
/// # Safety
/// `input` must point to `len` readable bytes. Returns an owned UTF-8 JSON C string.
/// Call from a background thread. Never log input or returned capability URLs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bluefin_iroh_start(input: *const u8, len: usize) -> *mut std::ffi::c_char {
    let result = std::panic::catch_unwind(|| {
        ensure!(!input.is_null() && len <= 16384, "input");
        let bytes = unsafe { std::slice::from_raw_parts(input, len) };
        let config = serde_json::from_slice(bytes)?;
        let (id, url) = start(config)?;
        Ok::<_, anyhow::Error>(serde_json::json!({"ok":true,"id":id,"baseUrl":url}))
    });
    let value=result.ok().and_then(Result::ok).unwrap_or_else(||serde_json::json!({"ok":false,"error":"Iroh tunnel unavailable or invalid configuration"}));
    std::ffi::CString::new(value.to_string())
        .unwrap()
        .into_raw()
}
#[unsafe(no_mangle)]
pub extern "C" fn bluefin_iroh_stop(id: u64) {
    let _ = std::panic::catch_unwind(|| stop(id));
}
/// # Safety
/// `p` must be null or a live pointer returned by bluefin_iroh_start, freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bluefin_iroh_free(p: *mut std::ffi::c_char) {
    if !p.is_null() {
        drop(unsafe { std::ffi::CString::from_raw(p) });
    }
}
#[cfg(test)]
mod tests;
