//! Loopback-only synthetic API/media fixture. No external service or account.
use bluefin_contract::{Capabilities, MusicQueue, ViewerCapabilities};
use bluefin_universal::{
    protocol::{Item, items},
    state::{Model, Section},
};
use serde_json::{Value, json};
use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};
use tiny_http::{Header, Request, Response, StatusCode};
use uuid::Uuid;
fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name, value).unwrap()
}
fn json_response(r: Request, value: Value) {
    let _ = r.respond(
        Response::from_string(value.to_string())
            .with_header(header("Content-Type", "application/json"))
            .with_header(header("Cache-Control", "no-store")),
    );
}
fn reject(r: Request, code: u16) {
    let _ = r.respond(Response::empty(code));
}
fn serve_file(r: Request, path: &Path) {
    let Ok(mut file) = std::fs::File::open(path) else {
        reject(r, 404);
        return;
    };
    let length = file.metadata().unwrap().len();
    let mime = match path.extension().and_then(|s| s.to_str()).unwrap_or("") {
        "wasm" => "application/wasm",
        "js" => "text/javascript",
        "html" => "text/html",
        "m3u8" => "application/vnd.apple.mpegurl",
        "ts" => "video/mp2t",
        "mp4" => "video/mp4",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    };
    let mut headers = vec![
        header("Content-Type", mime),
        header("Cache-Control", "no-store"),
        header("Accept-Ranges", "bytes"),
    ];
    let mut status = 200;
    let mut start = 0;
    let mut end = length.saturating_sub(1);
    if let Some(range) = r.headers().iter().find(|h| h.field.equiv("Range")) {
        let value = range.value.as_str().strip_prefix("bytes=").unwrap_or("");
        let Some((a, b)) = value.split_once('-') else {
            reject(r, 416);
            return;
        };
        if a.is_empty() {
            let n = b.parse::<u64>().unwrap_or(0);
            start = length.saturating_sub(n);
        } else {
            start = a.parse::<u64>().unwrap_or(length);
            if !b.is_empty() {
                end = b.parse::<u64>().unwrap_or(0).min(end);
            }
        }
        if start > end || start >= length {
            reject(r, 416);
            return;
        }
        status = 206;
        headers.push(header(
            "Content-Range",
            &format!("bytes {start}-{end}/{length}"),
        ));
    }
    let bytes = end + 1 - start;
    file.seek(SeekFrom::Start(start)).unwrap();
    let _ = r.respond(Response::new(
        StatusCode(status),
        headers,
        file.take(bytes),
        Some(bytes as usize),
        None,
    ));
}
#[derive(Default)]
struct FixtureState {
    tokens: Vec<String>,
    reports: Vec<Value>,
    revocations: u64,
    closed: u64,
    queue: MusicQueue,
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let web = PathBuf::from(args.get(1).expect("web distribution directory"));
    let media = PathBuf::from(args.get(2).expect("fixture media directory"));
    let port = args.get(3).map(String::as_str).unwrap_or("18461");
    let listener = Arc::new(tiny_http::Server::http(format!("127.0.0.1:{port}")).unwrap());
    let state = Arc::new(Mutex::new(FixtureState::default()));
    let epoch = Instant::now();
    println!("Loopback synthetic fixture server ready");
    let mut threads = Vec::new();
    for _ in 0..4 {
        let listener = listener.clone();
        let state = state.clone();
        let web = web.clone();
        let media = media.clone();
        threads.push(std::thread::spawn(move||while let Ok(mut r)=listener.recv(){
        let Ok(url)=url::Url::parse(&format!("http://localhost{}",r.url()))else{reject(r,400);continue;};let path=url.path().to_owned();
        if url.query_pairs().any(|(k,_)|bluefin_universal::protocol::credential(&k)) || r.headers().iter().any(|h|h.field.equiv("X-Emby-Token")||h.field.equiv("X-Emby-Authorization")){reject(r,401);continue;}
        if path=="/fixture/Users/AuthenticateByName"{
            let standard=r.headers().iter().any(|h|h.field.equiv("Authorization")&&h.value.as_str().starts_with("MediaBrowser Client=")&&h.value.as_str().contains("DeviceId="));
            if !standard{reject(r,401);continue;}
            let token=Uuid::new_v4().to_string();state.lock().unwrap().tokens.push(token.clone());
            json_response(r,json!({"AccessToken":token,"User":{"Id":Uuid::from_u128(99)}}));continue;
        }
        if path=="/fixture/evidence"{let s=state.lock().unwrap();json_response(r,json!({"reports":s.reports,"revocations":s.revocations,"closed":s.closed,"activeAccounts":s.tokens.len()}));continue;}
        if path.starts_with("/fixture/"){
            let auth=r.headers().iter().find(|h|h.field.equiv("Authorization")).map(|h|h.value.as_str()).unwrap_or("");
            let found=state.lock().unwrap().tokens.iter().position(|token|auth.contains(&format!("Token=\"{token}\"")));
            let Some(account)=found else{reject(r,401);continue;};
            if path.ends_with("Sessions/Logout"){let mut s=state.lock().unwrap();s.tokens.remove(account);s.revocations+=1;reject(r,204);continue;}
            if path.ends_with("Bluefin/Capabilities"){json_response(r,serde_json::to_value(Capabilities{protocol_version:1,music_queue_sync:true,max_queue_items:512,viewer:ViewerCapabilities{user_id:Uuid::from_u128(99),media_playback:true,audio_transcoding:true,content_downloading:false,live_tv_access:true},discovery:None}).unwrap());continue;}
            if path.ends_with("Bluefin/Music/Queue"){if r.method().as_str()=="PUT"{let mut body=String::new();let _=r.as_reader().take(65536).read_to_string(&mut body);if let Ok(v)=serde_json::from_str::<Value>(&body){let mut s=state.lock().unwrap();s.queue.queue=serde_json::from_value(v["queue"].clone()).unwrap();s.queue.revision+=1;}}let v=serde_json::to_value(&state.lock().unwrap().queue).unwrap();json_response(r,v);continue;}
            if path.contains("/PlaybackInfo"){
                let parts:Vec<_>=path.split('/').collect();let item=Uuid::parse_str(parts[3]).unwrap();let mut body=String::new();let _=r.as_reader().take(65536).read_to_string(&mut body);let b:Value=serde_json::from_str(&body).unwrap();let audio=item==Uuid::from_u128(3)||item==Uuid::from_u128(6);let live=item==Uuid::from_u128(5);let force=b["AudioStreamIndex"].is_number()||b["SubtitleStreamIndex"].as_i64().is_some_and(|n|n>=0);
                let prefix=format!("/fixture/{}/{}/",if audio{"Audio"}else{"Videos"},item.simple());
                let mut source=json!({"Id":"fixture-source","SupportsDirectPlay":!live&&!force,"Container":if audio{"wav"}else{"mp4"},"MediaStreams":[{"Index":0,"Type":"Video","Codec":"h264"},{"Index":1,"Type":"Audio","Codec":"aac","Language":"eng"},{"Index":2,"Type":"Subtitle","Codec":"srt","Language":"eng"}],"TranscodingUrl":format!("{prefix}{}?StartTimeTicks={}",if live{"live.m3u8"}else{"vod.m3u8"},b["StartTimeTicks"].as_u64().unwrap_or(0))});
                if live{source["LiveStreamId"]=json!("synthetic-live");}
                json_response(r,json!({"PlaySessionId":Uuid::new_v4().simple().to_string(),"MediaSources":[source]}));continue;
            }
            if path.contains("/Sessions/Playing"){
                let mut body=String::new();let _=r.as_reader().take(65536).read_to_string(&mut body);let mut v:Value=serde_json::from_str(&body).unwrap();v["phase"]=json!(path);state.lock().unwrap().reports.push(v);reject(r,204);continue;
            }
            if path.ends_with("LiveStreams/Close")||path.ends_with("Videos/ActiveEncodings"){state.lock().unwrap().closed+=1;reject(r,204);continue;}
            if path.contains("/Videos/")||path.contains("/Audio/"){
                let file=path.rsplit('/').next().unwrap();
                if file=="live.m3u8"{
                    let sequence=(epoch.elapsed().as_secs()/2).max(4)-4;let mut playlist=format!("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{sequence}\n");
                    for n in sequence..sequence+5{if n%20==0{playlist.push_str("#EXT-X-DISCONTINUITY\n");}playlist.push_str(&format!("#EXTINF:2.0,\nsegment{:03}.ts\n",n%20));}
                    let _=r.respond(Response::from_string(playlist).with_header(header("Content-Type","application/vnd.apple.mpegurl")).with_header(header("Cache-Control","no-store")));continue;
                }
                let target=if file=="stream"{media.join(if path.contains("/Audio/"){"tone.wav"}else{"pattern.mp4"})}else if file=="vod.m3u8"{media.join("hls/vod.m3u8")}else if file.starts_with("segment")&&file.len()==13&&!file.contains(['%','\\']){media.join("hls").join(file)}else{reject(r,404);continue;};serve_file(r,&target);continue;
            }
            let mut rows=Vec::<Item>::new();for section in [Section::Movies,Section::Music,Section::Guide]{rows.extend(Model{section,..Default::default()}.fixture_rows());}
            if path.ends_with("/Items")||path.ends_with("/LiveTv/Programs"){let _=items(json!(rows.clone())).unwrap();json_response(r,json!({"Items":rows}));continue;}
            if let Some(id)=path.rsplit('/').next().and_then(|s|Uuid::parse_str(s).ok())&&let Some(item)=rows.into_iter().find(|i|i.id==id){json_response(r,json!(item));continue;}
            reject(r,404);continue;
        }
        let relative=path.trim_start_matches('/');let relative=if relative.is_empty(){"index.html"}else{relative};
        if relative.contains("..")||relative.contains(['%','\\']){reject(r,403);continue;}serve_file(r,&web.join(relative));
    }));
    }
    for t in threads {
        let _ = t.join();
    }
}
