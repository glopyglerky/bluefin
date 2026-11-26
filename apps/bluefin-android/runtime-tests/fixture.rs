//! Original bounded loopback HTTP instrumentation fixture. No production accounts.
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
fn serve(
    mut s: TcpStream,
    audio: Arc<Vec<u8>>,
    active: Arc<AtomicUsize>,
    sessions: Arc<Mutex<Vec<String>>>,
) {
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut b = Vec::new();
    let mut buf = [0; 4096];
    while b.len() < 65536 {
        match s.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => b.extend_from_slice(&buf[..n]),
        }
        if let Some(end) = b.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&b[..end]);
            let length = headers
                .lines()
                .find_map(|l| {
                    l.split_once(':')
                        .filter(|(k, _)| k.eq_ignore_ascii_case("Content-Length"))
                        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if length > 65536 {
                return;
            }
            if b.len() >= end + 4 + length {
                break;
            }
        }
    }
    let req = String::from_utf8_lossy(&b);
    let line = req.lines().next().unwrap_or("");
    let path = line
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("");
    let login = path == "/Users/AuthenticateByName";
    if login
        && (!req.contains("original-fixture-user") || !req.contains("original-fixture-password"))
    {
        respond(&mut s, 403, b"{}", "application/json");
        return;
    }
    let auth = req
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("authorization:"))
        .unwrap_or("");
    let token = sessions
        .lock()
        .unwrap()
        .iter()
        .find(|t| auth.contains(&format!("Token=\"{t}\"")))
        .cloned();
    let valid = auth.contains("MediaBrowser Client=\"Bluefin Android\"")
        && ["Device=", "DeviceId=", "Version="]
            .iter()
            .all(|v| auth.contains(v))
        && (login || token.is_some());
    let forbidden = req.to_ascii_lowercase().contains("x-emby-token:")
        || req.to_ascii_lowercase().contains("x-emby-authorization:")
        || line.to_ascii_lowercase().contains("token=")
        || line.to_ascii_lowercase().contains("api_key=");
    if !valid || forbidden {
        println!("REJECT {}", path);
        respond(&mut s, 403, b"{}", "application/json");
        return;
    }
    println!("AUTH_OK {}", path);
    if path == "/Sessions/Logout" {
        sessions
            .lock()
            .unwrap()
            .retain(|t| Some(t) != token.as_ref());
    }
    if path.contains("delayed") && path.ends_with("PlaybackInfo") {
        active.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_secs(7));
        active.fetch_sub(1, Ordering::SeqCst);
    }
    let body = if login {
        let token = format!(
            "original-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let mut all = sessions.lock().unwrap();
        if all.len() >= 32 {
            respond(&mut s, 503, b"{}", "application/json");
            return;
        }
        all.push(token.clone());
        format!(r#"{{"User":{{"Id":"fixture-user"}},"AccessToken":"{token}"}}"#)
    } else if path == "/Users/fixture-user" || path == "/Users/Me" {
        r#"{"Id":"fixture-user","Policy":{"EnableContentDownloading":true}}"#.to_owned()
    } else if path.ends_with("PlaybackInfo") && path.contains("video") {
        r#"{"PlaySessionId":"fixture-session","MediaSources":[{"Id":"fixture-source","Container":"mp4","SupportsDirectPlay":true,"Protocol":"File","MediaStreams":[{"Type":"Video","Codec":"h264","Profile":"High","Width":320,"Height":180,"Level":30,"BitDepth":8,"VideoRangeType":"SDR"},{"Type":"Audio","Codec":"aac","Channels":2}]}]}"#.to_owned()
    } else if path.ends_with("PlaybackInfo") {
        let source = if req.contains("\"IsPlayback\":false") {
            if path.contains("/slow/") {
                "slow"
            } else {
                "song"
            }
        } else {
            "fixture-source"
        };
        format!(
            r#"{{"PlaySessionId":"fixture-session","MediaSources":[{{"Id":"{source}","Container":"wav","Size":{},"SupportsDirectPlay":true,"Protocol":"File","MediaStreams":[{{"Type":"Audio","Codec":"pcm_s16le","Channels":1,"SampleRate":44100}}]}}]}}"#,
            audio.len()
        )
    } else if path.starts_with("/Videos/") && path.ends_with("stream.mp4") {
        let video = std::fs::read(std::env::args().nth(2).expect("mp4 path")).unwrap();
        respond(&mut s, 200, &video, "video/mp4");
        return;
    } else if path.starts_with("/Audio/") || path.ends_with("/Download") {
        if path.contains("/slow/") {
            std::thread::sleep(Duration::from_secs(7));
        }
        respond(&mut s, 200, &audio, "audio/wav");
        return;
    } else if path.contains("/Items/") {
        r#"{"Id":"song","Type":"Audio","MediaType":"Audio"}"#.to_owned()
    } else {
        "{}".to_owned()
    };
    respond(&mut s, 200, body.as_bytes(), "application/json");
}
fn respond(s: &mut TcpStream, status: u16, b: &[u8], ty: &str) {
    let _ = write!(
        s,
        "HTTP/1.1 {} OK\r\nContent-Length: {}\r\nContent-Type: {}\r\nConnection: close\r\n\r\n",
        status,
        b.len(),
        ty
    );
    let _ = s.write_all(b);
}
fn main() {
    let path = std::env::args().nth(1).expect("original wav path");
    let audio = Arc::new(std::fs::read(path).unwrap());
    let l = TcpListener::bind("127.0.0.1:18765").unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let sessions = Arc::new(Mutex::new(Vec::new()));
    println!("READY 127.0.0.1:18765");
    for (n, s) in l.incoming().enumerate() {
        if n >= 400 {
            break;
        }
        let a = audio.clone();
        let c = active.clone();
        let sessions = sessions.clone();
        if let Ok(s) = s {
            std::thread::spawn(move || serve(s, a, c, sessions));
        }
    }
}
