use bluefin_universal::{
    bridge::Bridge,
    playback::{Plan, Selection},
    protocol::{Item, Session},
};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};
fn bridge(listener: &TcpListener) -> Bridge {
    let base = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let id = uuid::Uuid::from_u128(1);
    let prefix = format!("/Videos/{}/", id.simple());
    let plan = Plan {
        item: Item {
            id,
            name: "Original byte fixture".into(),
            kind: "Movie".into(),
            overview: String::new(),
            start_date: None,
            end_date: None,
            channel_name: None,
            channel_id: None,
        },
        url: base.join(&format!("{prefix}stream")).unwrap(),
        prefix,
        play_session: "synthetic".into(),
        media_source: "original".into(),
        live_stream: None,
        method: "DirectPlay".into(),
        hls: false,
        live: false,
        offset: 0.0,
        selection: Selection::default(),
        tracks: vec![],
    };
    Bridge::start(
        Session {
            base,
            token: "synthetic-test-only".into(),
            user: id,
        },
        plan,
        None,
    )
    .unwrap()
}
fn accept(listener: TcpListener) -> std::net::TcpStream {
    let (mut stream, _) = listener.accept().unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    let mut auth = false;
    loop {
        line.clear();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        let lower = line.to_lowercase();
        assert!(!lower.starts_with("x-emby"));
        assert!(!lower.starts_with("cookie:"));
        auth |= line.starts_with("authorization: MediaBrowser ")
            || line.starts_with("Authorization: MediaBrowser ");
    }
    assert!(auth);
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 36\r\nConnection: close\r\n\r\n").unwrap();
    stream
}
#[test]
#[ignore = "32-second regression for the former 30-second total media deadline"]
fn active_media_outlives_thirty_second_total_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let bridge = bridge(&listener);
    let task = std::thread::spawn(move || {
        let mut stream = accept(listener);
        for _ in 0..36 {
            stream.write_all(b"x").unwrap();
            std::thread::sleep(Duration::from_millis(900));
        }
    });
    let start = Instant::now();
    let bytes = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(45))
        .build()
        .unwrap()
        .get(&bridge.url)
        .send()
        .unwrap()
        .bytes()
        .unwrap();
    assert_eq!(bytes.len(), 36);
    assert!(start.elapsed() > Duration::from_secs(30));
    bridge.stop();
    task.join().unwrap();
}
#[test]
#[ignore = "10-second idle timeout and prompt cancellation runtime regression"]
fn idle_read_and_explicit_stop_remain_bounded() {
    for stop in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let bridge = bridge(&listener);
        let upstream = std::thread::spawn(move || {
            let mut stream = accept(listener);
            let _ = stream.write_all(b"x");
            std::thread::sleep(Duration::from_secs(12));
        });
        let mut response = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap()
            .get(&bridge.url)
            .send()
            .unwrap();
        let mut first = [0u8; 1];
        response.read_exact(&mut first).unwrap();
        let start = Instant::now();
        if stop {
            bridge.stop();
        }
        let mut rest = Vec::new();
        assert!(response.read_to_end(&mut rest).is_err());
        if stop {
            assert!(start.elapsed() < Duration::from_secs(2));
        } else {
            assert!(start.elapsed() < Duration::from_secs(12));
            assert!(start.elapsed() > Duration::from_secs(8));
            bridge.stop();
        }
        upstream.join().unwrap();
    }
}
