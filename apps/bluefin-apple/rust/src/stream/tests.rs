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
        routes: Mutex::new(Routes::default()),
    }
}
#[test]
fn sliding_live_playlists_outlast_the_route_capacity() {
    let s = state();
    let base = s.origin.clone();
    let anchor = s.route(&base, base.as_str()).unwrap();
    let rendition = s.route(&base, "audio/index.m3u8").unwrap();
    let key_playlist = String::from_utf8(
        s.playlist(
            &base,
            b"#EXTM3U\n#EXT-X-SESSION-KEY:METHOD=AES-128,URI=\"session.key\"\n",
        )
        .unwrap(),
    )
    .unwrap();
    let session_key = key_playlist.split('"').nth(1).unwrap();
    let old_segment = s.route(&base, "segment-0.ts").unwrap();
    for generation in 0..80 {
        let mut input = format!("#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:{}\n", generation * 64);
        for segment in generation * 64..(generation + 1) * 64 {
            input.push_str(&format!("#EXTINF:2,\nsegment-{segment}.ts\n"));
        }
        let rewritten = s.playlist(&base, input.as_bytes()).unwrap();
        let rewritten = String::from_utf8(rewritten).unwrap();
        assert_eq!(
            rewritten
                .lines()
                .filter(|line| line.starts_with("http"))
                .count(),
            64
        );
        assert!(!rewritten.contains("media.test"));
    }
    let id = |url: &str| url.rsplit('/').next().unwrap().parse::<usize>().unwrap();
    let mut routes = s.routes.lock().unwrap();
    assert_eq!(routes.entries.len(), ROUTES_MAX);
    assert_eq!(routes.urls.len(), ROUTES_MAX);
    assert_eq!(routes.recent.len(), ROUTES_MAX - 1);
    assert!(routes.lookup(id(&anchor)).unwrap().is_some());
    assert!(routes.lookup(id(&rendition)).unwrap().is_some());
    assert!(routes.lookup(id(session_key)).unwrap().is_some());
    assert!(routes.lookup(id(&old_segment)).unwrap().is_none());
    drop(routes);
    let renewed = s.route(&base, "segment-0.ts").unwrap();
    assert_ne!(renewed, old_segment, "Expired IDs must never be reassigned");
    assert!(s
        .routes
        .lock()
        .unwrap()
        .lookup(id(&old_segment))
        .unwrap()
        .is_none());
}

#[test]
fn rolling_live_hls_over_http_keeps_playing_and_rejects_expired_urls() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let _serial = SERIAL.lock().unwrap();
    let listener = Listener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(AtomicUsize::new(0));
    let stopping = done.clone();
    let observed = requests.clone();
    let origin = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut generation = 0;
        while !stopping.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
            let (mut socket, _) = match listener.accept() {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(error) => panic!("Origin accept failed: {error}"),
            };
            // Darwin may inherit the listener's nonblocking mode on accepted sockets.
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut input = Vec::new();
            while !input.ends_with(b"\r\n\r\n") {
                assert!(input.len() < 16384);
                let mut byte = [0];
                socket.read_exact(&mut byte).unwrap();
                input.push(byte[0]);
            }
            let input = String::from_utf8(input).unwrap();
            assert!(input.lines().any(|line| line.eq_ignore_ascii_case(
                "authorization: MediaBrowser Client=\"Bluefin Apple\", Token=\"fixture-only\""
            )));
            let path = input.split_whitespace().nth(1).unwrap();
            let (kind, body) = if path == "/Videos/i/master.m3u8" {
                let mut body = format!(
                    "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{}\n",
                    generation * 64
                );
                for segment in generation * 64..(generation + 1) * 64 {
                    body.push_str(&format!("#EXTINF:2,\nsegment-{segment}.ts\n"));
                }
                generation += 1;
                ("application/vnd.apple.mpegurl", body)
            } else {
                assert!(path.starts_with("/Videos/i/segment-"));
                ("video/mp2t", format!("original-fixture:{path}"))
            };
            observed.fetch_add(1, Ordering::Relaxed);
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    struct OriginGuard(Arc<AtomicBool>, Option<thread::JoinHandle<()>>);
    impl Drop for OriginGuard {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
            if let Some(join) = self.1.take() {
                let _ = join.join();
            }
        }
    }
    let mut origin = OriginGuard(done, Some(origin));
    let remote = Url::parse(&format!("http://{address}/Videos/i/master.m3u8")).unwrap();
    let (proxy, local) = start(
        remote,
        "/Videos/i/".into(),
        "MediaBrowser Client=\"Bluefin Apple\", Token=\"fixture-only\"".into(),
    )
    .unwrap();
    struct ProxyGuard(String);
    impl Drop for ProxyGuard {
        fn drop(&mut self) {
            let _ = stop(&self.0);
        }
    }
    let proxy = ProxyGuard(proxy);
    let mut expired = None;
    for generation in 0..80 {
        let playlist = fetch(&local, "GET", "");
        assert!(
            playlist.starts_with("HTTP/1.1 200"),
            "Generation {generation}: {playlist}"
        );
        assert!(!playlist.contains("fixture-only"));
        assert!(!playlist.contains(&address.to_string()));
        let body = playlist.split_once("\r\n\r\n").unwrap().1;
        let latest = body
            .lines()
            .rev()
            .find(|line| line.starts_with("http"))
            .unwrap();
        let segment = fetch(latest, "GET", "");
        assert!(segment.starts_with("HTTP/1.1 200"));
        assert!(segment.ends_with(&format!("segment-{}.ts", (generation + 1) * 64 - 1)));
        if expired.is_none() {
            expired = Some(latest.to_owned());
        }
    }
    assert!(fetch(expired.as_ref().unwrap(), "GET", "").starts_with("HTTP/1.1 400"));
    assert_eq!(requests.load(Ordering::Relaxed), 160);
    drop(proxy);
    origin.0.store(true, Ordering::Release);
    origin.1.take().unwrap().join().unwrap();
}

#[test]
fn oversized_manifest_never_publishes_expired_capabilities() {
    let s = state();
    let mut input = String::from("#EXTM3U\n");
    for segment in 0..ROUTES_MAX + 10 {
        input.push_str(&format!("segment-{segment}.ts\n"));
    }
    assert!(s.playlist(&s.origin, input.as_bytes()).is_err());
}

#[test]
fn active_progressive_media_outlasts_thirty_seconds() {
    let _serial = SERIAL.lock().unwrap();
    let listener = Listener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let origin = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut input = Vec::new();
        while !input.ends_with(b"\r\n\r\n") {
            assert!(input.len() < 16384);
            let mut byte = [0];
            socket.read_exact(&mut byte).unwrap();
            input.push(byte[0]);
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: 17\r\nConnection: close\r\n\r\n").unwrap();
        for i in 0..17 {
            if i != 0 {
                thread::sleep(Duration::from_secs(2));
            }
            if socket.write_all(b"x").is_err() {
                break;
            }
        }
    });
    let (proxy, local) = start(
        Url::parse(&format!("http://{address}/Videos/i/original.mp4")).unwrap(),
        "/Videos/i/".into(),
        "MediaBrowser Token=\"original-fixture\"".into(),
    )
    .unwrap();
    let response = fetch(&local, "GET", "");
    stop(&proxy).unwrap();
    origin.join().unwrap();
    assert!(response.starts_with("HTTP/1.1 200"));
    assert_eq!(
        response.split_once("\r\n\r\n").unwrap().1,
        "x".repeat(17),
        "An active response must not end at the metadata request timeout"
    );
}
#[test]
fn nested_playlists_and_uri_attributes() {
    let s = state();
    let base = s.origin.clone();
    let p=s.playlist(&base,b"#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,URI=\"audio/index.m3u8\",NAME=\"English\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"keys/key.bin\"\n#EXT-X-MAP:URI=\"init.mp4\"\nvariant/child.m3u8\n").unwrap();
    assert!(!String::from_utf8(p).unwrap().contains("media.test"));
    let child = s.routes.lock().unwrap().lookup(3).unwrap().unwrap();
    s.playlist(&child, b"#EXTM3U\n../segment.ts?ordinary=1\n")
        .unwrap();
    assert_eq!(
        s.routes
            .lock()
            .unwrap()
            .lookup(4)
            .unwrap()
            .unwrap()
            .as_str(),
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
    let cleaned = s
        .route(
            &base,
            "seg.ts?api_key=secret&ordinary=1&X-Emby-Token=secret",
        )
        .unwrap();
    let cleaned_id = cleaned.rsplit('/').next().unwrap().parse().unwrap();
    assert!(s
        .routes
        .lock()
        .unwrap()
        .lookup(cleaned_id)
        .unwrap()
        .unwrap()
        .as_str()
        .ends_with("seg.ts?ordinary=1"));
    assert!(s
        .route(&base, "https://user:password@media.test/Videos/i/a.ts")
        .is_err());
}
#[test]
fn vod_master_rewrites_trickplay_renditions() {
    let session = state();
    let master = &session.origin;
    let rewritten = session
        .playlist(
            master,
            b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1200000\nmain.m3u8\n#EXT-X-IMAGE-STREAM-INF:BANDWIDTH=18000,RESOLUTION=320x180,CODECS=\"jpeg\",URI=\"Trickplay/320/tiles.m3u8?MediaSourceId=source&ApiKey=secret\"\n",
        )
        .unwrap();
    let playlist = String::from_utf8(rewritten).unwrap();
    assert!(!playlist.contains("secret"));
    assert!(playlist.contains(&format!("URI=\"{}/1\"", session.local)));
    let image_playlist = session.routes.lock().unwrap().lookup(1).unwrap().unwrap();
    assert!(image_playlist
        .as_str()
        .ends_with("/Trickplay/320/tiles.m3u8?MediaSourceId=source"));
    assert!(session
        .playlist(
            &image_playlist,
            b"#EXTM3U\n#EXT-X-IMAGES-ONLY\n#EXT-X-TILES:RESOLUTION=320x180,LAYOUT=4x4,DURATION=5.0\n0.jpg\n",
        )
        .is_ok());

    for bad in [
        "#EXTM3U\n#EXT-X-IMAGE-STREAM-INF:BANDWIDTH=18000\n",
        "#EXTM3U\n#EXT-X-IMAGE-STREAM-INF:URI=\"https://foreign.test/tiles.m3u8\"\n",
        "#EXTM3U\n#EXT-X-IMAGE-STREAM-INF:URI=\"/Videos/different/tiles.m3u8\"\n",
    ] {
        assert!(session.playlist(master, bad.as_bytes()).is_err(), "{bad}");
    }
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
            match s.read(&mut b) {
                Ok(0) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::ConnectionAborted
                            | std::io::ErrorKind::NotConnected
                    ) => {}
                other => panic!("upstream socket did not close after stop: {other:?}"),
            }
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
    use bluefin_iroh_transport::{run_adapter, Origin};
    use serde_json::json;
    use sha2::{Digest, Sha256};
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
    let ep = runtime
        .block_on(async {
            iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
                .transport_config(
                    iroh::endpoint::QuicTransportConfig::builder()
                        .max_concurrent_bidi_streams(1u32.into())
                        .max_concurrent_uni_streams(0u32.into())
                        .stream_receive_window((8u32 * 1024 * 1024).into())
                        .receive_window((16u32 * 1024 * 1024).into())
                        .send_window(16 * 1024 * 1024)
                        .build(),
                )
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .relay_mode(iroh::RelayMode::Disabled)
                .alpns(vec![bluefin_iroh_transport::ALPN.to_vec()])
                .clear_ip_transports()
                .bind_addr("127.0.0.1:0")
                .unwrap()
                .bind()
                .await
        })
        .unwrap();
    assert_eq!(ep.bound_sockets().len(), 1);
    assert!(ep.bound_sockets()[0].ip().is_loopback());
    let peer = ep.id().to_string();
    let addr = format!("127.0.0.1:{}", ep.bound_sockets()[0].port());
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
        let mut digest = Sha256::new();
        let mut next_stall = 4 * 1024 * 1024;
        loop {
            let n = response.read(&mut block).unwrap();
            if n == 0 {
                break;
            }
            assert!(block[..n].iter().all(|b| *b == 0x5a));
            digest.update(&block[..n]);
            bytes += n;
            if bytes >= next_stall {
                std::thread::sleep(Duration::from_millis(20));
                next_stall += 4 * 1024 * 1024;
            }
        }
        assert_eq!(bytes, 64 * 1024 * 1024);
        let digest = format!("{:x}", digest.finalize());
        assert_eq!(
            digest,
            "103f23a15401a701b73587902f16e3b5b3bf38a039d5c94b675a9a8e84dbd5b5"
        );
        let seconds = start.elapsed().as_secs_f64();
        println!(
            "CORE_MEDIA_SAMPLE {}",
            json!({"sample":sample,"bytes":bytes,"sha256":digest,"seconds":seconds,"mbps":bytes as f64*8.0/seconds/1e6})
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

#[test]
fn tunnel_drain_failure_reaches_core_stop_receipt() {
    let _serial = SERIAL.lock().unwrap();
    let (id, _) = start_with_transport(
        Url::parse("https://media.test/Videos/i/master.m3u8").unwrap(),
        "/Videos/i/".into(),
        "secret".into(),
        Some(super::super::iroh_transport::Tunnel::failed_drain()),
    )
    .unwrap();
    let request = std::ffi::CString::new(
        serde_json::json!({"operation":"stopPlaybackProxy", "proxyId":id}).to_string(),
    )
    .unwrap();
    // Exercise the actual C entry point and its safe cleanup error envelope.
    let receipt = unsafe {
        let pointer = super::super::bluefin_call(request.as_ptr());
        let receipt: serde_json::Value =
            serde_json::from_slice(std::ffi::CStr::from_ptr(pointer).to_bytes()).unwrap();
        super::super::bluefin_free(pointer);
        receipt
    };
    assert_eq!(receipt["ok"], false);
    assert_eq!(receipt["error"], "Iroh tunnel cleanup unconfirmed");
    stop(&id).unwrap();
}

#[test]
fn session_drain_requires_request_owners_to_exit() {
    let mut session = Arc::new(state());
    let request = session.clone();
    assert_eq!(
        drain_session(&mut session).unwrap_err(),
        "Proxy cleanup unconfirmed"
    );
    drop(request);
    drain_session(&mut session).unwrap();
    drain_session(&mut session).unwrap();
}

#[test]
fn proxy_admission_stays_locked_until_thread_exit_and_error_receipt() {
    let _serial = SERIAL.lock().unwrap();
    let (stop_tx, stop_rx) = oneshot::channel();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (exit_tx, exit_rx) = std::sync::mpsc::channel();
    let join = thread::spawn(move || {
        stop_rx.blocking_recv().unwrap();
        entered_tx.send(()).unwrap();
        exit_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        Err(err("Iroh tunnel cleanup unconfirmed"))
    });
    proxies().lock().unwrap().insert(
        "drain-lifetime-fixture".into(),
        Proxy {
            stop: stop_tx,
            join,
        },
    );
    let stopping = thread::spawn(|| stop("drain-lifetime-fixture"));
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(
        proxies().try_lock(),
        Err(std::sync::TryLockError::WouldBlock)
    ));
    assert!(!stopping.is_finished());
    exit_tx.send(()).unwrap();
    assert_eq!(
        stopping.join().unwrap().unwrap_err(),
        "Iroh tunnel cleanup unconfirmed"
    );
    stop("drain-lifetime-fixture").unwrap();
}
