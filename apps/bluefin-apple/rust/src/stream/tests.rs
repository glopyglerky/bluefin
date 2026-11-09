use super::*;
use std::{
    io::{Read, Write},
    net::{TcpListener as Listener, TcpStream as Socket},
};
pub(crate) static SERIAL: Mutex<()> = Mutex::new(());
fn fetch(url: &str, method: &str, extra: &str) -> String {
    let u = Url::parse(url).unwrap();
    let mut s = Socket::connect(("127.0.0.1", u.port().unwrap())).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    write!(
        s,
        "{method} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n{extra}\r\n",
        u.path(),
        u.port().unwrap()
    )
    .unwrap();
    let mut b = String::new();
    let _ = s.read_to_string(&mut b);
    b
}
fn state() -> Session {
    Session {
        origin: Url::parse("https://media.test/Videos/i/master.m3u8").unwrap(),
        prefix: "/Videos/i/".into(),
        auth: "secret".into(),
        tunnel: None,
        local: "http://127.0.0.1:1234/cap".into(),
        routes: Mutex::new(Vec::new()),
    }
}
#[test]
fn nested_playlists_and_uri_attributes() {
    let s = state();
    let base = s.origin.clone();
    let p=s.playlist(&base,b"#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,URI=\"audio/index.m3u8\",NAME=\"English\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"keys/key.bin\"\n#EXT-X-MAP:URI=\"init.mp4\"\nvariant/child.m3u8\n").unwrap();
    assert!(!String::from_utf8(p).unwrap().contains("media.test"));
    let child = s.routes.lock().unwrap()[3].clone();
    s.playlist(&child, b"#EXTM3U\n../segment.ts?ordinary=1\n")
        .unwrap();
    assert_eq!(
        s.routes.lock().unwrap()[4].as_str(),
        "https://media.test/Videos/i/segment.ts?ordinary=1"
    );
    for p in [
        "#EXTM3U\nhttps://foreign.test/Videos/i/a.ts\n",
        "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"https://foreign.test/key\"\n",
        "#EXTM3U\n../../System/Info\n",
        "#EXTM3U\n#EXT-X-MAP:URI=bad\n",
        "#EXTM3U\n#EXT-X-MAP:URI=\"x\",URI=\"y\"\n",
        "#EXTM3U\n#EXT-X-PART:URI=\"part.ts\"\n",
    ] {
        assert!(s.playlist(&base, p.as_bytes()).is_err(), "{p}");
    }
    assert!(s
        .route(&base, "https://user:password@media.test/Videos/i/a")
        .is_err());
    s.route(
        &base,
        "seg.ts?api_key=secret&ordinary=1&X-Emby-Token=secret",
    )
    .unwrap();
    assert!(s
        .routes
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .as_str()
        .ends_with("seg.ts?ordinary=1"));
    assert!(s
        .route(&base, "https://user:password@media.test/Videos/i/a.ts")
        .is_err());
}
#[test]
fn range_head_auth_and_stop() {
    let _guard = SERIAL.lock().unwrap();
    for method in ["GET", "HEAD"] {
        let listener = Listener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let upstream = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                let mut b = [0];
                socket.read_exact(&mut b).unwrap();
                bytes.push(b[0]);
            }
            socket.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 4\r\nContent-Range: bytes 2-5/10\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n").unwrap();
            if method == "GET" {
                socket.write_all(b"2345").unwrap();
            }
            String::from_utf8(bytes).unwrap()
        });
        let remote =
            Url::parse(&format!("http://{address}/Videos/i/stream.mp4?segment=1")).unwrap();
        let (id, url) = start(remote, "/Videos/i/".into(), "Bearer secret-token".into()).unwrap();
        let response = fetch(&url, method, "Range: bytes=2-5\r\n");
        assert!(response.starts_with("HTTP/1.1 206"), "{response}");
        assert!(response.contains("content-range: bytes 2-5/10"));
        assert!(response.contains("accept-ranges: bytes"));
        assert!(response.ends_with(if method == "GET" { "2345" } else { "\r\n\r\n" }));
        let request = upstream.join().unwrap();
        assert!(!request.lines().next().unwrap().contains("secret-token"));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer secret-token"));
        stop(&id).unwrap();
        assert!(Socket::connect(("127.0.0.1", Url::parse(&url).unwrap().port().unwrap())).is_err());
    }
}
#[test]
fn reject_requests_cancel_and_release_slots() {
    let _guard = SERIAL.lock().unwrap();
    let remote = Url::parse("http://127.0.0.1:1/Videos/i/master.m3u8").unwrap();
    for _ in 0..3 {
        let (id, url) = start(remote.clone(), "/Videos/i/".into(), "secret".into()).unwrap();
        assert!(fetch(&url, "POST", "").starts_with("HTTP/1.1 400"));
        assert!(fetch(&url.replace(&id, "wrong"), "GET", "").starts_with("HTTP/1.1 400"));
        assert!(fetch(&url, "GET", "Host: evil\r\n").starts_with("HTTP/1.1 400"));
        assert!(
            fetch(&url, "GET", &format!("X-Huge: {}\r\n", "a".repeat(17000)))
                .starts_with("HTTP/1.1 400")
        );
        assert!(fetch(&url, "GET", "").starts_with("HTTP/1.1 400")); // upstream failure
        let u = Url::parse(&url).unwrap();
        let mut stalled = Socket::connect(("127.0.0.1", u.port().unwrap())).unwrap();
        stalled.write_all(b"GET ").unwrap();
        // Configure before cancellation: Darwin rejects SO_RCVTIMEO once the peer closed.
        stalled
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let now = std::time::Instant::now();
        stop(&id).unwrap();
        assert!(now.elapsed() < Duration::from_secs(2));
        let mut b = [0];
        match stalled.read(&mut b) {
            Ok(0) => {}
            Err(error) => assert!(
                matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::NotConnected
                ),
                "Cancellation did not close the stream: {error}"
            ),
            other => panic!("Cancellation left a readable stream: {other:?}"),
        }
    }
    assert!(proxies().lock().unwrap().len() <= 1); // another independent playback test may run
    assert_eq!(STREAMS.get().unwrap().available_permits(), 4);
    assert!(start(remote.clone(), "/unrelated/".into(), "secret".into()).is_err());
}
#[test]
fn two_proxy_limit() {
    let _guard = SERIAL.lock().unwrap();
    let remote = Url::parse("http://127.0.0.1:1/Videos/i/a").unwrap();
    let a = start(remote.clone(), "/Videos/i/".into(), "secret".into()).unwrap();
    let b = start(remote.clone(), "/Videos/i/".into(), "secret".into());
    if let Ok(b) = b {
        assert!(start(remote, "/Videos/i/".into(), "secret".into()).is_err());
        stop(&b.0).unwrap();
    }
    stop(&a.0).unwrap();
}
#[test]
fn stop_cancels_active_upstream_and_all_four_streams() {
    let _guard = SERIAL.lock().unwrap();
    let listener = Listener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (ready, wait) = std::sync::mpsc::channel();
    let upstream = thread::spawn(move || {
        let mut sockets = Vec::new();
        for _ in 0..4 {
            let (mut s, _) = listener.accept().unwrap();
            s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut b = Vec::new();
            while !b.ends_with(b"\r\n\r\n") {
                let mut x = [0];
                s.read_exact(&mut x).unwrap();
                b.push(x[0]);
            }
            sockets.push(s);
        }
        ready.send(()).unwrap();
        for mut s in sockets {
            let mut b = [0];
            assert!(matches!(s.read(&mut b), Ok(0) | Err(_)));
        }
    });
    let remote = Url::parse(&format!("http://{address}/Videos/i/a.ts")).unwrap();
    let (id, url) = start(remote, "/Videos/i/".into(), "Bearer secret".into()).unwrap();
    let u = Url::parse(&url).unwrap();
    let mut sockets = Vec::new();
    for _ in 0..4 {
        let mut s = Socket::connect(("127.0.0.1", u.port().unwrap())).unwrap();
        write!(
            s,
            "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
            u.path(),
            u.port().unwrap()
        )
        .unwrap();
        sockets.push(s);
    }
    wait.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(fetch(&url, "GET", "").starts_with("HTTP/1.1 503"));
    let now = std::time::Instant::now();
    stop(&id).unwrap();
    assert!(now.elapsed() < Duration::from_secs(2));
    upstream.join().unwrap();
    assert_eq!(STREAMS.get().unwrap().available_permits(), 4);
}
#[test]
fn actual_nested_hls_stream() {
    let _guard = SERIAL.lock().unwrap();
    let listener = Listener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let h = thread::spawn(move || {
        for expected in [
            "/Videos/i/master.m3u8",
            "/Videos/i/nested/child.m3u8",
            "/Videos/i/seg.ts?ordinary=1",
        ] {
            let (mut s, _) = listener.accept().unwrap();
            s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut b = [0];
                s.read_exact(&mut b).unwrap();
                request.push(b[0]);
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with(&format!("GET {expected} HTTP/1.1")));
            assert!(request
                .to_ascii_lowercase()
                .contains("authorization: bearer secret"));
            let body = if expected.ends_with("master.m3u8") {
                "#EXTM3U\nnested/child.m3u8\n"
            } else if expected.ends_with("child.m3u8") {
                "#EXTM3U\n../seg.ts?ordinary=1\n"
            } else {
                "media-bytes"
            };
            write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    let (id, url) = start(
        Url::parse(&format!("http://{addr}/Videos/i/master.m3u8")).unwrap(),
        "/Videos/i/".into(),
        "Bearer secret".into(),
    )
    .unwrap();
    let master = fetch(&url, "GET", "");
    let child = master.lines().find(|l| l.starts_with("http://")).unwrap();
    let media = fetch(child, "GET", "");
    let segment = media.lines().find(|l| l.starts_with("http://")).unwrap();
    assert!(fetch(segment, "GET", "").ends_with("media-bytes"));
    stop(&id).unwrap();
    h.join().unwrap();
}

#[test]
fn core_hls_over_real_https_and_pinned_iroh() {
    use super::super::{plan_playback, Api};
    use bluefin_iroh_transport::{endpoint, run_adapter, Origin};
    use serde_json::json;
    let _guard = SERIAL.lock().unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let key =
        tokio_rustls::rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
    let tls = tokio_rustls::rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.cert.der().clone()], key.into())
        .unwrap();
    let accept = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
    let listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let origin = format!(
        "https://localhost:{}",
        listener.local_addr().unwrap().port()
    );
    let fixture_origin = origin.clone();
    let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let active_fixture = active.clone();
    let media_sent = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let sent_fixture = media_sent.clone();
    let fixture = runtime.spawn(async move {
        let mut jobs = JoinSet::new();
        loop { tokio::select! {
            Some(_)=jobs.join_next(),if !jobs.is_empty()=>{},
            v=listener.accept()=>{let (tcp,_)=v.unwrap();let accept=accept.clone();let origin=fixture_origin.clone(); let active=active_fixture.clone();let sent=sent_fixture.clone();jobs.spawn(async move {
                let mut s=accept.accept(tcp).await.unwrap(); let mut raw=Vec::new();
                while !raw.ends_with(b"\r\n\r\n") {raw.push(s.read_u8().await.unwrap());}
                let raw=String::from_utf8(raw).unwrap();
                if !raw.contains("Token=\"viewer\"") {s.write_all(b"HTTP/1.1 401 Denied\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();return;}
                let path=raw.lines().next().unwrap().split(' ').nth(1).unwrap();
                if path.ends_with("stall.ts") {active.fetch_add(1,std::sync::atomic::Ordering::SeqCst);let mut b=[0];let _=s.read(&mut b).await;active.fetch_sub(1,std::sync::atomic::Ordering::SeqCst);return;}
                if path.ends_with("throughput.bin") {
                    let total=64*1024*1024;
                    s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                    if raw.starts_with("HEAD") {let _=s.shutdown().await;return;}
                    let block=[0x5au8;32768];
                    for _ in 0..total/block.len() {if s.write_all(&block).await.is_err(){return;}sent.fetch_add(block.len(),std::sync::atomic::Ordering::SeqCst);}
                    let _=s.shutdown().await;return;
                }
                let body=if path.ends_with("master.m3u8") {"#EXTM3U\nvariant/child.m3u8\n".to_owned()}
                else if path.ends_with("child.m3u8") {format!("#EXTM3U\n../relative.ts\n/Videos/i/root.ts\n{origin}/Videos/i/absolute.ts\n/Videos/i/stall.ts\n")}
                else {"0123456789".into()};
                let range=raw.contains("range: bytes=2-5\r\n");
                let body=if range {"2345".into()} else {body};
                let h=format!("HTTP/1.1 {} OK\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",if range {206}else{200},body.len(),if range {"Content-Range: bytes 2-5/10\r\n"}else{""});
                s.write_all(h.as_bytes()).await.unwrap(); if !raw.starts_with("HEAD") {s.write_all(body.as_bytes()).await.unwrap();}let _=s.shutdown().await;
            });}
        }}
    });
    let adapter = Origin::library_media(&origin, &["/Videos/i/".into(), "/Audio/song/".into()])
        .unwrap()
        .with_root_certificate(reqwest::Certificate::from_der(cert.cert.der()).unwrap())
        .unwrap();
    assert!(Origin::library_media(&origin, &["/Videos/".into()]).is_err());
    let ep = runtime.block_on(endpoint()).unwrap();
    let peer = ep.id().to_string();
    let addr = format!("127.0.0.1:{}", ep.addr().ip_addrs().next().unwrap().port());
    let (shutdown, rx) = oneshot::channel();
    let adapter_job = runtime.spawn(run_adapter(ep, adapter, rx));
    let v = json!({"server":origin,"token":"viewer","itemId":"i","iroh":{"peerId":peer,"peerAddress":addr}});
    let info = json!({"PlaySessionId":"p","MediaSources":[{"Id":"m","TranscodingUrl":"/Videos/i/master.m3u8"}]});
    let api = Api::new(&v).unwrap();
    let p = plan_playback(&api, &info).unwrap();
    let p2 = plan_playback(&api, &info).unwrap();
    assert!(plan_playback(&api, &info).is_err());
    stop(p2["proxyId"].as_str().unwrap()).unwrap();
    let master = fetch(p["url"].as_str().unwrap(), "GET", "");
    let child = master.lines().find(|l| l.starts_with("http://")).unwrap();
    let nested = fetch(child, "GET", "");
    let routes = nested
        .lines()
        .filter(|l| l.starts_with("http://"))
        .collect::<Vec<_>>();
    assert_eq!(routes.len(), 4);
    for route in &routes[..3] {
        assert!(fetch(route, "GET", "").ends_with("0123456789"));
    }
    let ranged = fetch(routes[0], "GET", "Range: bytes=2-5\r\n");
    assert!(ranged.starts_with("HTTP/1.1 206"));
    assert!(ranged.ends_with("2345"));
    let head = fetch(routes[1], "HEAD", "");
    assert!(head.starts_with("HTTP/1.1 200"));
    assert!(!head.contains("0123456789"));
    let stall = Url::parse(routes[3]).unwrap();
    let mut socket = Socket::connect(("127.0.0.1", stall.port().unwrap())).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(
        socket,
        "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
        stall.path(),
        stall.port().unwrap()
    )
    .unwrap();
    for _ in 0..100 {
        if active.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(active.load(std::sync::atomic::Ordering::SeqCst), 1);
    stop(p["proxyId"].as_str().unwrap()).unwrap();
    stop(p["proxyId"].as_str().unwrap()).unwrap();
    let mut b = [0];
    match socket.read(&mut b) {
        Ok(0) => {}
        Err(e)
            if !matches!(
                e.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) => {}
        other => panic!("not cancelled: {other:?}"),
    };
    for _ in 0..100 {
        if active.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(active.load(std::sync::atomic::Ordering::SeqCst), 0);
    for change in [
        json!({"token":"wrong"}),
        json!({"iroh":{"peerId":iroh_wrong_pin(),"peerAddress":addr}}),
    ] {
        let mut bad = v.clone();
        for (key, value) in change.as_object().unwrap() {
            bad[key] = value.clone();
        }
        let badapi = Api::new(&bad).unwrap();
        let p = plan_playback(&badapi, &info).unwrap();
        assert!(!fetch(p["url"].as_str().unwrap(), "GET", "").contains("#EXTM3U"));
        stop(p["proxyId"].as_str().unwrap()).unwrap();
    }
    // Same foreground adapter serves another explicitly admitted item, using viewer auth anew.
    let mut audio = v.clone();
    audio["itemId"] = json!("song");
    audio["mediaType"] = json!("Audio");
    let api = Api::new(&audio).unwrap();
    let info = json!({"PlaySessionId":"p","MediaSources":[{"Id":"m","Container":"flac","SupportsDirectPlay":true,"MediaStreams":[{"Type":"Audio","Codec":"flac"}]}]});
    let p = plan_playback(&api, &info).unwrap();
    assert!(fetch(p["url"].as_str().unwrap(), "GET", "").ends_with("0123456789"));
    stop(p["proxyId"].as_str().unwrap()).unwrap();
    // Metadata failure must dispose the proxy and its tunnel, leaving both slots reusable.
    let mut huge = info.clone();
    huge["padding"] = json!("x".repeat(super::super::RESPONSE_MAX));
    assert!(plan_playback(&api, &huge)
        .unwrap_err()
        .contains("metadata too large"));
    let one = plan_playback(&api, &info).unwrap();
    let two = plan_playback(&api, &info).unwrap();
    stop(one["proxyId"].as_str().unwrap()).unwrap();
    stop(two["proxyId"].as_str().unwrap()).unwrap();
    // Bounded streaming measurement, no whole-file buffering or network shaping.
    let mut media = v.clone();
    media["itemId"] = json!("i");
    let media_api = Api::new(&media).unwrap();
    let media_info = json!({"PlaySessionId":"p","MediaSources":[{"Id":"m","TranscodingUrl":"/Videos/i/throughput.bin"}]});
    let media_plan = plan_playback(&media_api, &media_info).unwrap();
    let http = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    for sample in 0..3 {
        let start = std::time::Instant::now();
        let mut response = http
            .get(media_plan["url"].as_str().unwrap())
            .send()
            .unwrap();
        assert_eq!(response.status(), 200);
        let mut block = [0u8; 32768];
        let mut bytes = 0usize;
        loop {
            let n = response.read(&mut block).unwrap();
            if n == 0 {
                break;
            }
            assert!(block[..n].iter().all(|b| *b == 0x5a));
            bytes += n;
        }
        assert_eq!(bytes, 64 * 1024 * 1024);
        let seconds = start.elapsed().as_secs_f64();
        println!(
            "CORE_MEDIA_SAMPLE {}",
            json!({"sample":sample,"bytes":bytes,"seconds":seconds,"mbps":bytes as f64*8.0/seconds/1e6})
        );
    }
    for sample in 0..5 {
        let now = std::time::Instant::now();
        let response = http
            .head(media_plan["url"].as_str().unwrap())
            .send()
            .unwrap();
        assert_eq!(response.status(), 200);
        println!(
            "CORE_MEDIA_HEAD {}",
            json!({"sample":sample,"transactionMs":now.elapsed().as_secs_f64()*1000.0})
        );
    }
    // A reader that never drains must stop upstream before the whole body is queued.
    media_sent.store(0, std::sync::atomic::Ordering::SeqCst);
    let slow_url = Url::parse(media_plan["url"].as_str().unwrap()).unwrap();
    let mut slow = Socket::connect(("127.0.0.1", slow_url.port().unwrap())).unwrap();
    write!(
        slow,
        "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
        slow_url.path(),
        slow_url.port().unwrap()
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(250));
    let queued = media_sent.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        queued > 0 && queued < 64 * 1024 * 1024,
        "upstream queued {queued} bytes"
    );
    let cancelled = std::time::Instant::now();
    stop(media_plan["proxyId"].as_str().unwrap()).unwrap();
    let cancel_ms = cancelled.elapsed().as_secs_f64() * 1000.0;
    assert!(cancelled.elapsed() < Duration::from_secs(2));
    println!(
        "CORE_MEDIA_BACKPRESSURE {}",
        json!({"upstreamBytesWrittenBeforeStop":queued,"bodyBytes":64*1024*1024,"stopMs":cancel_ms})
    );
    drop(slow);
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
        println!(
            "CORE_MEDIA_RUSAGE_MAXRSS {}",
            unsafe { usage.assume_init() }.ru_maxrss
        );
    }
    let _ = shutdown.send(());
    runtime.block_on(adapter_job).unwrap();
    fixture.abort();
}
fn iroh_wrong_pin() -> String {
    "00".repeat(32)
}
