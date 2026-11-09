use super::*;
use std::{net::TcpListener, thread};
static FFI_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn ffi(value: Value) -> Value {
    let input = CString::new(value.to_string()).unwrap();
    unsafe {
        let output = bluefin_call(input.as_ptr());
        let result =
            serde_json::from_str(std::ffi::CStr::from_ptr(output).to_str().unwrap()).unwrap();
        bluefin_free(output);
        result
    }
}
#[test]
fn cleanup_under_saturated_ffi_closes_listener() {
    use std::{
        io::Write,
        sync::{Arc, Barrier},
    };
    let _serial = stream::tests::SERIAL.lock().unwrap();
    let _ffi = FFI_TEST_LOCK.lock().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let ready = Arc::new(Barrier::new(5));
    let release = Arc::new(Barrier::new(5));
    let r = ready.clone();
    let done = release.clone();
    let server = thread::spawn(move || {
        let mut workers = Vec::new();
        for _ in 0..4 {
            let (mut socket, _) = listener.accept().unwrap();
            let r = r.clone();
            let done = done.clone();
            workers.push(thread::spawn(move || {
                let mut bytes = [0; 16384];
                let read = socket.read(&mut bytes).unwrap();
                assert!(read > 0);
                r.wait();
                done.wait();
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                    )
                    .unwrap();
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
    });
    let clients = (0..4).map(|_| thread::spawn(move || ffi(json!({"operation":"info","server":format!("http://{address}"),"allowLocalHttp":true})))).collect::<Vec<_>>();
    let (id, local) = stream::start(
        Url::parse(&format!("http://{address}/Videos/i/file.mp4")).unwrap(),
        "/Videos/i/".into(),
        "viewer".into(),
    )
    .unwrap();
    ready.wait();
    let result = ffi(json!({"operation":"stopPlaybackProxy","proxyId":id}));
    release.wait();
    for client in clients {
        assert_eq!(client.join().unwrap()["ok"], true);
    }
    server.join().unwrap();
    let url = Url::parse(&local).unwrap();
    let closed = std::net::TcpStream::connect(("127.0.0.1", url.port().unwrap())).is_err();
    stream::stop(&id).unwrap();
    assert_eq!(
        result["ok"], true,
        "Cleanup must bypass ordinary FFI saturation"
    );
    assert!(closed);
}
#[test]
fn lossless_source_wins_regardless_of_source_order() {
    let _guard = stream::tests::SERIAL.lock().unwrap();
    for reverse in [false, true] {
        let direct = json!({"Id":"lossless","Container":"flac","SupportsDirectPlay":true,"MediaStreams":[{"Type":"Audio","Codec":"flac"}]});
        let lossy = json!({"Id":"lossy","TranscodingUrl":"Audio/i/master.m3u8","MediaStreams":[{"Type":"Audio","Codec":"wma"}]});
        let sources = if reverse {
            vec![direct, lossy]
        } else {
            vec![lossy, direct]
        };
        let (server, worker) = mock(
            "200 OK",
            &json!({"PlaySessionId":"p","MediaSources":sources}).to_string(),
        );
        let plan = call(&json!({"operation":"playback","mediaType":"Audio","server":server,"allowLocalHttp":true,"token":"viewer-token","userId":"u","itemId":"i"})).unwrap();
        worker.join().unwrap();
        stream::stop(plan["proxyId"].as_str().unwrap()).unwrap();
        assert_eq!(plan["mediaSourceId"], "lossless");
        assert_eq!(plan["playMethod"], "DirectPlay");
    }
}
#[test]
fn rejected_preparation_closes_opened_live_source() {
    use std::io::Write;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let mut requests = Vec::new();
        for body in [
            r#"{"PlaySessionId":"p","MediaSources":[{"Id":"m","LiveStreamId":"live-one","Container":"mkv","SupportsDirectPlay":false}]}"#,
            "",
        ] {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut buffer = [0; 16384];
            let size = socket.read(&mut buffer).unwrap();
            requests.push(String::from_utf8_lossy(&buffer[..size]).into_owned());
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .unwrap();
        }
        requests
    });
    let error = call(&json!({"operation":"playback","server":format!("http://{address}"),"allowLocalHttp":true,"token":"viewer","userId":"u","itemId":"i"})).unwrap_err();
    let requests = worker.join().unwrap();
    assert_eq!(error, "No supported Apple playback source");
    assert!(requests[1].starts_with("POST /LiveStreams/Close?liveStreamId=live-one "));
    assert!(requests[1].contains("Token=\"viewer\""));
}
#[test]
fn favorite_updates_are_user_scoped() {
    for (favorite, method) in [(true, "POST"), (false, "DELETE")] {
        let (server, worker) = mock("200 OK", "{}");
        call(&json!({"operation":"favorite","server":server,"allowLocalHttp":true,"token":"viewer-token","userId":"viewer","itemId":"track","favorite":favorite})).expect("favorite operation must be implemented");
        let request = worker.join().unwrap();
        assert!(request.starts_with(&format!(
            "{method} /jellyfin/Users/viewer/FavoriteItems/track "
        )));
    }
}
#[test]
fn home_assistant_filters_private_attributes_and_scopes_actions() {
    let (server, worker) = mock(
        "200 OK",
        r#"[{"entity_id":"media_player.kitchen","state":"playing","attributes":{"friendly_name":"Kitchen","media_title":"A song","media_content_id":"secret-url","access_token":"secret"}},{"entity_id":"sensor.secret","state":"password"}]"#,
    );
    let result = call(&json!({"operation":"homeAssistantStates","server":server,"allowLocalHttp":true,"token":"ha-token"})).unwrap();
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result[0]["entityId"], "media_player.kitchen");
    assert!(!result.to_string().contains("secret"));
    let request = worker.join().unwrap();
    assert!(request.starts_with("GET /jellyfin/api/states "));
    assert!(request
        .to_ascii_lowercase()
        .contains("authorization: bearer ha-token"));
    assert!(!request.lines().next().unwrap().contains("ha-token"));
    for (entity, service) in [
        ("sensor.secret", "media_stop"),
        ("media_player.kitchen", "turn_on"),
        ("media_player.*", "media_stop"),
    ] {
        assert!(call(&json!({"operation":"homeAssistantControl","server":"https://example.com","token":"ha-token","entityId":entity,"service":service})).is_err());
    }
}
#[test]
fn lossless_audio_prefers_native_direct_stream_over_lossy_transcode() {
    let _guard = stream::tests::SERIAL.lock().unwrap();
    let (server, worker) = mock(
        "200 OK",
        r#"{"PlaySessionId":"p","MediaSources":[{"Id":"m","Container":"flac","SupportsDirectPlay":true,"TranscodingUrl":"Audio/i/master.m3u8","MediaStreams":[{"Type":"Audio","Codec":"flac","BitDepth":24,"SampleRate":192000}]}]}"#,
    );
    let plan = call(&json!({"operation":"playback","mediaType":"Audio","server":server,"allowLocalHttp":true,"token":"viewer-token","userId":"u","itemId":"i"})).unwrap();
    worker.join().unwrap();
    stream::stop(plan["proxyId"].as_str().unwrap()).unwrap();
    assert_eq!(plan["playMethod"], "DirectPlay");
    assert!(plan["remoteUrl"].as_str().unwrap().contains("stream.flac"));
}
#[test]
fn release_playback_is_session_scoped_and_idempotent_for_direct_media() {
    let (server, worker) = mock("204 No Content", "");
    call(&json!({"operation":"releasePlayback","server":server,"allowLocalHttp":true,"token":"viewer-token","deviceId":"device-one","playSessionId":"session-one","playMethod":"Transcode"})).unwrap();
    let request = worker.join().unwrap();
    assert!(request.starts_with(
        "DELETE /jellyfin/Videos/ActiveEncodings?deviceId=device-one&playSessionId=session-one "
    ));
    let (server, worker) = mock("204 No Content", "");
    call(&json!({"operation":"releasePlayback","server":server,"allowLocalHttp":true,"token":"viewer-token","playMethod":"DirectPlay","liveStreamId":"live-one"})).unwrap();
    assert!(worker
        .join()
        .unwrap()
        .starts_with("POST /jellyfin/LiveStreams/Close?liveStreamId=live-one "));
    call(&json!({"operation":"releasePlayback","server":"https://example.com","token":"viewer-token","playMethod":"DirectPlay"})).unwrap();
}
pub(crate) fn mock(status: &str, body: &str) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let h = thread::spawn(move || {
        use std::io::Write;
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut buf = [0; 65536];
        let mut n = 0;
        loop {
            let read = s.read(&mut buf[n..]).unwrap();
            assert!(read > 0, "Incomplete request");
            n += read;
            let request = String::from_utf8_lossy(&buf[..n]);
            if let Some(end) = request.find("\r\n\r\n") {
                let length = request[..end]
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")?
                            .parse::<usize>()
                            .ok()
                    })
                    .unwrap_or(0);
                if n >= end + 4 + length {
                    break;
                }
            }
            assert!(n < buf.len(), "Request too large for fixture");
        }
        s.write_all(response.as_bytes()).unwrap();
        String::from_utf8_lossy(&buf[..n]).into_owned()
    });
    (format!("http://{addr}/jellyfin/"), h)
}

#[test]
fn premium_playback_source_and_quality_reach_the_planner() {
    let _guard = stream::tests::SERIAL.lock().unwrap();
    for (quality, expected) in [
        ("automatic", 80_000_000u64),
        ("original", 200_000_000),
        ("balanced", 12_000_000),
        ("dataSaver", 3_000_000),
    ] {
        let (server, worker) = mock(
            "200 OK",
            &json!({"PlaySessionId":"p","MediaSources":[
                {"Id":"other","TranscodingUrl":"Videos/i/other.m3u8"},
                {"Id":"selected","TranscodingUrl":"Videos/i/selected.m3u8"}
            ]})
            .to_string(),
        );
        let plan = call(&json!({"operation":"playback","server":server,"allowLocalHttp":true,"token":"viewer","userId":"u","itemId":"i","mediaSourceId":"selected","quality":quality,"audioStreamIndex":1,"subtitleStreamIndex":-1})).unwrap();
        stream::stop(plan["proxyId"].as_str().unwrap()).unwrap();
        let request = worker.join().unwrap();
        let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(plan["mediaSourceId"], "selected");
        assert_eq!(body["MediaSourceId"], "selected");
        assert_eq!(body["DeviceProfile"]["MaxStreamingBitrate"], expected);
        assert_eq!(body["MaxStreamingBitrate"], expected);
        assert_eq!(
            body["EnableDirectPlay"], false,
            "An explicit audio track must not be ignored by static direct play"
        );
        assert_eq!(body["SubtitleStreamIndex"], -1);
    }
}
#[test]
fn url_security() {
    for u in [
        "https://user:secret@example.com",
        "https://example.com?api_key=secret",
        "https://example.com/#x",
        "http://example.com",
        "file:///tmp",
        "https://example.com/%2f/",
    ] {
        assert!(
            server(&json!({"server":u,"allowLocalHttp":true})).is_err(),
            "{u}"
        );
    }
    assert!(server(&json!({"server":"http://127.0.0.1"})).is_err());
    assert!(server(&json!({"server":"http://127.0.0.1","allowLocalHttp":true})).is_ok());
    let base = Url::parse("https://example.com/jellyfin/").unwrap();
    for raw in [
        "https://evil.test/v?api_key=secret",
        "//evil.test/v",
        "http://example.com/v",
        "https://user:secret@example.com/v",
        "\\\\evil.test/v",
    ] {
        assert!(playback_url(&base, raw).is_err());
    }
    assert_eq!(
        playback_url(&base, "Videos/x/master.m3u8").unwrap().path(),
        "/jellyfin/Videos/x/master.m3u8"
    );
}
#[test]
fn identifiers_and_escaping() {
    let v = json!({"server":"https://example.com/base","token":"secret"});
    let a = Api::new(&v).unwrap();
    assert_eq!(
        a.url(&["Users", "name space"]).unwrap().as_str(),
        "https://example.com/base/Users/name%20space"
    );
    for id in ["..", "a/b", "a%2fb", "x?api_key=secret"] {
        assert!(a.url(&["Items", id]).is_err());
    }
}
#[test]
fn token_header_and_search_query() {
    let (server, h) = mock("200 OK", r#"{"Items":[],"TotalRecordCount":0}"#);
    let r=call(&json!({"operation":"items","server":server,"allowLocalHttp":true,"token":"secret-token","userId":"u","searchTerm":"a&api_key=oops # /"})).unwrap();
    assert_eq!(r["TotalRecordCount"], 0);
    let req = h.join().unwrap();
    let first = req.lines().next().unwrap();
    assert!(first.starts_with("GET /jellyfin/Users/u/Items?"));
    assert!(!first.contains("secret-token"));
    assert!(first.contains("a%26api_key%3Doops"));
    assert!(req.contains("Token=\"secret-token\""));
}
#[test]
fn server_errors_are_safe_and_session_failure_propagates() {
    for op in ["info", "started", "progress", "stopped", "logout"] {
        let (server, h) = mock("401 Unauthorized", "secret password token");
        let e=call(&json!({"operation":op,"server":server,"allowLocalHttp":true,"token":"secret","itemId":"i","mediaSourceId":"m","playSessionId":"p","playMethod":"Transcode"})).unwrap_err();
        assert_eq!(e, "Server returned HTTP 401");
        let req = h.join().unwrap();
        let path = match op {
            "started" => "/Sessions/Playing",
            "progress" => "/Sessions/Playing/Progress",
            "stopped" => "/Sessions/Playing/Stopped",
            "logout" => "/Sessions/Logout",
            _ => "/System/Info/Public",
        };
        assert!(req.lines().next().unwrap().contains(path));
    }
}
#[test]
fn redirects_and_foreign_transcode_rejected() {
    let (server, h) = mock("302 Found", "");
    assert!(
        call(&json!({"operation":"info","server":server,"allowLocalHttp":true}))
            .unwrap_err()
            .contains("302")
    );
    h.join().unwrap();
    let (server, h) = mock(
        "200 OK",
        r#"{"PlaySessionId":"p","MediaSources":[{"Id":"m","TranscodingUrl":"https://evil.test/master.m3u8?api_key=secret"}]}"#,
    );
    assert!(call(&json!({"operation":"playback","server":server,"allowLocalHttp":true,"token":"secret","userId":"u","itemId":"i"})).unwrap_err().contains("server origin"));
    h.join().unwrap();
}
#[test]
fn playback_preserves_metadata_and_removes_query_credentials() {
    let _guard = stream::tests::SERIAL.lock().unwrap();
    let (server, h) = mock(
        "200 OK",
        r#"{"PlaySessionId":"p","MediaSources":[{"Id":"m","TranscodingUrl":"Videos/i/master.m3u8?api_key=secret&x=1&StartTimeTicks=123"}]}"#,
    );
    let r=call(&json!({"operation":"playback","server":server,"allowLocalHttp":true,"token":"secret","userId":"u","itemId":"i","startTicks":123})).unwrap();
    assert!(r["remoteUrl"]
        .as_str()
        .unwrap()
        .ends_with("/jellyfin/Videos/i/master.m3u8?x=1&StartTimeTicks=123"));
    assert!(r["url"].as_str().unwrap().starts_with("http://127.0.0.1:"));
    assert_eq!(r["startOffsetTicks"], 123);
    call(&json!({"operation":"stopPlaybackProxy","proxyId":r["proxyId"]})).unwrap();
    assert_eq!(r["startTicks"], 123);
    assert_eq!(r["playSessionId"], "p");
    h.join().unwrap();
}
#[test]
fn limits_and_ffi() {
    let _ffi = FFI_TEST_LOCK.lock().unwrap();
    assert!(count(&json!({"limit":201}), "limit", 50, 200).is_err());
    assert!(index(&json!({"audioStreamIndex":-2}), "audioStreamIndex").is_err());
    let s = CString::new("{bad").unwrap();
    unsafe {
        let p = bluefin_call(s.as_ptr());
        let v: Value = serde_json::from_slice(std::ffi::CStr::from_ptr(p).to_bytes()).unwrap();
        assert_eq!(v["ok"], false);
        bluefin_free(p);
        let p = bluefin_call(std::ptr::null());
        bluefin_free(p);
        bluefin_free(std::ptr::null_mut());
    }
}

#[test]
fn auth_injection_and_oversized_input_rejected() {
    let _ffi = FFI_TEST_LOCK.lock().unwrap();
    for extra in [
        json!({"token":"secret\"Injected"}),
        json!({"deviceId":"device\"Injected","token":"valid"}),
    ] {
        let mut v = json!({"server":"https://example.test","token":"valid"});
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        assert!(Api::new(&v).unwrap().auth(true).is_err());
    }
    let input = CString::new(vec![b' '; INPUT_MAX + 1]).unwrap();
    unsafe {
        let p = bluefin_call(input.as_ptr());
        let s = std::ffi::CStr::from_ptr(p).to_str().unwrap();
        assert!(s.contains("Input too large"));
        bluefin_free(p);
    }
}
#[test]
fn oversized_response_is_rejected_before_reading() {
    use std::io::Write;
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let h = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut b = [0; 8192];
        assert!(s.read(&mut b).unwrap() > 0);
        s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 999999999\r\n\r\n")
            .unwrap();
    });
    assert_eq!(
        call(&json!({"operation":"info","server":format!("http://{addr}"),"allowLocalHttp":true}))
            .unwrap_err(),
        "Server response too large"
    );
    h.join().unwrap();
}
#[test]
fn home_fails_if_any_component_fails() {
    let (server, h) = mock("503 Service Unavailable", "private server detail");
    assert_eq!(call(&json!({"operation":"home","server":server,"allowLocalHttp":true,"token":"valid","userId":"u"})).unwrap_err(),"Server returned HTTP 503");
    h.join().unwrap();
}
#[test]
fn unsupported_source_fails_closed() {
    let (server, h) = mock(
        "200 OK",
        r#"{"PlaySessionId":"p","MediaSources":[{"Id":"m","SupportsDirectPlay":true,"Container":"mkv","MediaStreams":[{"Type":"Video","Codec":"hevc"}]}]}"#,
    );
    assert_eq!(call(&json!({"operation":"playback","server":server,"allowLocalHttp":true,"token":"valid","userId":"u","itemId":"i"})).unwrap_err(),"No supported Apple playback source");
    h.join().unwrap();
}

#[test]
fn standalone_audio_plan() {
    let _guard = stream::tests::SERIAL.lock().unwrap();
    let (server, h) = mock(
        "200 OK",
        r#"{"PlaySessionId":"p","MediaSources":[{"Id":"m","TranscodingUrl":"Audio/song/master.m3u8?AudioCodec=aac","MediaStreams":[{"Type":"Audio","Codec":"flac"}]}]}"#,
    );
    let r=call(&json!({"operation":"playback","server":server,"allowLocalHttp":true,"token":"secret","userId":"u","itemId":"song","mediaType":"Audio","startTicks":987})).unwrap();
    assert!(r["remoteUrl"]
        .as_str()
        .unwrap()
        .contains("/Audio/song/master.m3u8?"));
    assert!(r["remoteUrl"].as_str().unwrap().contains("AudioCodec=aac"));
    assert_eq!(r["startOffsetTicks"], 0);
    assert_eq!(r["startTicks"], 987);
    call(&json!({"operation":"stopPlaybackProxy","proxyId":r["proxyId"]})).unwrap();
    let request = h.join().unwrap();
    assert!(request.contains("PlaybackInfo"));
}
#[test]
fn premium_music_relationships_and_projection() {
    let (server, h) = mock(
        "200 OK",
        r#"{"Items":[{"Id":"song","AlbumId":"album","ArtistItems":[{"Id":"artist","Name":"Artist"}],"Path":"/private","Chapters":[{"Name":"Part","StartPositionTicks":20,"ImageTag":"tag"}],"MediaStreams":[{"Type":"Audio","Codec":"flac","Path":"secret"}]}],"TotalRecordCount":12}"#,
    );
    let r=call(&json!({"operation":"musicItems","server":server,"allowLocalHttp":true,"token":"secret","userId":"u","kind":"songs","artistId":"artist","albumId":"album","sortBy":"IndexNumber","isFavorite":true,"startIndex":4,"limit":2})).unwrap();
    let request = h.join().unwrap();
    let u = Url::parse(&format!(
        "http://localhost{}",
        request
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
    ))
    .unwrap();
    let q = u.query_pairs().collect::<std::collections::HashMap<_, _>>();
    assert_eq!(q["artistIds"], "artist");
    assert_eq!(q["parentId"], "album");
    assert_eq!(q["isFavorite"], "true");
    assert_eq!(r["TotalRecordCount"], 12);
    assert_eq!(r["Items"][0]["AlbumId"], "album");
    assert!(r["Items"][0].get("Path").is_none());
    assert!(r["Items"][0]["MediaStreams"][0].get("Path").is_none());
}
#[test]
fn premium_playlist_checked_semantics() {
    for (op, key, path, method) in [
        (
            "addPlaylistItems",
            "itemIds",
            "/jellyfin/Playlists/p/Items",
            "POST",
        ),
        (
            "removePlaylistItems",
            "entryIds",
            "/jellyfin/Playlists/p/Items",
            "DELETE",
        ),
        ("renamePlaylist", "name", "/jellyfin/Playlists/p", "POST"),
    ] {
        let (server, h) = mock("204 No Content", "");
        let mut v = json!({"operation":op,"server":server,"allowLocalHttp":true,"token":"secret","userId":"u","playlistId":"p"});
        v[key] = if key == "name" {
            json!("New name")
        } else {
            json!(["entry"])
        };
        assert_eq!(call(&v).unwrap()["updated"], true);
        let req = h.join().unwrap();
        assert!(req.starts_with(&format!("{method} {path}")));
        if key == "entryIds" {
            assert!(req.contains("entryIds=entry"));
            assert!(!req.contains("?ids="));
        }
    }
    let (server, h) = mock("403 Forbidden", "");
    assert_eq!(call(&json!({"operation":"renamePlaylist","server":server,"allowLocalHttp":true,"token":"secret","userId":"u","playlistId":"p","name":"N"})).unwrap_err(),"Server returned HTTP 403");
    h.join().unwrap();
    let (server, h) = mock("200 OK", r#"{"Type":"Audio"}"#);
    assert!(call(&json!({"operation":"deletePlaylist","server":server,"allowLocalHttp":true,"token":"secret","userId":"u","playlistId":"song"})).unwrap_err().contains("non-playlist"));
    h.join().unwrap();
}
#[test]
fn premium_lyrics_and_live_contract() {
    let (server, h) = mock(
        "200 OK",
        r#"{"Metadata":{"IsSynced":true,"Offset":50},"Lyrics":[{"Text":"line","Start":100}]}"#,
    );
    let r=call(&json!({"operation":"lyrics","server":server,"allowLocalHttp":true,"token":"secret","userId":"u","itemId":"song"})).unwrap();
    assert_eq!(r["Lyrics"][0]["Start"], 100);
    assert!(h
        .join()
        .unwrap()
        .starts_with("GET /jellyfin/Audio/song/Lyrics "));
    let (server, h) = mock(
        "200 OK",
        r#"{"Items":[{"Id":"p","StartDate":"2026-11-08T12:00:00Z","ChannelId":"c"}],"TotalRecordCount":1}"#,
    );
    let r=call(&json!({"operation":"livePrograms","server":server,"allowLocalHttp":true,"token":"secret","userId":"u","channelIds":["c"],"minEndDate":"2026-11-08T12:00:00Z"})).unwrap();
    assert_eq!(r["Items"][0]["ChannelId"], "c");
    let req = h.join().unwrap();
    assert!(req.contains("channelIds=c"));
    assert!(!req.contains("isSports"));
}
