//! Test-only loopback fixture. Original PCM tones and manually authored words.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
const USER: &str = "synthetic-ipad";
const PW: &str = "original-tone-qa-1109";
const TOKEN: &str = "synthetic-ipad-session-1109";
const ID: &str = "qa-original-tone";
struct State {
    prefs: Value,
    version: u64,
    count: u64,
    logout: bool,
}
struct Media {
    a: Vec<u8>,
    v: Vec<u8>,
    mix: Vec<u8>,
    manifest: Value,
    date: String,
}
fn wav(role: u8) -> Vec<u8> {
    let len = 180 * 44100 * 2;
    let mut b = Vec::with_capacity(len + 44);
    b.extend(b"RIFF");
    b.extend(((len + 36) as u32).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(44100u32.to_le_bytes());
    b.extend(88200u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend((len as u32).to_le_bytes());
    for i in 0..180 * 44100 {
        let t = i as f64 / 44100.;
        let a = (t * std::f64::consts::TAU * 220.).sin() * 0.08;
        let v = (t * std::f64::consts::TAU * 440.).sin() * 0.05;
        let n = match role {
            0 => a,
            1 => v,
            _ => a + v,
        };
        b.extend(((n * 32767.) as i16).to_le_bytes());
    }
    b
}
fn item() -> Value {
    json!({"Id":ID,"Name":"Lanterns on the test shore","Type":"Audio","MediaType":"Audio","Album":"Original QA sketches","Artists":["Synthetic duet"],"AlbumArtist":"Synthetic duet","RunTimeTicks":1800000000,"UserData":{"IsFavorite":false,"PlaybackPositionTicks":0},"Overview":"Original synthetic tones. Manually timed words, no singing-quality claim."})
}
fn send(s: &mut TcpStream, status: u16, b: &[u8], mime: &str, head: bool, extra: &str) {
    let _ = write!(
        s,
        "HTTP/1.1 {status} QA\r\nContent-Length: {}\r\nContent-Type: {mime}\r\nConnection: close\r\n{extra}\r\n",
        b.len()
    );
    if !head {
        let _ = s.write_all(b);
    }
}
fn serve(mut s: TcpStream, state: &Mutex<State>, m: &Media) -> std::io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    s.set_write_timeout(Some(Duration::from_secs(10)))?;
    let request_deadline = Instant::now() + Duration::from_secs(15);
    let mut raw = Vec::new();
    let mut buf = [0; 4096];
    let (end, len) = loop {
        if Instant::now() >= request_deadline {
            return Ok(());
        }
        let n = s.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        raw.extend_from_slice(&buf[..n]);
        if raw.len() > 131072 {
            return Ok(());
        }
        if let Some(e) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            let h = String::from_utf8_lossy(&raw[..e]);
            let len = h
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|x| x.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if len > 60000 {
                return Ok(());
            }
            if raw.len() >= e + 4 + len {
                break (e, len);
            }
        }
    };
    let h = String::from_utf8_lossy(&raw[..end]);
    let mut lines = h.lines();
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let method = first.next().unwrap_or("");
    let target = first.next().unwrap_or("");
    let path = target.split('?').next().unwrap_or("");
    let headers: HashMap<_, _> = lines
        .filter_map(|l| {
            l.split_once(':')
                .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect();
    let auth = headers
        .get("authorization")
        .map(String::as_str)
        .unwrap_or("");
    let identity = auth.starts_with("MediaBrowser ")
        && [
            "Client=\"Bluefin Apple\"",
            "Device=\"",
            "DeviceId=\"",
            "Version=\"",
        ]
        .iter()
        .all(|x| auth.contains(x));
    let authenticated = identity
        && auth.contains(&format!("Token=\"{TOKEN}\""))
        && !headers.contains_key("x-emby-token")
        && !target.to_lowercase().contains("token");
    let body: Value = serde_json::from_slice(&raw[end + 4..end + 4 + len]).unwrap_or(Value::Null);
    let mut st = state.lock().unwrap();
    st.count += 1;
    println!(
        "{} {} {} identity={} authenticated={} version={} bytes={} range={:?}",
        st.count,
        method,
        target,
        identity,
        authenticated,
        st.version,
        len,
        headers.get("range")
    );
    let mut status = 200;
    let mut extra = String::new();
    let mut binary: Option<&[u8]> = None;
    let value = if !identity
        || (!["/Users/AuthenticateByName", "/System/Info/Public"].contains(&path)
            && (!authenticated || st.logout))
    {
        status = 401;
        json!({"error":"unauthorized"})
    } else if path == "/Users/AuthenticateByName" && method == "POST" {
        if body["Username"] == USER && body["Pw"] == PW {
            st.logout = false;
            json!({"AccessToken":TOKEN,"User":{"Id":USER,"Name":"Synthetic iPad QA"}})
        } else {
            status = 401;
            json!({"error":"unknown synthetic user"})
        }
    } else if path == "/System/Info/Public" {
        json!({"Id":"synthetic-loopback","ServerName":"Original QA fixture","Version":"0.0.0"})
    } else if path == "/Users/Me" {
        json!({"Id":USER,"Policy":{"EnableContentDownloading":true}})
    } else if path == "/Sessions/Logout" {
        st.logout = true;
        status = 204;
        Value::Null
    } else if path.starts_with("/Sessions/Playing") && body["ItemId"] == ID {
        status = 204;
        Value::Null
    } else if path == "/sports/api/v1/preferences" {
        if method == "PUT" {
            if headers.get("if-match") != Some(&format!("\"{}\"", st.version)) {
                status = 412
            } else if body["schema"] != 1 || !body["teams"].is_array() {
                status = 400
            } else {
                st.prefs = body.clone();
                st.version += 1;
                println!("CAS saved {} teams={}", st.version, st.prefs["teams"]);
            }
        }
        extra = format!("ETag: \"{}\"\r\n", st.version);
        json!({"version":st.version,"preferences":st.prefs})
    } else if path == "/sports/api/v1/home" {
        let games:Vec<Value>=[("LIVE","IN","Third quarter","72","68"),("UPCOMING","PRE","Later today","0","0"),("FINAL","POST","Final","91","88"),("UNKNOWN","UNKNOWN","Unconfirmed schedule","","")].iter().enumerate().map(|(i,(kind,state,detail,a,b))|json!({"kind":kind,"item_id":format!("qa-game-{i}"),"playback_verified":false,"provenance":"Original fictional QA schedule","event":{"name":"Harbor at Summit","league_name":"Fictional QA League","date":m.date,"state":state,"detail":detail,"competitors":[{"id":"harbor","name":"Harbor Herons","short_name":"Harbor","abbreviation":"HBR","side":"away","score":a},{"id":"summit","name":"Summit Owls","short_name":"Summit","abbreviation":"SMT","side":"home","score":b}],"broadcasters":["Synthetic source, no viewing rights"]}})).collect();
        json!({"schema":1,"sports_only":true,"presentation_only":true,"games":games,"local_broadcasts":[{"kind":"ON_NOW","item_id":"qa-unknown-broadcast","title":"Unidentified synthetic broadcast","playback_verified":false}],"providers":[]})
    } else if path == format!("/Bluefin/Karaoke/{ID}") {
        m.manifest.clone()
    } else if path.starts_with(&format!(
        "/Bluefin/Karaoke/{ID}/{}/Stems/",
        m.manifest["revision"].as_str().unwrap()
    )) {
        binary = match path.rsplit('/').next() {
            Some("accompaniment") => Some(&m.a),
            Some("lead-vocal") => Some(&m.v),
            _ => None,
        };
        if binary.is_none() {
            status = 404
        }
        Value::Null
    } else if path == format!("/Items/{ID}/PlaybackInfo") {
        json!({"PlaySessionId":"qa-original-session","MediaSources":[{"Id":ID,"Container":"wav","Protocol":"File","SupportsDirectPlay":true,"Size":m.mix.len(),"Bitrate":705600,"MediaStreams":[{"Type":"Audio","Index":0,"Codec":"pcm_s16le","SampleRate":44100,"BitDepth":16,"Channels":1}]}]})
    } else if path == format!("/Audio/{ID}/stream.wav") {
        binary = Some(&m.mix);
        Value::Null
    } else if path == format!("/Audio/{ID}/Lyrics") {
        json!({"Metadata":{},"Lyrics":[{"Text":"Original manually timed QA words","Start":0}]})
    } else if path == format!("/Users/{USER}/Items/{ID}") {
        item()
    } else if path == format!("/Users/{USER}/Views") {
        json!({"Items":[{"Id":"qa-music","Name":"Original QA music","Type":"CollectionFolder","CollectionType":"music","IsFolder":true}],"TotalRecordCount":1})
    } else if path == format!("/Users/{USER}/Items/Latest") {
        json!([item()])
    } else if path == format!("/Users/{USER}/Items/Resume") || path == "/Shows/NextUp" {
        json!({"Items":[],"TotalRecordCount":0})
    } else if path == format!("/Users/{USER}/Items") {
        if ["MusicAlbum", "MusicArtist", "Playlist"]
            .iter()
            .any(|x| target.contains(x))
        {
            json!({"Items":[],"TotalRecordCount":0})
        } else {
            json!({"Items":[item()],"TotalRecordCount":1})
        }
    } else {
        status = 404;
        json!({"error":"unowned or unsupported route"})
    };
    drop(st);
    if let Some(bytes) = binary {
        let mut lo = 0;
        let mut hi = bytes.len() - 1;
        extra = "Accept-Ranges: bytes\r\n".into();
        if let Some(range) = headers.get("range") {
            if let Some((a, b)) = range.strip_prefix("bytes=").and_then(|x| x.split_once('-')) {
                if a.is_empty() {
                    if let Ok(n) = b.parse::<usize>() {
                        lo = bytes.len().saturating_sub(n)
                    } else {
                        status = 416
                    }
                } else if let Ok(n) = a.parse() {
                    lo = n;
                    if !b.is_empty() {
                        match b.parse::<usize>() {
                            Ok(n) => hi = n.min(hi),
                            Err(_) => status = 416,
                        }
                    }
                } else {
                    status = 416
                }
            } else {
                status = 416
            }
            if lo > hi || lo >= bytes.len() {
                status = 416
            }
            if status != 416 {
                status = 206;
                extra += &format!("Content-Range: bytes {lo}-{hi}/{}\r\n", bytes.len())
            }
        }
        if status == 416 {
            send(&mut s, 416, b"", "audio/wav", false, "")
        } else {
            send(
                &mut s,
                status,
                &bytes[lo..=hi],
                "audio/wav",
                method == "HEAD",
                &extra,
            )
        }
    } else {
        let bytes = if status == 204 {
            Vec::new()
        } else {
            value.to_string().into_bytes()
        };
        send(
            &mut s,
            status,
            &bytes,
            "application/json",
            method == "HEAD",
            &extra,
        )
    }
    Ok(())
}
fn main() {
    let mut args = std::env::args().skip(1);
    let port: u16 = args.next().expect("port").parse().unwrap();
    let date = args.next().expect("current ISO UTC date");
    let stop = args.next().expect("owned stop sentinel path");
    let a = wav(0);
    let v = wav(1);
    let mix = wav(2);
    let hash = |b: &[u8]| format!("{:x}", Sha256::digest(b));
    let revision = hash(&[a.as_slice(), v.as_slice()].concat());
    let lines:Vec<Value>=(0..30).map(|i|{let t=i as f64*6.;json!({"id":format!("original-line-{i}"),"voice":if i%3==0{"lead"}else if i%3==1{"partner"}else{"together"},"tokens":[{"text":"Lanterns ","start":t,"end":t+2.},{"text":"cross ","start":t+2.,"end":t+4.},{"text":"the test shore","start":t+4.,"end":t+6.}]})}).collect();
    let manifest = json!({"itemId":ID,"revision":revision,"durationSeconds":180,"processing":"suppliedStems","accompaniment":{"sha256":hash(&a),"bytes":a.len()},"leadVocal":{"sha256":hash(&v),"bytes":v.len()},"timeline":{"offsetSeconds":0,"lines":lines}});
    println!("MANIFEST {manifest}");
    let media = Arc::new(Media {
        a,
        v,
        mix,
        manifest,
        date,
    });
    let state = Arc::new(Mutex::new(State {
        prefs: json!({"schema":1,"sports":["basketball"],"leagues":[],"conferences":[],"teams":[],"players":[],"all_conferences":true,"all_teams":true,"all_players":true,"home":true,"away":true,"native_home":true,"auto_record":false,"sports_only":true,"completed":true}),
        version: 1,
        count: 0,
        logout: false,
    }));
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    listener.set_nonblocking(true).unwrap();
    println!("READY 127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(3600);
    let mut workers: Vec<std::thread::JoinHandle<()>> = Vec::new();
    while Instant::now() < deadline && !std::path::Path::new(&stop).exists() {
        let mut i = 0;
        while i < workers.len() {
            if workers[i].is_finished() {
                let h = workers.swap_remove(i);
                let _ = h.join();
            } else {
                i += 1;
            }
        }
        match listener.accept() {
            Ok((s, _)) if workers.len() < 8 => {
                let st = state.clone();
                let m = media.clone();
                workers.push(std::thread::spawn(move || {
                    if let Err(e) = serve(s, &st, &m) {
                        eprintln!("request closed: {e}")
                    }
                }));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(e) => panic!("{e}"),
        }
    }
    for h in workers {
        let _ = h.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn media() -> Media {
        Media {
            a: vec![1; 128],
            v: vec![2; 128],
            mix: (0..128).collect(),
            manifest: json!({"revision":"test"}),
            date: "2026-11-09T15:00:00Z".into(),
        }
    }
    fn state() -> Mutex<State> {
        Mutex::new(State {
            prefs: json!({"schema":1,"teams":[]}),
            version: 1,
            count: 0,
            logout: false,
        })
    }
    fn request(raw: &str, st: &Mutex<State>) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let (s, _) = l.accept().unwrap();
                serve(s, st, &media()).unwrap();
            });
            let mut client = TcpStream::connect(addr).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            client.write_all(raw.as_bytes()).unwrap();
            let mut bytes = Vec::new();
            client.read_to_end(&mut bytes).unwrap();
            worker.join().unwrap();
            String::from_utf8_lossy(&bytes).into_owned()
        })
    }
    fn auth() -> String {
        format!(
            "Authorization: MediaBrowser Client=\"Bluefin Apple\", Device=\"iOS\", DeviceId=\"selftest\", Version=\"0.1.0\", Token=\"{TOKEN}\"\r\n"
        )
    }
    #[test]
    fn rejects_legacy_query_unknown_user_and_unowned_item() {
        let st = state();
        for raw in [
            "GET /Users/Me HTTP/1.1\r\nX-Emby-Token: synthetic-ipad-session-1109\r\n\r\n"
                .to_string(),
            format!("GET /Users/Me?token={TOKEN} HTTP/1.1\r\n{}\r\n", auth()),
        ] {
            assert!(request(&raw, &st).starts_with("HTTP/1.1 401"));
        }
        let body = r#"{"Username":"unknown","Pw":"unknown"}"#;
        assert!(
            request(
                &format!(
                    "POST /Users/AuthenticateByName HTTP/1.1\r\n{}Content-Length: {}\r\n\r\n{body}",
                    auth(),
                    body.len()
                ),
                &st
            )
            .starts_with("HTTP/1.1 401")
        );
        assert!(
            request(
                &format!("GET /Users/{USER}/Items/unowned HTTP/1.1\r\n{}\r\n", auth()),
                &st
            )
            .starts_with("HTTP/1.1 404")
        );
    }
    #[test]
    fn cas_rejects_stale_and_reopens_saved_account() {
        let st = state();
        let body = r#"{"schema":1,"teams":["harbor"],"sports_only":true}"#;
        let put = |tag| {
            format!(
                "PUT /sports/api/v1/preferences HTTP/1.1\r\n{}If-Match: \"{tag}\"\r\nContent-Length: {}\r\n\r\n{body}",
                auth(),
                body.len()
            )
        };
        let first = request(&put(1), &st);
        assert!(first.starts_with("HTTP/1.1 200"));
        assert!(first.contains("ETag: \"2\""));
        assert!(request(&put(1), &st).starts_with("HTTP/1.1 412"));
        let get = request(
            &format!("GET /sports/api/v1/preferences HTTP/1.1\r\n{}\r\n", auth()),
            &st,
        );
        assert!(get.contains("harbor") && get.contains("\"version\":2"));
    }
    #[test]
    fn real_head_and_ranges() {
        let st = state();
        let head = request(
            &format!("HEAD /Audio/{ID}/stream.wav HTTP/1.1\r\n{}\r\n", auth()),
            &st,
        );
        assert!(head.contains("Content-Length: 128") && head.ends_with("\r\n\r\n"));
        let part = request(
            &format!(
                "GET /Audio/{ID}/stream.wav HTTP/1.1\r\n{}Range: bytes=0-3\r\n\r\n",
                auth()
            ),
            &st,
        );
        assert!(part.starts_with("HTTP/1.1 206") && part.contains("Content-Range: bytes 0-3/128"));
        assert!(
            request(
                &format!(
                    "GET /Audio/{ID}/stream.wav HTTP/1.1\r\n{}Range: bytes=200-300\r\n\r\n",
                    auth()
                ),
                &st
            )
            .starts_with("HTTP/1.1 416")
        );
    }
}
