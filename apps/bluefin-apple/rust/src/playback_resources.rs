//! Android process-owned playback resources retained until release or sign-out.
use super::*;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

#[derive(Clone, PartialEq)]
struct Resource {
    owner: String,
    cleanup: Value,
    local: Local,
    remote: bool,
    encoding: bool,
    live: bool,
}
#[derive(Clone, Copy, PartialEq)]
enum Local {
    Pending,
    Stopping,
    Confirmed,
    Unconfirmed,
}
static RESOURCES: OnceLock<Mutex<HashMap<String, Resource>>> = OnceLock::new();
fn resources() -> &'static Mutex<HashMap<String, Resource>> {
    RESOURCES.get_or_init(Default::default)
}
pub(super) fn stop_proxy(v: &Value) -> Result<()> {
    if client_platform::Platform::read(v)? != client_platform::Platform::Android {
        return stream::stop(text(v, "proxyId")?);
    }
    let id = text(v, "proxyId")?;
    let wanted = owner(v)?;
    let entry = resources()
        .lock()
        .map_err(|_| err("Playback registry unavailable"))?
        .get(id)
        .cloned();
    match entry {
        Some(r) if r.owner == wanted => stop(id, &r),
        Some(_) => Err(err("Cleanup authority does not match captured resource")),
        // Successful release already joined and acknowledged this ID. Never
        // operate an unregistered proxy on the strength of a caller-supplied ID.
        None => Ok(()),
    }
}

fn owner(v: &Value) -> Result<String> {
    Ok(format!(
        "{}\0{}\0{}\0{}",
        server(v)?,
        text(v, "userId")?,
        text(v, "deviceId")?,
        // Identity only. Header authorization below validates token syntax; an
        // invalid token must not prevent an independent local-cache result.
        v["token"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 4096)
            .ok_or_else(|| err("Invalid cleanup token identity"))?
    ))
}
fn opaque(v: &Value, key: &str) -> Result<String> {
    let s = text(v, key)?;
    if s.len() > 128
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return Err(err("Invalid cleanup identifier"));
    }
    Ok(s.into())
}
fn cleanup(v: &Value, receipt: &Value) -> Result<Value> {
    let api = Api::new(v)?;
    api.auth(true)?;
    opaque(v, "userId")?;
    opaque(v, "deviceId")?;
    let method = text(receipt, "playMethod")?;
    if !matches!(method, "DirectPlay" | "DirectStream" | "Transcode") {
        return Err(err("Invalid cleanup method"));
    }
    let live = match receipt.get("liveStreamId") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(s)) if s.is_empty() => Value::Null,
        Some(_) => json!(opaque(receipt, "liveStreamId")?),
    };
    Ok(
        json!({"operation":"releasePlayback","clientPlatform":"android",
        "server":api.base.as_str(),"userId":text(v,"userId")?,"token":text(v,"token")?,
        "deviceId":text(v,"deviceId")?,"playSessionId":opaque(receipt,"playSessionId")?,
        "playMethod":method,"liveStreamId":live,"allowLocalHttp":v["allowLocalHttp"]}),
    )
}
fn receipt(id: &str, r: &Resource) -> Value {
    json!({"proxyId":id,"playSessionId":r.cleanup["playSessionId"],
        "playMethod":r.cleanup["playMethod"],"liveStreamId":r.cleanup["liveStreamId"],"remoteReleased":r.remote,
        "encodingReleased":r.encoding,"liveClosed":r.live})
}
/// Only opaque operation IDs are restored. Routes and authentication always come
/// from the captured protected account, never from the receipt or server URLs.
pub(super) fn restore(v: &Value) -> Result<()> {
    let Some(state) = v.get("cleanupResources") else {
        return Ok(());
    };
    if v.to_string().len() > INPUT_MAX
        || state["version"] != 1
        || state.as_object().is_none_or(|o| {
            o.keys()
                .any(|k| !matches!(k.as_str(), "version" | "resources"))
        })
    {
        return Err(err("Invalid cleanup state"));
    }
    let records = state["resources"]
        .as_array()
        .filter(|a| a.len() <= 32)
        .ok_or_else(|| err("Invalid cleanup resources"))?;
    let owner = owner(v)?;
    let mut additions = HashMap::new();
    for record in records {
        let id = opaque(record, "proxyId")?;
        if record.as_object().is_none_or(|r| {
            r.keys().any(|k| {
                !matches!(
                    k.as_str(),
                    "proxyId"
                        | "playSessionId"
                        | "playMethod"
                        | "liveStreamId"
                        | "remoteReleased"
                        | "encodingReleased"
                        | "liveClosed"
                )
            })
        }) {
            return Err(err("Invalid cleanup receipt"));
        }
        let remote = record["remoteReleased"]
            .as_bool()
            .ok_or_else(|| err("Invalid remote receipt"))?;
        let cleanup = cleanup(v, record)?;
        let encoding = record
            .get("encodingReleased")
            .map(|x| x.as_bool().ok_or_else(|| err("Invalid encoding receipt")))
            .transpose()?
            .unwrap_or(remote || cleanup["playMethod"] != "Transcode");
        let live = record
            .get("liveClosed")
            .map(|x| x.as_bool().ok_or_else(|| err("Invalid live receipt")))
            .transpose()?
            .unwrap_or(remote || cleanup["liveStreamId"].is_null());
        if remote && !(encoding && live) {
            return Err(err("Inconsistent remote receipt"));
        }
        let r = Resource {
            owner: owner.clone(),
            cleanup,
            local: Local::Pending,
            remote: encoding && live,
            encoding,
            live,
        };
        if additions.insert(id, r).is_some() {
            return Err(err("Duplicate cleanup resource"));
        }
    }
    let mut all = resources()
        .lock()
        .map_err(|_| err("Playback registry unavailable"))?;
    for (id, r) in &additions {
        if let Some(existing) = all.get(id) {
            if existing.owner != r.owner || existing.cleanup != r.cleanup {
                return Err(err("Cleanup authority does not match captured resource"));
            }
        }
    }
    if all.len() + additions.keys().filter(|id| !all.contains_key(*id)).count() > 32 {
        return Err(err("Unreleased playback resources require sign-out"));
    }
    for (id, r) in additions {
        all.entry(id).or_insert(r);
    }
    Ok(())
}
pub(super) fn snapshot(v: &Value) -> Result<Value> {
    let owner = owner(v)?;
    let all = resources()
        .lock()
        .map_err(|_| err("Playback registry unavailable"))?;
    let mut entries = all
        .iter()
        .filter(|(_, r)| r.owner == owner)
        .collect::<Vec<_>>();
    entries.sort_by_key(|(id, _)| *id);
    Ok(
        json!({"version":1,"resources":entries.into_iter().map(|(id,r)|receipt(id,r)).collect::<Vec<_>>()}),
    )
}
pub(super) fn register(api: &Api<'_>, plan: &Value) -> Result<()> {
    if client_platform::Platform::read(api.v)? != client_platform::Platform::Android {
        return Ok(());
    }
    let id = opaque(plan, "proxyId")?;
    let mut record = plan.clone();
    record["liveStreamId"] = plan["source"]["LiveStreamId"].clone();
    let cleanup = cleanup(api.v, &record)?;
    let encoding = cleanup["playMethod"] != "Transcode";
    let live = cleanup["liveStreamId"].is_null();
    let mut all = resources()
        .lock()
        .map_err(|_| err("Playback registry unavailable"))?;
    if all.len() >= 32 || all.contains_key(&id) {
        return Err(err("Unreleased playback resources require sign-out"));
    }
    all.insert(
        id,
        Resource {
            owner: owner(api.v)?,
            cleanup,
            local: Local::Pending,
            remote: encoding && live,
            encoding,
            live,
        },
    );
    Ok(())
}
fn stop(id: &str, expected: &Resource) -> Result<()> {
    {
        let mut all = resources()
            .lock()
            .map_err(|_| err("Playback registry unavailable"))?;
        let Some(r) = all.get_mut(id) else {
            return Ok(());
        };
        if r.owner != expected.owner || r.cleanup != expected.cleanup {
            return Err(err("Cleanup resource changed"));
        }
        match r.local {
            Local::Confirmed => return Ok(()),
            Local::Stopping | Local::Unconfirmed => return Err(err(
                "Local playback retirement unconfirmed; finish sign out after restarting the app",
            )),
            Local::Pending => r.local = Local::Stopping,
        }
    }
    // No registry mutex spans a join or network request. Failed drain is sticky:
    // stream::stop consumes its join handle, so absence on retry is not proof.
    let result = stream::stop(id);
    let mut all = resources()
        .lock()
        .map_err(|_| err("Playback registry unavailable"))?;
    if let Some(r) = all
        .get_mut(id)
        .filter(|r| r.owner == expected.owner && r.cleanup == expected.cleanup)
    {
        r.local = if result.is_ok() {
            Local::Confirmed
        } else {
            Local::Unconfirmed
        };
    }
    result
}
fn matches(r: &Resource, v: &Value, owner: &str) -> bool {
    r.owner == owner
        && r.cleanup["playSessionId"] == v["playSessionId"]
        && r.cleanup["playMethod"] == v["playMethod"]
        && r.cleanup["liveStreamId"].as_str()
            == v["liveStreamId"].as_str().filter(|s| !s.is_empty())
}
pub(super) fn release(v: &Value) -> Result<()> {
    let owner = owner(v)?;
    let entries: Vec<_> = resources()
        .lock()
        .map_err(|_| err("Playback registry unavailable"))?
        .iter()
        .filter(|(_, r)| matches(r, v, &owner))
        .map(|(id, r)| (id.clone(), r.clone()))
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return Err(err(
            "Captured playback resource unavailable; finish sign out to restore cleanup",
        ));
    }
    let (local, remote) = retire_entries(entries)?;
    if !(local && remote) {
        return Err(err("Playback cleanup incomplete"));
    }
    Ok(())
}
pub(super) fn retire(v: &Value) -> Result<(bool, bool)> {
    let owner = owner(v)?;
    let entries = resources()
        .lock()
        .map_err(|_| err("Playback registry unavailable"))?
        .iter()
        .filter(|(_, r)| r.owner == owner)
        .map(|(id, r)| (id.clone(), r.clone()))
        .collect::<Vec<_>>();
    retire_entries(entries)
}
fn retire_entries(entries: Vec<(String, Resource)>) -> Result<(bool, bool)> {
    let (mut local, mut remote) = (true, true);
    for (id, r) in entries {
        let stopped = stop(&id, &r).is_ok();
        let api = Api::new(&r.cleanup)?;
        let mut encoding = r.encoding;
        let mut live_closed = r.live;
        if !encoding {
            encoding = api
                .cleanup_request(
                    Method::DELETE,
                    &["Videos", "ActiveEncodings"],
                    vec![
                        ("deviceId", text(&r.cleanup, "deviceId")?.into()),
                        ("playSessionId", text(&r.cleanup, "playSessionId")?.into()),
                    ],
                )
                .is_ok();
        }
        if let Some(live) = r.cleanup["liveStreamId"].as_str().filter(|_| !live_closed) {
            live_closed = api
                .cleanup_request(
                    Method::POST,
                    &["LiveStreams", "Close"],
                    vec![("liveStreamId", live.into())],
                )
                .is_ok();
        }
        let released = encoding && live_closed;
        local &= stopped;
        remote &= released;
        {
            let mut all = resources()
                .lock()
                .map_err(|_| err("Playback registry unavailable"))?;
            if let Some(current) = all
                .get_mut(&id)
                .filter(|current| current.owner == r.owner && current.cleanup == r.cleanup)
            {
                current.encoding |= encoding;
                current.live |= live_closed;
                current.remote = current.encoding && current.live;
            }
        }
        if stopped && released {
            let mut all = resources()
                .lock()
                .map_err(|_| err("Playback registry unavailable"))?;
            if all.get(&id).is_some_and(|current| {
                current.owner == r.owner
                    && current.cleanup == r.cleanup
                    && current.local == Local::Confirmed
            }) {
                all.remove(&id);
            }
        }
    }
    Ok((local, remote))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ordinary_release_preserves_progress(encoding_first: bool) {
        let _serial = stream::tests::SERIAL.lock().unwrap();
        let statuses = if encoding_first {
            vec![
                "204 No Content",
                "503 Service Unavailable",
                "204 No Content",
            ]
        } else {
            vec![
                "503 Service Unavailable",
                "204 No Content",
                "204 No Content",
            ]
        };
        let (base, network) = fixture(statuses);
        let v = request(&base);
        let (id, _) = registered(&v, "Transcode", Some("live-a"));
        let mut release = v.clone();
        release["operation"] = json!("releasePlayback");
        release["playSessionId"] = json!("retry-session");
        release["playMethod"] = json!("Transcode");
        release["liveStreamId"] = json!("live-a");
        let response = super::super::envelope(&serde_json::to_vec(&release).unwrap());
        let pending = snapshot(&v).unwrap();
        let outcome = retire(&v).unwrap();
        let requests = network.join().unwrap();
        resources().lock().unwrap().remove(&id);
        assert_eq!(response["ok"], false);
        assert_eq!(pending["resources"][0]["encodingReleased"], encoding_first);
        assert_eq!(pending["resources"][0]["liveClosed"], !encoding_first);
        assert_eq!(response["cleanupResources"], pending);
        assert_eq!(outcome, (true, true));
        let encoding = requests.iter().filter(|r| r.starts_with("DELETE ")).count();
        let live = requests.iter().filter(|r| r.starts_with("POST ")).count();
        assert_eq!(encoding, if encoding_first { 1 } else { 2 });
        assert_eq!(live, if encoding_first { 2 } else { 1 });
    }
    #[test]
    fn ordinary_release_retains_encoding_ack_when_live_close_fails() {
        ordinary_release_preserves_progress(true);
    }
    #[test]
    fn ordinary_release_retains_live_ack_when_encoding_release_fails() {
        ordinary_release_preserves_progress(false);
    }
    #[test]
    fn ordinary_release_retains_remote_success_when_local_drain_fails() {
        let _serial = stream::tests::SERIAL.lock().unwrap();
        let (base, network) = fixture(vec!["204 No Content", "204 No Content"]);
        let mut v = request(&base);
        let api = Api::new(&v).unwrap();
        let (id, _) = stream::start_with_transport(
            api.url(&["Audio", "synthetic", "stream.wav"]).unwrap(),
            "/jellyfin/Audio/synthetic/".into(),
            api.auth(true).unwrap(),
            Some(iroh_transport::Tunnel::failed_drain()),
        )
        .unwrap();
        register(&api, &json!({"proxyId":id,"playSessionId":"failed-local","playMethod":"Transcode","source":{"LiveStreamId":"live-a"}})).unwrap();
        v["operation"] = json!("releasePlayback");
        v["playSessionId"] = json!("failed-local");
        v["playMethod"] = json!("Transcode");
        v["liveStreamId"] = json!("live-a");
        let response = super::super::envelope(&serde_json::to_vec(&v).unwrap());
        let outcome = retire(&v).unwrap();
        let requests = network.join().unwrap();
        resources().lock().unwrap().remove(&id);
        assert_eq!(response["ok"], false);
        assert_eq!(
            response["cleanupResources"]["resources"][0]["remoteReleased"],
            true
        );
        assert_eq!(outcome, (false, true));
        assert_eq!(
            requests.len(),
            2,
            "acknowledged remote work must not repeat"
        );
    }
    #[test]
    fn standalone_failed_stop_must_not_become_successful_retirement() {
        let _serial = stream::tests::SERIAL.lock().unwrap();
        let v = request("https://example.invalid/");
        let api = Api::new(&v).unwrap();
        let (id, _) = stream::start_with_transport(
            api.url(&["Audio", "synthetic", "stream.wav"]).unwrap(),
            "/Audio/synthetic/".into(),
            api.auth(true).unwrap(),
            Some(iroh_transport::Tunnel::failed_drain()),
        )
        .unwrap();
        register(&api, &json!({"proxyId":id,"playSessionId":"standalone-drain","playMethod":"DirectPlay","source":{}})).unwrap();
        let mut stop = v.clone();
        stop["operation"] = json!("stopPlaybackProxy");
        stop["proxyId"] = json!(id);
        assert!(super::super::call(&stop).is_err());
        assert_eq!(
            retire(&v).unwrap(),
            (false, true),
            "standalone failed drain was forgotten before sign-out"
        );
        assert!(resources().lock().unwrap().contains_key(&id));
        // The fixture's failed owner has joined. Remove only its intentionally
        // pending receipt so another test cannot inherit this simulated debt.
        resources().lock().unwrap().remove(&id);
    }
    use std::{io::Write, net::TcpListener, thread};
    fn fixture(statuses: Vec<&'static str>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let mut requests = Vec::new();
            for status in statuses {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = [0; INPUT_MAX];
                let mut n = 0;
                loop {
                    let read = socket.read(&mut bytes[n..]).unwrap();
                    assert!(read > 0);
                    n += read;
                    let request = String::from_utf8_lossy(&bytes[..n]);
                    if let Some(end) = request.find("\r\n\r\n") {
                        let size = request[..end]
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")?
                                    .parse::<usize>()
                                    .ok()
                            })
                            .unwrap_or(0);
                        if n >= end + 4 + size {
                            break;
                        }
                    }
                    assert!(n < INPUT_MAX);
                }
                // Native registry remains available while real HTTP is in flight.
                assert!(resources().try_lock().is_ok());
                requests.push(String::from_utf8_lossy(&bytes[..n]).into_owned());
                write!(
                    socket,
                    "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            }
            requests
        });
        (format!("http://{address}/jellyfin/"), worker)
    }
    fn cache() -> std::path::PathBuf {
        let mut random = [0; 8];
        getrandom::fill(&mut random).unwrap();
        let p = std::path::PathBuf::from(std::env::var("BLUEFIN_TEST_CACHE").unwrap())
            .join(format!("cleanup-{}", u64::from_ne_bytes(random)));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    fn request(server: &str) -> Value {
        json!({"operation":"androidSignOut","clientPlatform":"android","server":server,"allowLocalHttp":true,
            "userId":"retry-owner","deviceId":"retry-device","token":"fixture-token"})
    }
    fn registered(v: &Value, method: &str, live: Option<&str>) -> (String, String) {
        let api = Api::new(v).unwrap();
        let prefix = api.url(&["Audio", "synthetic"]).unwrap().path().to_owned() + "/";
        let (id, url) = stream::start(
            api.url(&["Audio", "synthetic", "stream.wav"]).unwrap(),
            prefix,
            api.auth(true).unwrap(),
        )
        .unwrap();
        register(&api,&json!({"proxyId":id,"playSessionId":"retry-session","playMethod":method,"source":{"LiveStreamId":live}})).unwrap();
        (id, url)
    }
    #[test]
    fn failed_remote_release_keeps_owned_retry_debt() {
        let _serial = stream::tests::SERIAL.lock().unwrap();
        let (server, network) = fixture(vec!["503 Service Unavailable", "204 No Content"]);
        let v = json!({"clientPlatform":"android","server":server,"allowLocalHttp":true,
            "userId":"retry-owner","deviceId":"retry-device","token":"fixture-token"});
        let api = Api::new(&v).unwrap();
        let (id, _) = stream::start(
            api.url(&["Audio", "synthetic", "stream.wav"]).unwrap(),
            "/jellyfin/Audio/synthetic/".into(),
            api.auth(true).unwrap(),
        )
        .unwrap();
        register(&api, &json!({"proxyId":id,"playSessionId":"retry-session","playMethod":"Transcode","source":{}})).unwrap();
        let result = retire(&v).unwrap();
        assert_eq!(result, (true, false));
        assert!(
            resources().lock().unwrap().contains_key(&id),
            "failed remote cleanup lost its captured owner and resource"
        );
        assert_eq!(retire(&v).unwrap(), (true, true));
        assert!(!resources().lock().unwrap().contains_key(&id));
        let requests = network.join().unwrap();
        assert_eq!(requests.len(), 2);
        for request in requests {
            assert!(request.starts_with("DELETE /jellyfin/Videos/ActiveEncodings?deviceId=retry-device&playSessionId=retry-session "));
            assert!(request.contains("Token=\"fixture-token\""));
        }
    }
    #[test]
    fn logout_fails_once_unknown_cache_is_preserved_and_exact_retry_recovers() {
        use sha2::{Digest, Sha256};
        let _serial = stream::tests::SERIAL.lock().unwrap();
        let (server, network) = fixture(vec!["503 Service Unavailable", "204 No Content"]);
        let path = cache();
        let mut v = request(&server);
        v["cacheRoot"] = json!(path);
        offline::call(&v, "downloads").unwrap();
        let hash = Sha256::digest(format!("{}\nretry-owner", server).as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let unknown = path.join(format!("bluefin-{hash}/user-note.txt"));
        std::fs::write(&unknown, b"preserve this original synthetic fixture note").unwrap();
        let first = super::super::call(&v).unwrap();
        assert_eq!(first["localCleanupComplete"], false);
        assert_eq!(first["serverRevoked"], false);
        assert_eq!(first["playbackReleaseComplete"], true);
        let second = super::super::call(&v).unwrap();
        assert_eq!(second["localCleanupComplete"], false);
        assert_eq!(second["serverRevoked"], true);
        assert!(unknown.is_file());
        let requests = network.join().unwrap();
        assert!(requests
            .iter()
            .all(|r| r.starts_with("POST /jellyfin/Sessions/Logout ")
                && r.contains("Token=\"fixture-token\"")));
        std::fs::remove_file(unknown).unwrap(); // Test owner removes its own unexpected fixture.
        v["cleanupServerRevoked"] = json!(true);
        let third = super::super::call(&v).unwrap();
        assert_eq!(third["localCleanupComplete"], true);
        assert_eq!(third["serverRevoked"], true);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn failed_live_release_defers_logout_and_restored_receipt_uses_original_authority() {
        let _serial = stream::tests::SERIAL.lock().unwrap();
        let (server, network) = fixture(vec![
            "503 Service Unavailable",
            "204 No Content",
            "204 No Content",
        ]);
        let path = cache();
        let mut v = request(&server);
        v["cacheRoot"] = json!(path);
        let (id, url) = registered(&v, "DirectPlay", Some("live-a"));
        let first = super::super::call(&v).unwrap();
        assert_eq!(first["localCleanupComplete"], true);
        assert_eq!(first["serverRevoked"], false);
        assert_eq!(first["playbackReleaseComplete"], false);
        assert_eq!(
            first["cleanupResources"]["resources"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(reqwest::blocking::get(url).is_err());
        // Emulate loss of the process registry only after the actual proxy joined.
        // This checks protected receipt rehydration, not an Android process restart.
        resources().lock().unwrap().remove(&id);
        v["cleanupResources"] = first["cleanupResources"].clone();
        let second = super::super::call(&v).unwrap();
        assert_eq!(second["playbackReleaseComplete"], true);
        assert_eq!(second["serverRevoked"], true);
        assert_eq!(second["cleanupResources"]["resources"], json!([]));
        let requests = network.join().unwrap();
        assert!(requests[0].starts_with("POST /jellyfin/LiveStreams/Close?liveStreamId=live-a "));
        assert!(requests[1].starts_with("POST /jellyfin/LiveStreams/Close?liveStreamId=live-a "));
        assert!(requests[2].starts_with("POST /jellyfin/Sessions/Logout "));
        assert!(requests
            .iter()
            .all(|r| r.contains("Token=\"fixture-token\"")
                && r.contains("DeviceId=\"retry-device\"")));
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn partial_remote_success_is_acknowledged_without_replaying_that_operation() {
        let _serial = stream::tests::SERIAL.lock().unwrap();
        let (server, network) = fixture(vec![
            "204 No Content",
            "503 Service Unavailable",
            "204 No Content",
        ]);
        let v = request(&server);
        let (id, _) = registered(&v, "Transcode", Some("live-a"));
        assert_eq!(retire(&v).unwrap(), (true, false));
        let pending = snapshot(&v).unwrap();
        assert_eq!(pending["resources"][0]["encodingReleased"], true);
        assert_eq!(pending["resources"][0]["liveClosed"], false);
        assert_eq!(retire(&v).unwrap(), (true, true));
        assert!(!resources().lock().unwrap().contains_key(&id));
        let requests = network.join().unwrap();
        assert!(requests[0].starts_with("DELETE /jellyfin/Videos/ActiveEncodings?"));
        assert!(requests[1..]
            .iter()
            .all(|r| r.starts_with("POST /jellyfin/LiveStreams/Close?liveStreamId=live-a ")));
    }
    #[test]
    fn token_rotation_or_b_cannot_retire_a_or_restore_over_its_proxy() {
        let _serial = stream::tests::SERIAL.lock().unwrap();
        let v = request("https://example.invalid/");
        let (id, url) = registered(&v, "DirectPlay", None);
        let receipt = snapshot(&v).unwrap();
        for (field, value) in [
            ("token", "token-b"),
            ("userId", "user-b"),
            ("deviceId", "device-b"),
            ("server", "https://other.invalid/"),
        ] {
            let mut b = v.clone();
            b[field] = json!(value);
            assert_eq!(retire(&b).unwrap(), (true, true));
            assert!(resources().lock().unwrap().contains_key(&id));
            b["cleanupResources"] = receipt.clone();
            assert!(restore(&b).is_err());
        }
        let u = Url::parse(&url).unwrap();
        assert!(std::net::TcpStream::connect(("127.0.0.1", u.port().unwrap())).is_ok());
        assert_eq!(retire(&v).unwrap(), (true, true));
        assert!(!resources().lock().unwrap().contains_key(&id));
    }
    #[test]
    fn failed_local_drain_is_sticky_and_preserves_confirmed_remote_receipt() {
        let _serial = stream::tests::SERIAL.lock().unwrap();
        let v = request("https://example.invalid/");
        let api = Api::new(&v).unwrap();
        let (id, _) = stream::start_with_transport(
            api.url(&["Audio", "synthetic", "stream.wav"]).unwrap(),
            "/Audio/synthetic/".into(),
            api.auth(true).unwrap(),
            Some(iroh_transport::Tunnel::failed_drain()),
        )
        .unwrap();
        register(&api,&json!({"proxyId":id,"playSessionId":"failed-drain","playMethod":"DirectPlay","source":{}})).unwrap();
        // Real proxy/join path, existing injected tunnel-drain failure; no claim
        // of a real remote Iroh transport failure or Android process restart.
        assert_eq!(retire(&v).unwrap(), (false, true));
        assert_eq!(retire(&v).unwrap(), (false, true));
        let receipt = snapshot(&v).unwrap();
        assert_eq!(receipt["resources"][0]["remoteReleased"], true);
        resources().lock().unwrap().remove(&id);
        let mut restart = v.clone();
        restart["cleanupResources"] = receipt;
        restore(&restart).unwrap();
        assert_eq!(retire(&restart).unwrap(), (true, true));
    }
    #[test]
    fn bounded_restore_rejects_routes_duplicates_and_overflow_without_partial_mutation() {
        let _serial = stream::tests::SERIAL.lock().unwrap();
        let mut v = request("https://example.invalid/");
        let base = json!({"proxyId":"restored","playSessionId":"session","playMethod":"DirectPlay","remoteReleased":false});
        for records in [
            vec![base.clone(), base.clone()],
            vec![
                json!({"proxyId":"x","playSessionId":"../invalid","playMethod":"DirectPlay","remoteReleased":false}),
            ],
            vec![
                json!({"proxyId":"x","playSessionId":"valid","playMethod":"DirectPlay","remoteReleased":false,"cleanupUrl":"https://other.invalid"}),
            ],
        ] {
            v["cleanupResources"] = json!({"version":1,"resources":records});
            assert!(restore(&v).is_err());
        }
        let entries = (0..32)
            .map(|i| {
                let mut e = base.clone();
                e["proxyId"] = json!(format!("restored-{i}"));
                e
            })
            .collect::<Vec<_>>();
        let before = resources().lock().unwrap().len();
        assert_eq!(before, 0);
        v["cleanupResources"] = json!({"version":1,"resources":entries});
        restore(&v).unwrap();
        assert_eq!(resources().lock().unwrap().len(), 32);
        let mut extra = base.clone();
        extra["proxyId"] = json!("overflow");
        v["cleanupResources"] = json!({"version":1,"resources":[extra]});
        assert!(restore(&v).is_err());
        assert_eq!(resources().lock().unwrap().len(), 32);
        assert_eq!(retire(&v).unwrap(), (true, true));
        assert_eq!(resources().lock().unwrap().len(), 0);
    }
    #[test]
    fn concurrent_release_and_retirement_do_not_lose_failed_remote_debt() {
        let _serial = stream::tests::SERIAL.lock().unwrap();
        let (server, network) = fixture(vec![
            "503 Service Unavailable",
            "503 Service Unavailable",
            "204 No Content",
        ]);
        let v = request(&server);
        let (id, _) = registered(&v, "Transcode", None);
        let mut release = v.clone();
        release["operation"] = json!("releasePlayback");
        release["playSessionId"] = json!("retry-session");
        release["playMethod"] = json!("Transcode");
        let retired = v.clone();
        let a = thread::spawn(move || super::super::call(&release));
        let b = thread::spawn(move || retire(&retired));
        assert!(a.join().unwrap().is_err());
        assert!(!b.join().unwrap().unwrap().1);
        assert!(resources().lock().unwrap().contains_key(&id));
        assert_eq!(retire(&v).unwrap(), (true, true));
        assert!(!resources().lock().unwrap().contains_key(&id));
        assert_eq!(network.join().unwrap().len(), 3);
    }
    #[test]
    fn sign_out_retires_only_owned_streams_and_reports_revocation_independently() {
        let _serial = stream::tests::SERIAL.lock().unwrap();
        let (server, network) = crate::tests::mock("204 No Content", "");
        let path =
            std::path::PathBuf::from(std::env::var("BLUEFIN_TEST_CACHE").unwrap()).join(format!(
                "signout-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        std::fs::create_dir_all(&path).unwrap();
        let v = json!({"operation":"androidSignOut","clientPlatform":"android","server":server,"allowLocalHttp":true,
            "userId":"owner","deviceId":"device","token":"fixture-token","cacheRoot":path});
        let api = Api::new(&v).unwrap();
        let remote = api.url(&["Audio", "song", "stream.wav"]).unwrap();
        let prefix = api.url(&["Audio", "song"]).unwrap().path().to_owned() + "/";
        let (first, first_url) =
            stream::start(remote.clone(), prefix.clone(), api.auth(true).unwrap()).unwrap();
        register(
            &api,
            &json!({"proxyId":first,"playSessionId":"one","playMethod":"DirectPlay","source":{}}),
        )
        .unwrap();
        let mut other = v.clone();
        other["userId"] = json!("other");
        let other_api = Api::new(&other).unwrap();
        let (second, second_url) =
            stream::start(remote, prefix, other_api.auth(true).unwrap()).unwrap();
        register(
            &other_api,
            &json!({"proxyId":second,"playSessionId":"two","playMethod":"DirectPlay","source":{}}),
        )
        .unwrap();
        let result = super::super::call(&v).unwrap();
        assert_eq!(result["localCleanupComplete"], true);
        assert_eq!(result["serverRevoked"], true);
        assert_eq!(result["playbackReleaseComplete"], true);
        let addr = |url: &str| {
            let u = Url::parse(url).unwrap();
            format!("127.0.0.1:{}", u.port().unwrap())
        };
        assert!(std::net::TcpStream::connect(addr(&first_url)).is_err());
        assert!(std::net::TcpStream::connect(addr(&second_url)).is_ok());
        other["operation"] = json!("releasePlayback");
        other["playSessionId"] = json!("two");
        other["playMethod"] = json!("DirectPlay");
        super::super::call(&other).unwrap();
        let request = network.join().unwrap();
        assert!(request.starts_with("POST /jellyfin/Sessions/Logout"));
        assert!(request.contains("Bluefin Android"));
        assert!(!request.contains("Bluefin Apple"));
        let mut invalid = v.clone();
        invalid["token"] = json!("invalid\r\ntoken");
        let result = super::super::call(&invalid).unwrap();
        assert_eq!(result["localCleanupComplete"], true);
        assert_eq!(result["serverRevoked"], false);
        std::fs::remove_dir_all(path).unwrap();
    }
}
