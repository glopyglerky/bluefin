use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
async fn fixture() -> (Origin, tokio::task::JoinHandle<()>, Arc<AtomicUsize>) {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let key =
        tokio_rustls::rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
    let tls = tokio_rustls::rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.cert.der().clone()], key.into())
        .unwrap();
    let accept = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let http = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .add_root_certificate(reqwest::Certificate::from_der(cert.cert.der()).unwrap())
        .connect_timeout(Duration::from_secs(5))
        .read_timeout(IDLE)
        .build()
        .unwrap();
    let origin =
        Origin::with_client(&format!("https://localhost:{port}"), "/Videos/test/", http).unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let active2 = active.clone();
    let job = tokio::spawn(async move {
        let mut jobs = JoinSet::new();
        loop {
            tokio::select! {
                Some(_)=jobs.join_next(),if !jobs.is_empty()=>{},
                v=listener.accept()=>{let (tcp,_)=v.unwrap();let accept=accept.clone();let active=active2.clone();jobs.spawn(async move {
                    let mut s=accept.accept(tcp).await.unwrap();let raw=headers(&mut s).await.unwrap();let raw=String::from_utf8(raw).unwrap();
                    if !raw.contains("authorization: Bearer synthetic-viewer\r\n") {s.write_all(b"HTTP/1.1 401 Denied\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();return;}
                    if raw.contains("/stall") {active.fetch_add(1,Ordering::SeqCst);let mut b=[0];let _=s.read(&mut b).await;active.fetch_sub(1,Ordering::SeqCst);return;}
                    if raw.contains("range: bytes=99-100\r\n") {s.write_all(b"HTTP/1.1 416 Range\r\nContent-Length: 0\r\nContent-Range: bytes */10\r\nConnection: close\r\n\r\n").await.unwrap();return;}
            let range=raw.contains("range: bytes=2-5\r\n");let body=if range {"2345"} else {"0123456789"};
                    let head=format!("HTTP/1.1 {} OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n{}Connection: close\r\n\r\n",if range {206}else{200},body.len(),if range {"Content-Range: bytes 2-5/10\r\n"}else{""});
                    s.write_all(head.as_bytes()).await.unwrap();if !raw.starts_with("HEAD") {s.write_all(body.as_bytes()).await.unwrap();}s.shutdown().await.unwrap();
                });}
            }
        }
    });
    (origin, job, active)
}
async fn request(
    peer: EndpointId,
    addr: SocketAddr,
    method: &str,
    path: &str,
    auth: Option<&str>,
    range: Option<&str>,
) -> Result<Vec<u8>> {
    let ep = endpoint().await?;
    let conn = tokio::time::timeout(
        Duration::from_secs(3),
        ep.connect(EndpointAddr::new(peer).with_ip_addr(addr), ALPN),
    )
    .await??;
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(&encode(&Request {
        method: method.into(),
        path: path.into(),
        auth: auth.map(str::to_owned),
        range: range.map(str::to_owned),
    }))
    .await?;
    send.finish()?;
    let result = tokio::time::timeout(Duration::from_secs(3), recv.read_to_end(65536))
        .await?
        .map_err(Into::into);
    for path in conn.paths().iter() {
        if let Some(rtt) = conn.rtt(path.id()) {
            println!("IROH_FIXTURE_RTT_US {}", rtt.as_micros());
        }
    }
    conn.close(0u32.into(), b"test");
    ep.close().await;
    result
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_tls_origin_over_pinned_iroh() {
    let (origin, fixture, active) = fixture().await;
    let ep = endpoint().await.unwrap();
    let peer = ep.id();
    let port = ep.addr().ip_addrs().next().unwrap().port();
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let (stop, rx) = oneshot::channel();
    let adapter = tokio::spawn(run_adapter(ep, origin, rx));
    let b = request(
        peer,
        addr,
        "GET",
        "/Videos/test/file",
        Some("Bearer synthetic-viewer"),
        None,
    )
    .await
    .unwrap();
    assert!(b.ends_with(b"0123456789"));
    let b = request(
        peer,
        addr,
        "GET",
        "/Videos/test/file",
        Some("Bearer synthetic-viewer"),
        Some("bytes=2-5"),
    )
    .await
    .unwrap();
    assert!(b.starts_with(b"HTTP/1.1 206"));
    assert!(b.ends_with(b"2345"));
    let b = request(
        peer,
        addr,
        "GET",
        "/Videos/test/file",
        Some("Bearer synthetic-viewer"),
        Some("bytes=99-100"),
    )
    .await
    .unwrap();
    assert!(b.starts_with(b"HTTP/1.1 416"));
    let b = request(
        peer,
        addr,
        "HEAD",
        "/Videos/test/file",
        Some("Bearer synthetic-viewer"),
        None,
    )
    .await
    .unwrap();
    assert!(b.ends_with(b"\r\n\r\n"));
    let b = request(
        peer,
        addr,
        "GET",
        "/Videos/test/file",
        Some("Bearer wrong"),
        None,
    )
    .await
    .unwrap();
    assert!(b.starts_with(b"HTTP/1.1 401"));
    assert!(
        request(
            peer,
            addr,
            "GET",
            "/System/Info",
            Some("Bearer synthetic-viewer"),
            None
        )
        .await
        .map_or(true, |b| b.is_empty())
    );
    assert!(
        request(peer, addr, "GET", "/Videos/test/file", None, None)
            .await
            .map_or(true, |b| b.is_empty())
    );
    let wrong = endpoint().await.unwrap();
    assert!(
        request(
            wrong.id(),
            addr,
            "GET",
            "/Videos/test/file",
            Some("Bearer synthetic-viewer"),
            None
        )
        .await
        .is_err()
    );
    wrong.close().await;
    // Actual local tunnel, capability URL, viewer authorization injection and stop.
    let (id,url)=tokio::task::spawn_blocking(move|| {
        let input=serde_json::json!({"peer_id":peer.to_string(),"peer_address":addr.to_string(),"authorization":"Bearer synthetic-viewer"}).to_string();
        let ptr=unsafe{bluefin_iroh_start(input.as_ptr(),input.len())};
        let v:serde_json::Value=serde_json::from_str(unsafe{std::ffi::CStr::from_ptr(ptr)}.to_str().unwrap()).unwrap();
        unsafe{bluefin_iroh_free(ptr)};assert_eq!(v["ok"],true);
        (v["id"].as_u64().unwrap(),v["baseUrl"].as_str().unwrap().to_owned())
    }).await.unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let b = client
        .get(format!("{url}/Videos/test/file"))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(&b[..], b"0123456789");
    assert!(
        client
            .get(format!(
                "{}/wrong/Videos/test/file",
                url.rsplit_once('/').unwrap().0
            ))
            .send()
            .await
            .is_err()
    );
    let address = reqwest::Url::parse(&url).unwrap();
    let mut oversized = tokio::net::TcpStream::connect(("127.0.0.1", address.port().unwrap()))
        .await
        .unwrap();
    let _ = oversized.write_all(&vec![b'x'; HEAD + 1]).await;
    let mut b = [0; 1];
    assert!(
        tokio::time::timeout(Duration::from_secs(2), oversized.read(&mut b))
            .await
            .unwrap()
            .map_or(true, |n| n == 0)
    );
    let (second, _) = tokio::task::spawn_blocking(move || {
        start(ClientConfig {
            peer_id: peer.to_string(),
            peer_address: addr,
            authorization: "Bearer synthetic-viewer".into(),
        })
    })
    .await
    .unwrap()
    .unwrap();
    assert!(
        tokio::task::spawn_blocking(move || start(ClientConfig {
            peer_id: peer.to_string(),
            peer_address: addr,
            authorization: "Bearer synthetic-viewer".into()
        }))
        .await
        .unwrap()
        .is_err()
    );
    tokio::task::spawn_blocking(move || bluefin_iroh_stop(second))
        .await
        .unwrap();
    let stalled = tokio::spawn({
        let c = client.clone();
        let url = url.clone();
        async move { c.get(format!("{url}/Videos/test/stall")).send().await }
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while active.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap();
    stalled.abort();
    let _ = stalled.await;
    tokio::time::timeout(Duration::from_secs(3), async {
        while active.load(Ordering::SeqCst) != 0 {
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap();
    let mut stalls = Vec::new();
    for _ in 0..4 {
        let c = client.clone();
        let url = url.clone();
        stalls.push(tokio::spawn(async move {
            c.get(format!("{url}/Videos/test/stall")).send().await
        }));
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while active.load(Ordering::SeqCst) != 4 {
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap();
    assert!(
        client
            .get(format!("{url}/Videos/test/file"))
            .send()
            .await
            .is_err()
    );
    tokio::task::spawn_blocking(move || bluefin_iroh_stop(id))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while active.load(Ordering::SeqCst) != 0 {
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap();
    for job in stalls {
        assert!(job.await.unwrap().is_err());
    }
    let _ = stop.send(());
    adapter.await.unwrap();
    fixture.abort();
    let _ = fixture.await;
}
#[test]
fn validation() {
    for p in [
        "https://evil/",
        "//evil/",
        "/Videos/test/../x",
        "/Videos/test/%2e%2e/x",
        "/Videos/test/a?api_key=x",
        "/Videos/test/a?TOKEN=x",
        "/Videos/test/a#x",
        "/Videos/test/a\\x",
    ] {
        assert!(!path_valid(p), "{p}");
    }
    assert!(Origin::new("http://localhost", "/Videos/test/").is_err());
    assert!(Origin::new("https://localhost", "/").is_err());
    for v in ["bytes=2-1", "bytes=-", "bytes=1-2,3-4", "bytes=a-b"] {
        assert!(!range_valid(v));
    }
    for raw in [
        "POST /x HTTP/1.1\r\nHost: bluefin\r\n\r\n",
        "GET /x HTTP/1.1\r\nHost: evil\r\n\r\n",
        "GET /x HTTP/1.1\r\nHost: bluefin\r\nContent-Length: 2\r\n\r\n",
        "GET /x HTTP/1.1\r\nHost: bluefin\r\nHost: bluefin\r\n\r\n",
    ] {
        assert!(parse(raw.as_bytes(), "bluefin").is_err());
    }
}

#[test]
fn strict_range_decimal_grammar() {
    for r in [
        "bytes=+1-2",
        "bytes=1-+2",
        "bytes=-+2",
        "bytes=9-2",
        "bytes=18446744073709551616-",
        "bytes=-",
        "bytes=1-2,3-4",
    ] {
        assert!(!range_valid(r), "{r}");
    }
    for r in ["bytes=1-2", "bytes=1-", "bytes=-2", "bytes=0-0"] {
        assert!(range_valid(r), "{r}");
    }
}
