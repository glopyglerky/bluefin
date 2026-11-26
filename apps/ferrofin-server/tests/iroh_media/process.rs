use super::*;
use std::collections::BTreeSet;
struct Seed {
    temp: tempfile::TempDir,
    path: String,
    viewer: String,
    hash: [u8; 32],
    size: usize,
}
struct Process {
    child: tokio::process::Child,
    output: tokio::task::JoinHandle<String>,
    base: String,
}
fn udp_sockets(pid: u32) -> usize {
    let mut fds = BTreeSet::new();
    for fd in std::fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .flatten()
    {
        if let Ok(link) = std::fs::read_link(fd.path())
            && let Some(inode) = link
                .to_string_lossy()
                .strip_prefix("socket:[")
                .and_then(|v| v.strip_suffix(']'))
        {
            fds.insert(inode.to_owned());
        }
    }
    ["udp", "udp6"]
        .iter()
        .map(|name| {
            std::fs::read_to_string(format!("/proc/{pid}/net/{name}"))
                .unwrap()
                .lines()
                .skip(1)
                .filter(|line| {
                    line.split_whitespace()
                        .nth(9)
                        .is_some_and(|inode| fds.contains(inode))
                })
                .count()
        })
        .sum()
}
async fn http_json(
    client: &reqwest::Client,
    method: reqwest::Method,
    base: &str,
    path: &str,
    token: &str,
    body: Value,
) -> (StatusCode, Value) {
    let r = client
        .request(method, format!("{base}{path}"))
        .header(header::AUTHORIZATION, auth(token))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = r.status();
    let bytes = r.bytes().await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn seed() -> Seed {
    let f = Fixture::new(512 * 1024).await;
    let (status, mut network) = call(
        &f.app,
        "GET",
        "/System/Configuration/network",
        &f.admin,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    network["AutoDiscovery"] = json!(false);
    assert!(
        call(
            &f.app,
            "POST",
            "/System/Configuration/network",
            &f.admin,
            network
        )
        .await
        .0
        .is_success()
    );
    let sqlite: String = sqlx::query_scalar("SELECT sqlite_version()")
        .fetch_one(f.db.pool())
        .await
        .unwrap();
    assert_eq!(sqlite, "3.54.1");
    eprintln!("actual selected SQLite runtime={sqlite}; libsqlite3-sys remains 0.39.1");
    let path = f.path();
    let Fixture {
        app,
        mut wired,
        db,
        viewer,
        hash,
        size,
        temp,
        ..
    } = f;
    wired.state.library.shutdown_scans().await;
    wired.task_manager.shutdown().await;
    for job in &wired.background {
        job.abort();
    }
    for job in wired.background.drain(..) {
        let _ = job.await;
    }
    drop((app, wired));
    db.close().await;
    Seed {
        temp,
        path,
        viewer,
        hash,
        size,
    }
}
fn command(root: &Path, port: u16, config: &Path, enabled: bool) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_ferrofin-server"));
    cmd.env_clear()
        .kill_on_drop(true)
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("TMPDIR", root)
        .env("FERROFIN_DATA_DIR", root.join("data"))
        .env("FERROFIN_CONFIG_DIR", root.join("config"))
        .env("FERROFIN_CACHE_DIR", root.join("cache"))
        .env("FERROFIN_WEB_DIR", root.join("web"))
        .env("FERROFIN_BIND_ADDR", "127.0.0.1")
        .env("FERROFIN_PORT", port.to_string())
        .env("FERROFIN_SERVER_NAME", "iroh-private-process-fixture")
        .env(
            "FERROFIN_STUDIOS_REPO_URL",
            ferrofin_server::config::OFFLINE_PROVIDER_URL,
        )
        .env(
            "FERROFIN_MUSICBRAINZ_URL",
            ferrofin_server::config::OFFLINE_PROVIDER_URL,
        )
        .env("FERROFIN_FFMPEG_PATH", ffmpeg_binary())
        .env(
            "FERROFIN_FFPROBE_PATH",
            std::env::var_os("BLUEFIN_TEST_FFPROBE").unwrap(),
        )
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit());
    if enabled {
        cmd.env("BLUEFIN_IROH_CONFIG", config);
    }
    cmd
}
impl Process {
    async fn start(root: &Path, config: &Path, enabled: bool) -> Self {
        use tokio::io::AsyncBufReadExt as _;
        let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let mut child = command(root, port, config, enabled).spawn().unwrap();
        let pid = child.id().unwrap();
        let mut lines = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
        let (ready, rx) = tokio::sync::oneshot::channel();
        let output = tokio::spawn(async move {
            let mut ready = Some(ready);
            let mut log = String::new();
            while let Some(line) = lines.next_line().await.unwrap() {
                if line.contains("ferrofin-server ready")
                    && let Some(ready) = ready.take()
                {
                    let _ = ready.send(());
                }
                log.push_str(&line);
                log.push('\n');
            }
            log
        });
        tokio::time::timeout(Duration::from_secs(30), rx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            udp_sockets(pid),
            usize::from(enabled),
            "exact process-owned UDP socket count"
        );
        assert_eq!(root.join("identity/key").exists(), enabled);
        Self {
            child,
            output,
            base: format!("http://127.0.0.1:{port}"),
        }
    }
    async fn stop(
        mut self,
        client: &reqwest::Client,
        admin: &str,
        enabled: bool,
        udp: Option<SocketAddr>,
    ) {
        assert!(
            http_json(
                client,
                reqwest::Method::POST,
                &self.base,
                "/System/Shutdown",
                admin,
                Value::Null
            )
            .await
            .0
            .is_success()
        );
        let status = tokio::time::timeout(Duration::from_secs(15), self.child.wait())
            .await
            .unwrap()
            .unwrap();
        let log = self.output.await.unwrap();
        assert!(status.success(), "production process failed: {log}");
        if enabled {
            assert!(
                log.contains("\"endpoint_closed\":true"),
                "actual joined endpoint receipt: {log}"
            );
        }
        if let Some(address) = udp {
            drop(std::net::UdpSocket::bind(address).unwrap());
        }
        eprintln!(
            "actual production process: enabled={enabled}, orderly exit={status}; persistent identity and selected socket verified"
        );
    }
}
async fn sessions(client: &reqwest::Client, base: &str) -> (String, String) {
    let mut tokens = Vec::new();
    for (name, password) in [
        ("iroh-viewer", "viewer-password"),
        ("iroh-admin", "synthetic-password"),
    ] {
        let (status, value) = http_json(
            client,
            reqwest::Method::POST,
            base,
            "/Users/AuthenticateByName",
            "",
            json!({"Username":name,"Pw":password}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        tokens.push(value["AccessToken"].as_str().unwrap().to_owned());
    }
    (tokens.remove(0), tokens.remove(0))
}
async fn basic_http(seed: &Seed, client: &reqwest::Client, base: &str, token: &str) -> Value {
    let (status, cap) = http_json(
        client,
        reqwest::Method::GET,
        base,
        "/Bluefin/Capabilities",
        token,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        http_json(
            client,
            reqwest::Method::GET,
            base,
            "/Bluefin/Capabilities",
            "",
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let response = client
        .get(format!("{base}{}", seed.path))
        .header(header::AUTHORIZATION, auth(token))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.bytes().await.unwrap();
    assert_eq!(bytes.len(), seed.size);
    assert_eq!(<[u8; 32]>::from(Sha256::digest(bytes)), seed.hash);
    cap
}
async fn opted_in(seed: &Seed, cap: &Value, token: &str) -> (SocketAddr, String, Vec<u8>) {
    let c = &cap["irohMediaTransport"];
    let id = c["endpointId"].as_str().unwrap().to_owned();
    assert_eq!(c["directOnly"], true);
    assert_eq!(c["alpn"], "bluefin/http/1");
    let address: SocketAddr = c["directAddresses"][0].as_str().unwrap().parse().unwrap();
    let identity = seed.temp.path().join("identity/key");
    assert_eq!(
        std::fs::metadata(&identity).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let key = std::fs::read(identity).unwrap();
    assert_eq!(key.len(), 32);
    let ep = endpoint(IpAddr::V4(Ipv4Addr::LOCALHOST)).await;
    let conn = ep
        .connect(
            EndpointAddr::new(id.parse().unwrap()).with_ip_addr(address),
            ALPN,
        )
        .await
        .unwrap();
    let (mut request_stream, mut recv) = conn.open_bi().await.unwrap();
    request_stream
        .write_all(raw("GET", &seed.path, &auth(token), "Range: bytes=0-1023\r\n").as_bytes())
        .await
        .unwrap();
    request_stream.finish().unwrap();
    assert!(read_head(&mut recv).await.starts_with("HTTP/1.1 206 "));
    assert_eq!(
        recv.read_to_end(1024).await.unwrap(),
        std::fs::read(seed.temp.path().join("music/Original.wav")).unwrap()[..1024]
    );
    conn.close(0u32.into(), b"production process application drained");
    drop((request_stream, recv, conn));
    ep.close().await;
    (address, id, key)
}
async fn revoke_and_restore(
    seed: &Seed,
    client: &reqwest::Client,
    base: &str,
    token: &str,
    admin: &str,
) {
    let uri = format!("/Users/{}", seed.viewer);
    let (_, me) = http_json(client, reqwest::Method::GET, base, &uri, admin, Value::Null).await;
    let mut policy = me["Policy"].clone();
    policy["EnableMediaPlayback"] = json!(false);
    let uri = format!("{uri}/Policy");
    assert!(
        http_json(
            client,
            reqwest::Method::POST,
            base,
            &uri,
            admin,
            policy.clone()
        )
        .await
        .0
        .is_success()
    );
    let (_, cap) = http_json(
        client,
        reqwest::Method::GET,
        base,
        "/Bluefin/Capabilities",
        token,
        Value::Null,
    )
    .await;
    assert!(cap.get("irohMediaTransport").is_none());
    assert_eq!(
        client
            .get(format!("{base}{}", seed.path))
            .header(header::AUTHORIZATION, auth(token))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    policy["EnableMediaPlayback"] = json!(true);
    assert!(
        http_json(client, reqwest::Method::POST, base, &uri, admin, policy)
            .await
            .0
            .is_success()
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn actual_production_process_default_off_opt_in_identity_projection_and_stop() {
    let seed = seed().await;
    let root = seed.temp.path();
    let config = root.join("iroh.json");
    std::fs::write(
        &config,
        serde_json::to_vec(
            &json!({"secret_key_file":root.join("identity/key"),"bind_address":"127.0.0.1:0"}),
        )
        .unwrap(),
    )
    .unwrap();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let mut persistent = None;
    for enabled in [false, true, true] {
        let process = Process::start(root, &config, enabled).await;
        let (token, admin) = sessions(&client, &process.base).await;
        let cap = basic_http(&seed, &client, &process.base, &token).await;
        let udp = if enabled {
            let (address, id, key) = opted_in(&seed, &cap, &token).await;
            if let Some((previous_id, previous_key)) = &persistent {
                assert_eq!(previous_id, &id);
                assert_eq!(previous_key, &key);
            }
            persistent = Some((id, key));
            revoke_and_restore(&seed, &client, &process.base, &token, &admin).await;
            Some(address)
        } else {
            assert!(cap.get("irohMediaTransport").is_none());
            None
        };
        process.stop(&client, &admin, enabled, udp).await;
    }
}
