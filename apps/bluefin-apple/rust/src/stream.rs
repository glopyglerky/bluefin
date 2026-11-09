//! Capability-only, origin-pinned loopback media server. No caller-supplied upstream URL.
use super::{err, Result};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{oneshot, Semaphore},
    task::JoinSet,
};
use url::Url;
const PLAYLIST_MAX: usize = 1024 * 1024;
const ROUTES_MAX: usize = 4096;
struct Proxy {
    stop: oneshot::Sender<()>,
    join: thread::JoinHandle<()>,
}
static PROXIES: OnceLock<Mutex<HashMap<String, Proxy>>> = OnceLock::new();
static STREAMS: OnceLock<Arc<Semaphore>> = OnceLock::new();
fn proxies() -> &'static Mutex<HashMap<String, Proxy>> {
    PROXIES.get_or_init(Default::default)
}
struct Session {
    origin: Url,
    prefix: String,
    auth: String,
    tunnel: Option<super::iroh_transport::Tunnel>,
    local: String,
    routes: Mutex<Vec<Url>>,
}
impl Session {
    fn route(&self, base: &Url, raw: &str) -> Result<String> {
        if raw.len() > 16384 || raw.contains(['\\', '\r', '\n']) || raw.contains("{$") {
            return Err(err("Invalid media route"));
        }
        // URL resolution permits relative parents only while the resolved item prefix stays pinned.
        if raw.split(['?', '#']).next().unwrap_or("").contains('%') {
            return Err(err("Encoded media paths unsupported"));
        }
        let mut u = base.join(raw).map_err(|_| err("Invalid media route"))?;
        if u.origin() != self.origin.origin()
            || !u.username().is_empty()
            || u.password().is_some()
            || u.fragment().is_some()
            || !u
                .path()
                .to_ascii_lowercase()
                .starts_with(&self.prefix.to_ascii_lowercase())
        {
            return Err(err("Media route outside session"));
        }
        let pairs: Vec<(String, String)> = u
            .query_pairs()
            .filter(|(k, _)| !credential(k))
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        u.set_query(None);
        if !pairs.is_empty() {
            u.query_pairs_mut().extend_pairs(pairs);
        }
        let mut routes = self.routes.lock().map_err(|_| err("Proxy unavailable"))?;
        let id = if let Some(i) = routes.iter().position(|x| x == &u) {
            i
        } else {
            if routes.len() >= ROUTES_MAX {
                return Err(err("Too many media routes"));
            }
            routes.push(u);
            routes.len() - 1
        };
        Ok(format!("{}/{id}", self.local))
    }
    fn playlist(&self, base: &Url, input: &[u8]) -> Result<Vec<u8>> {
        let s = std::str::from_utf8(input).map_err(|_| err("Invalid playlist"))?;
        if !s.starts_with("#EXTM3U\n") && !s.starts_with("#EXTM3U\r\n") {
            return Err(err("Invalid playlist"));
        }
        let mut out = String::new();
        for line in s.lines() {
            if line.len() > 16384 || line.chars().any(|c| c.is_control()) {
                return Err(err("Invalid playlist line"));
            }
            if !line.starts_with('#') && !line.is_empty() {
                out.push_str(&self.route(base, line)?);
            } else if let Some((tag, attrs)) = line.split_once(':').filter(|(t, _)| {
                matches!(
                    *t,
                    "#EXT-X-KEY"
                        | "#EXT-X-SESSION-KEY"
                        | "#EXT-X-MAP"
                        | "#EXT-X-MEDIA"
                        | "#EXT-X-I-FRAME-STREAM-INF"
                )
            }) {
                out.push_str(tag);
                out.push(':');
                let mut rest = attrs;
                let mut first = true;
                let mut keys = Vec::new();
                let mut no_key = false;
                while !rest.is_empty() {
                    let (key, tail) = rest
                        .split_once('=')
                        .ok_or_else(|| err("Malformed playlist attributes"))?;
                    if key.is_empty()
                        || !key
                            .bytes()
                            .all(|b| b.is_ascii_uppercase() || b == b'-' || b.is_ascii_digit())
                        || keys.contains(&key)
                    {
                        return Err(err("Malformed playlist attributes"));
                    }
                    keys.push(key);
                    let (value, next, quoted) = if let Some(t) = tail.strip_prefix('"') {
                        let end = t.find('"').ok_or_else(|| err("Malformed playlist quote"))?;
                        (&t[..end], &t[end + 1..], true)
                    } else {
                        let end = tail.find(',').unwrap_or(tail.len());
                        (&tail[..end], &tail[end..], false)
                    };
                    if value.is_empty() || value.contains(['\\', '"']) {
                        return Err(err("Unsupported playlist attribute"));
                    }
                    if !first {
                        out.push(',');
                    }
                    first = false;
                    out.push_str(key);
                    out.push('=');
                    if key == "METHOD" && value == "NONE" {
                        no_key = true;
                    }
                    if (key == "KEYFORMAT" && value != "identity")
                        || (key == "METHOD" && !matches!(value, "NONE" | "AES-128"))
                    {
                        return Err(err("Unsupported playlist encryption"));
                    }
                    if key == "URI" {
                        if !quoted {
                            return Err(err("Unquoted playlist URI"));
                        }
                        out.push('"');
                        out.push_str(&self.route(base, value)?);
                        out.push('"');
                    } else {
                        if key.contains("URI") || value.contains("://") {
                            return Err(err("Unsupported URI attribute"));
                        }
                        if quoted {
                            out.push('"');
                        }
                        out.push_str(value);
                        if quoted {
                            out.push('"');
                        }
                    }
                    if next.is_empty() {
                        break;
                    }
                    rest = next
                        .strip_prefix(',')
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| err("Malformed playlist attributes"))?;
                }
                if attrs.is_empty()
                    || (matches!(tag, "#EXT-X-KEY" | "#EXT-X-SESSION-KEY")
                        && !keys.contains(&"METHOD"))
                    || (matches!(tag, "#EXT-X-MAP" | "#EXT-X-I-FRAME-STREAM-INF")
                        && !keys.contains(&"URI"))
                    || (matches!(tag, "#EXT-X-KEY" | "#EXT-X-SESSION-KEY")
                        && !no_key
                        && !keys.contains(&"URI"))
                {
                    return Err(err("Missing playlist attribute"));
                }
            } else {
                // Unknown URI-bearing extensions, variable expansion and content steering fail closed.
                if line.contains("URI")
                    || line.contains("{$")
                    || line.starts_with("#EXT-X-DEFINE")
                    || line.starts_with("#EXT-X-CONTENT-STEERING")
                {
                    return Err(err("Unsupported playlist extension"));
                }
                out.push_str(line);
            }
            out.push('\n');
            if out.len() > PLAYLIST_MAX * 4 {
                return Err(err("Rewritten playlist too large"));
            }
        }
        Ok(out.into_bytes())
    }
}
pub(super) fn credential(k: &str) -> bool {
    matches!(
        k.to_ascii_lowercase().as_str(),
        "api_key"
            | "apikey"
            | "access_token"
            | "token"
            | "authorization"
            | "x-emby-token"
            | "x-mediabrowser-token"
    )
}
#[cfg(test)]
pub(super) fn start(remote: Url, prefix: String, auth: String) -> Result<(String, String)> {
    start_with_transport(remote, prefix, auth, None)
}
pub(super) fn start_with_transport(
    remote: Url,
    prefix: String,
    auth: String,
    tunnel: Option<super::iroh_transport::Tunnel>,
) -> Result<(String, String)> {
    let mut map = proxies().lock().map_err(|_| err("Proxy unavailable"))?;
    if map.len() >= 2 {
        return Err(err("Playback proxy limit reached"));
    }
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|_| err("Random source unavailable"))?;
    let id = random
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").map_err(|_| err("Loopback unavailable"))?;
    listener
        .set_nonblocking(true)
        .map_err(|_| err("Loopback unavailable"))?;
    let address = listener
        .local_addr()
        .map_err(|_| err("Loopback unavailable"))?;
    let local = format!("http://{address}/{id}");
    let state = Arc::new(Session {
        origin: remote.clone(),
        prefix,
        auth,
        tunnel,
        local,
        routes: Mutex::new(Vec::new()),
    });
    let url = state.route(&remote, remote.as_str())?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| err("Proxy runtime unavailable"))?;
    let (stop, mut stopped) = oneshot::channel();
    let join=thread::Builder::new().name("bluefin-playback".into()).spawn(move || {
        runtime.block_on(async move {
            let Ok(listener)=TcpListener::from_std(listener) else {return;};
            let Ok(client)=reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(5)).read_timeout(Duration::from_secs(30)).build() else {return;};
            let mut jobs=JoinSet::new();
            loop {tokio::select! {
                _=&mut stopped=>break,
                Some(_)=jobs.join_next(), if !jobs.is_empty()=>{},
                accepted=listener.accept()=> {
                    let Ok((mut socket,_))=accepted else {break;};
                    let slots=STREAMS.get_or_init(|| Arc::new(Semaphore::new(4))).clone();
                    if let Ok(lease)=slots.try_acquire_owned() {
                        let state=state.clone();let client=client.clone();
                        jobs.spawn(async move {let _lease=lease; let _=tokio::time::timeout(Duration::from_secs(6*60*60),serve(socket,state,client)).await;});
                    } else {let _=tokio::time::timeout(Duration::from_millis(100),socket.write_all(b"HTTP/1.1 503 Busy\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")).await;}
                }
            }}
            jobs.abort_all();while jobs.join_next().await.is_some() {}
        });
    }).map_err(|_|err("Proxy thread unavailable"))?;
    map.insert(id.clone(), Proxy { stop, join });
    Ok((id, url))
}
pub(super) fn stop(id: &str) -> Result<()> {
    // Keep the slot until the thread has exited, including all sockets and upstream requests.
    let mut map = proxies().lock().map_err(|_| err("Proxy unavailable"))?;
    if let Some(p) = map.remove(id) {
        let _ = p.stop.send(());
        p.join
            .join()
            .map_err(|_| err("Proxy stopped with failure"))?;
    }
    Ok(())
}
async fn serve(
    mut socket: TcpStream,
    state: Arc<Session>,
    client: reqwest::Client,
) -> std::io::Result<()> {
    let mut started = false;
    let result = transfer(&mut socket, &state, &client, &mut started).await;
    if result.is_err() && !started {
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            socket.write_all(
                b"HTTP/1.1 400 Rejected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            ),
        )
        .await;
    }
    Ok(())
}
async fn transfer(
    socket: &mut TcpStream,
    state: &Session,
    client: &reqwest::Client,
    started: &mut bool,
) -> Result<()> {
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let b = socket.read_u8().await.map_err(|_| err("Request failed"))?;
            bytes.push(b);
            if bytes.len() > 16384 {
                return Err(err("Headers too large"));
            }
            if bytes.ends_with(b"\r\n\r\n") {
                return Ok(());
            }
        }
    })
    .await
    .map_err(|_| err("Header timeout"))??;
    let s = std::str::from_utf8(&bytes).map_err(|_| err("Invalid request"))?;
    let mut lines = s.split("\r\n");
    let first = lines.next().ok_or_else(|| err("Invalid request"))?;
    let parts = first.split(' ').collect::<Vec<_>>();
    if parts.len() != 3 || !matches!(parts[0], "GET" | "HEAD") || parts[2] != "HTTP/1.1" {
        return Err(err("Unsupported request"));
    }
    let local = Url::parse(&state.local).map_err(|_| err("Invalid local route"))?;
    let prefix = format!("{}/", local.path());
    let id = parts[1]
        .strip_prefix(&prefix)
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| err("Bad capability"))?
        .parse::<usize>()
        .map_err(|_| err("Bad route"))?;
    let remote = state
        .routes
        .lock()
        .map_err(|_| err("Proxy unavailable"))?
        .get(id)
        .cloned()
        .ok_or_else(|| err("Unknown route"))?;
    let expected = format!("127.0.0.1:{}", local.port().unwrap_or(80));
    let mut host = false;
    let mut range = None;
    let mut seen = Vec::new();
    for line in lines.take_while(|l| !l.is_empty()) {
        let (k, v) = line
            .split_once(':')
            .ok_or_else(|| err("Malformed header"))?;
        let k = k.to_ascii_lowercase();
        let v = v.trim();
        if k.is_empty()
            || !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || v.bytes().any(|b| b < 32 || b == 127)
            || seen.contains(&k)
        {
            return Err(err("Invalid header"));
        }
        seen.push(k.clone());
        match k.as_str() {
            "host" => {
                if v != expected {
                    return Err(err("Invalid host"));
                }
                host = true;
            }
            "range" => {
                let r = v
                    .strip_prefix("bytes=")
                    .ok_or_else(|| err("Invalid range"))?;
                let (a, b) = r.split_once('-').ok_or_else(|| err("Invalid range"))?;
                if (a.is_empty() && b.is_empty())
                    || !a.bytes().chain(b.bytes()).all(|b| b.is_ascii_digit())
                {
                    return Err(err("Invalid range"));
                }
                range = Some(v.to_owned());
            }
            "content-length" if v == "0" => {}
            "content-length" | "transfer-encoding" | "authorization" | "cookie" | "origin" => {
                return Err(err("Unsupported header"))
            }
            _ => {}
        }
    }
    if !host {
        return Err(err("Missing host"));
    }
    let head = parts[0] == "HEAD";
    let mut request = client
        .request(
            if head {
                reqwest::Method::HEAD
            } else {
                reqwest::Method::GET
            },
            match &state.tunnel {
                Some(tunnel) => tunnel.fetch_url(&remote)?,
                None => remote.clone(),
            },
        )
        .header("Accept-Encoding", "identity");
    if state.tunnel.is_none() {
        request = request.header("Authorization", &state.auth);
    }
    if let Some(r) = range {
        request = request.header("Range", r);
    }
    let mut response = request.send().await.map_err(|_| err("Upstream failed"))?;
    let status = response.status();
    if !matches!(status.as_u16(), 200 | 206 | 416) {
        return Err(err("Upstream status rejected"));
    }
    let playlist = remote.path().to_ascii_lowercase().ends_with(".m3u8")
        || response
            .headers()
            .get("content-type")
            .and_then(|x| x.to_str().ok())
            .is_some_and(|s| s.contains("mpegurl"));
    let mut headers = format!(
        "HTTP/1.1 {} OK\r\nConnection: close\r\nCache-Control: no-store\r\n",
        status.as_u16()
    );
    if playlist && status.as_u16() != 200 {
        return Err(err("Partial playlist unsupported"));
    }
    if playlist && !head {
        let mut data = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| err("Playlist failed"))? {
            if data.len() + chunk.len() > PLAYLIST_MAX {
                return Err(err("Playlist too large"));
            }
            data.extend_from_slice(&chunk);
        }
        let data = state.playlist(&remote, &data)?;
        headers.push_str(&format!(
            "Content-Type: application/vnd.apple.mpegurl\r\nContent-Length: {}\r\n\r\n",
            data.len()
        ));
        *started = true;
        write(socket, headers.as_bytes()).await?;
        write(socket, &data).await?;
    } else {
        for key in [
            "content-type",
            "content-length",
            "content-range",
            "accept-ranges",
        ] {
            if playlist && key != "content-type" {
                continue;
            }
            if let Some(v) = response.headers().get(key) {
                let v = v.to_str().map_err(|_| err("Invalid response header"))?;
                if v.contains(['\r', '\n']) {
                    return Err(err("Invalid response header"));
                }
                headers.push_str(&format!("{key}: {v}\r\n"));
            }
        }
        headers.push_str("\r\n");
        *started = true;
        write(socket, headers.as_bytes()).await?;
        if !head {
            while let Some(chunk) = response.chunk().await.map_err(|_| err("Stream failed"))? {
                for block in chunk.chunks(32768) {
                    write(socket, block).await?;
                }
            }
        }
    }
    Ok(())
}
async fn write(socket: &mut TcpStream, bytes: &[u8]) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(30), socket.write_all(bytes))
        .await
        .map_err(|_| err("Player write timeout"))?
        .map_err(|_| err("Player disconnected"))
}
#[cfg(test)]
pub(crate) mod tests;
