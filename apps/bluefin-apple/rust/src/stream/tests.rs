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
        let now = std::time::Instant::now();
        stop(&id).unwrap();
        assert!(now.elapsed() < Duration::from_secs(2));
        stalled
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut b = [0];
        assert!(matches!(stalled.read(&mut b), Ok(0) | Err(_)));
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
