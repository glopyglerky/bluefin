//! Optional Bluefin v1 contracts. A missing extension never implies support.
use super::*;
use bluefin_contract::{Capabilities, MusicQueue, QueueState, UpdateMusicQueue, PROTOCOL_VERSION};

pub(super) fn call(api: &Api<'_>, operation: &str) -> Result<Value> {
    match operation {
        "bluefinCapabilities" => {
            let value = api.get(&["Bluefin", "Capabilities"], vec![])?;
            let transport = project_iroh_transport(value.get("irohMediaTransport"));
            let search = project_library_search(value.get("librarySearch"));
            let capabilities: Capabilities =
                serde_json::from_value(value).map_err(|_| err("Invalid Bluefin capabilities"))?;
            if capabilities.protocol_version != PROTOCOL_VERSION
                || capabilities.max_queue_items > bluefin_contract::MAX_QUEUE_ITEMS
                || capabilities.max_queue_items == 0
                || Some(capabilities.viewer.user_id.simple().to_string())
                    != normalized_id(text(api.v, "userId")?)
            {
                return Err(err("Unsupported Bluefin capabilities"));
            }
            let can_play = capabilities.viewer.media_playback;
            let mut projected = serde_json::to_value(capabilities)
                .map_err(|_| err("Invalid Bluefin capabilities"))?;
            if let Some(transport) = transport.filter(|_| can_play) {
                projected["irohMediaTransport"] = transport;
            }
            if let Some(search) = search { projected["librarySearch"] = search; }
            Ok(projected)
        }
        "bluefinMusicQueue" | "saveBluefinMusicQueue" => {
            let value = if operation == "saveBluefinMusicQueue" {
                let update: UpdateMusicQueue = serde_json::from_value(json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "expectedRevision": api.v.get("expectedRevision"),
                    "queue": api.v.get("queue")
                }))
                .map_err(|_| err("Invalid music queue"))?;
                if !update.queue.is_valid() {
                    return Err(err("Invalid music queue"));
                }
                let body = serde_json::to_value(update).map_err(|_| err("Invalid music queue"))?;
                if body.to_string().len() > bluefin_contract::MAX_QUEUE_BODY_BYTES {
                    return Err(err("Music queue too large"));
                }
                api.request(
                    Method::PUT,
                    &["Bluefin", "Music", "Queue"],
                    vec![],
                    Some(body),
                    true,
                )?
            } else {
                api.get(&["Bluefin", "Music", "Queue"], vec![])?
            };
            let saved: MusicQueue =
                serde_json::from_value(value).map_err(|_| err("Invalid music queue response"))?;
            if saved.protocol_version != PROTOCOL_VERSION || !saved.queue.is_valid() {
                return Err(err("Unsupported music queue response"));
            }
            serde_json::to_value(saved).map_err(|_| err("Invalid music queue response"))
        }
        "bluefinMusicQueueItems" => queue_items(api),
        _ => Err(err("Unknown Bluefin operation")),
    }
}

/// Missing or malformed optional search extensions never imply support.
fn project_library_search(value: Option<&Value>) -> Option<Value> {
    let value = value?;
    if value["version"].as_u64() != Some(1) || value["explicitSort"].as_bool() != Some(true) {
        return None;
    }
    let keys = value["sortBy"].as_array()?;
    let directions = value["sortOrder"].as_array()?;
    if keys.is_empty() || keys.len() > 5 || directions.len() != 2 {
        return None;
    }
    if !keys.iter().all(|key| key.as_str().is_some_and(|key|
        ["SortName", "Name", "DateCreated", "PremiereDate", "ProductionYear"].contains(&key)))
        || !directions.contains(&json!("Ascending")) || !directions.contains(&json!("Descending")) {
        return None;
    }
    Some(json!({"version":1,"explicitSort":true,"sortBy":keys,"sortOrder":directions}))
}

/// Optional direct-transport metadata is not an account grant or a reachable-path proof.
/// Unknown/malformed extensions must not break otherwise supported queue capabilities.
fn project_iroh_transport(value: Option<&Value>) -> Option<Value> {
    let mut transport: bluefin_contract::transport::IrohMediaTransport =
        serde_json::from_value(value?.clone()).ok()?;
    if transport.version != 1
        || transport.alpn != "bluefin/http/1"
        || !transport.direct_only
        || transport.endpoint_id.len() != 64
        || !transport
            .endpoint_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || transport.direct_addresses.is_empty()
        || transport.direct_addresses.len() > 16
        || transport.direct_addresses.iter().any(|address| {
            address.len() > 128
                || !address
                    .parse::<std::net::SocketAddr>()
                    .is_ok_and(|address| {
                        address.port() != 0
                            && !address.ip().is_unspecified()
                            && !address.ip().is_multicast()
                    })
        })
    {
        return None;
    }
    let mut seen = std::collections::HashSet::new();
    transport
        .direct_addresses
        .retain(|address| seen.insert(address.clone()));
    serde_json::to_value(transport).ok()
}

fn normalized_id(value: &str) -> Option<String> {
    let value = value.replace('-', "").to_ascii_lowercase();
    (value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit())).then_some(value)
}

fn queue_items(api: &Api<'_>) -> Result<Value> {
    let queue: QueueState =
        serde_json::from_value(api.v["queue"].clone()).map_err(|_| err("Invalid music queue"))?;
    if !queue.is_valid() {
        return Err(err("Invalid music queue"));
    }
    let user = text(api.v, "userId")?;
    let mut ids = queue.item_ids.clone();
    ids.sort();
    ids.dedup();
    let mut items = std::collections::HashMap::new();
    // Keep query lines below common proxy limits. Never issue one request per track.
    for batch in ids.chunks(64) {
        let page = api.get(
            &["Users", user, "Items"],
            vec![
                (
                    "Ids",
                    batch
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(","),
                ),
                ("Limit", "64".into()),
                ("Recursive", "true".into()),
                ("Fields", premium::FIELDS.into()),
            ],
        )?;
        let entries = page["Items"]
            .as_array()
            .filter(|items| items.len() <= 64)
            .ok_or_else(|| err("Invalid music metadata response"))?;
        for item in entries {
            if item["Type"] != "Audio" {
                return Err(err("Queue contains unavailable audio"));
            }
            let id = item["Id"]
                .as_str()
                .and_then(normalized_id)
                .ok_or_else(|| err("Invalid music metadata response"))?;
            if !batch
                .iter()
                .any(|candidate| candidate.simple().to_string() == id)
                || items.contains_key(&id)
            {
                return Err(err("Invalid music metadata response"));
            }
            items.insert(id, premium::project(item));
        }
    }
    let ordered = queue
        .item_ids
        .iter()
        .map(|id| {
            items.get(&id.simple().to_string()).cloned().ok_or_else(|| {
                err("Queue changed. Reload its available tracks before restoring it.")
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(json!({"Items": ordered, "TotalRecordCount": ordered.len()}))
}

#[cfg(test)]
mod tests {

    #[test]
    fn library_search_capability_requires_a_known_complete_advertisement() {
        assert!(super::project_library_search(None).is_none());
        for value in [
            serde_json::json!({"version":2,"explicitSort":true,"sortBy":["SortName"],"sortOrder":["Ascending","Descending"]}),
            serde_json::json!({"version":1,"explicitSort":true}),
            serde_json::json!({"version":1,"explicitSort":true,"sortBy":["Unknown"],"sortOrder":["Ascending","Descending"]}),
            serde_json::json!({"version":1,"explicitSort":false,"sortBy":["SortName"],"sortOrder":["Ascending","Descending"]})
        ] {
            assert!(super::project_library_search(Some(&value)).is_none());
        }
        let valid = serde_json::json!({"version":1,"explicitSort":true,
            "sortBy":["SortName","DateCreated","PremiereDate"],
            "sortOrder":["Ascending","Descending"],"untrustedExtra":"ignored"});
        let projected = super::project_library_search(Some(&valid)).expect("supported search");
        assert_eq!(projected["explicitSort"], true);
        assert!(projected.get("untrustedExtra").is_none());
    }

    use super::*;
    use std::{io::Write, net::TcpListener, thread};
    const A: &str = "00000000-0000-0000-0000-000000000001";
    const B: &str = "00000000-0000-0000-0000-000000000002";
    fn fixture(status: &str, value: Value) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_string();
        (
            format!("http://{address}/media"),
            thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let n = socket.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    let text = String::from_utf8_lossy(&bytes);
                    if let Some(end) = text.find("\r\n\r\n") {
                        let length = text[..end]
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")?
                                    .parse::<usize>()
                                    .ok()
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let body = value.to_string();
                write!(
                    socket,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                String::from_utf8(bytes).unwrap()
            }),
        )
    }
    fn request(server: &str) -> Value {
        json!({"server":server,"allowLocalHttp":true,"token":"viewer-token","userId":A})
    }
    fn queue() -> Value {
        json!({"itemIds":[A,B,A],"cursor":2,"shuffle":true,"repeat":"all"})
    }
    #[test]
    fn queue_body_is_shared_typed_and_revision_conflict_is_not_retried() {
        let (server, worker) = fixture("409 Conflict", json!({"private":"not returned"}));
        let mut request = request(&server);
        request["queue"] = queue();
        request["expectedRevision"] = json!(3);
        let error = call(&Api::new(&request).unwrap(), "saveBluefinMusicQueue").unwrap_err();
        assert!(error.contains("409") && !error.contains("private"));
        let request = worker.join().unwrap();
        assert!(request.starts_with("PUT /media/Bluefin/Music/Queue HTTP/1.1"));
        assert!(
            request.contains("Token=\"viewer-token\"")
                && !request.lines().next().unwrap().contains("viewer-token")
        );
        let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(
            body,
            json!({"protocolVersion":1,"expectedRevision":3,"queue":queue()})
        );
    }
    #[test]
    fn invalid_cursor_and_extra_queue_fields_fail_before_network() {
        let mut request = request("https://example.invalid");
        request["expectedRevision"] = json!(0);
        request["queue"] = queue();
        request["queue"]["cursor"] = json!(3);
        assert_eq!(
            call(&Api::new(&request).unwrap(), "saveBluefinMusicQueue").unwrap_err(),
            "Invalid music queue"
        );
        request["queue"] = queue();
        request["queue"]["viewer"] = json!("other");
        assert_eq!(
            call(&Api::new(&request).unwrap(), "saveBluefinMusicQueue").unwrap_err(),
            "Invalid music queue"
        );
    }
    #[test]
    fn capabilities_are_version_and_current_account_scoped() {
        for user in [A, B] {
            let (server, worker) = fixture(
                "200 OK",
                json!({"protocolVersion":1,"musicQueueSync":true,"maxQueueItems":512,"viewer":{"userId":user,"mediaPlayback":true,"audioTranscoding":true,"contentDownloading":false,"liveTvAccess":false}}),
            );
            let request = request(&server);
            assert_eq!(
                call(&Api::new(&request).unwrap(), "bluefinCapabilities").is_ok(),
                user == A
            );
            assert!(worker
                .join()
                .unwrap()
                .starts_with("GET /media/Bluefin/Capabilities HTTP/1.1"));
        }
    }
    #[test]
    fn capabilities_preserve_supported_account_scoped_iroh_advertisement() {
        let transport = json!({"version":1,"endpointId":"01".repeat(32),"directAddresses":["127.0.0.1:32100","127.0.0.1:32100","[::1]:32100"],"alpn":"bluefin/http/1","directOnly":true,"privateExtra":"never project"});
        for can_play in [false, true] {
            let (server, worker) = fixture(
                "200 OK",
                json!({"protocolVersion":1,"musicQueueSync":true,"maxQueueItems":512,"viewer":{"userId":A,"mediaPlayback":can_play,"audioTranscoding":false,"contentDownloading":false,"liveTvAccess":false},"irohMediaTransport":transport}),
            );
            let value = call(&Api::new(&request(&server)).unwrap(), "bluefinCapabilities").unwrap();
            worker.join().unwrap();
            if can_play {
                assert_eq!(
                    value["irohMediaTransport"]["endpointId"],
                    transport["endpointId"]
                );
                assert_eq!(
                    value["irohMediaTransport"]["directAddresses"],
                    json!(["127.0.0.1:32100", "[::1]:32100"])
                );
                assert!(value["irohMediaTransport"].get("privateExtra").is_none());
            } else {
                assert!(value.get("irohMediaTransport").is_none());
            }
        }
    }
    #[test]
    fn unsupported_iroh_extension_does_not_disable_queue_capabilities() {
        let valid = json!({"version":1,"endpointId":"01".repeat(32),"directAddresses":["127.0.0.1:32100"],"alpn":"bluefin/http/1","directOnly":true});
        for (field, invalid) in [
            ("version", json!(2)),
            ("endpointId", json!("invalid")),
            ("alpn", json!("other")),
            ("directOnly", json!(false)),
            ("directAddresses", json!(["0.0.0.0:1"])),
            ("directAddresses", json!(["127.0.0.1:0"])),
            ("directAddresses", json!(["https://example.invalid"])),
        ] {
            let mut transport = valid.clone();
            transport[field] = invalid;
            let (server, worker) = fixture(
                "200 OK",
                json!({"protocolVersion":1,"musicQueueSync":true,"maxQueueItems":512,"viewer":{"userId":A,"mediaPlayback":true,"audioTranscoding":false,"contentDownloading":false,"liveTvAccess":false},"irohMediaTransport":transport}),
            );
            let value = call(&Api::new(&request(&server)).unwrap(), "bluefinCapabilities").unwrap();
            worker.join().unwrap();
            assert_eq!(value["musicQueueSync"], true);
            assert!(
                value.get("irohMediaTransport").is_none(),
                "unsupported {field}"
            );
        }
        assert!(project_iroh_transport(None).is_none());
        assert!(project_iroh_transport(Some(&Value::Null)).is_none());
    }
    #[test]
    fn metadata_batch_preserves_duplicate_order_and_refuses_missing_audio() {
        for missing in [false, true] {
            let mut items = vec![json!({"Id":B,"Type":"Audio","Name":"B"})];
            if !missing {
                items.push(json!({"Id":A,"Type":"Audio","Name":"A","Path":"private"}));
            }
            let (server, worker) = fixture("200 OK", json!({"Items":items}));
            let mut request = request(&server);
            request["queue"] = queue();
            let result = call(&Api::new(&request).unwrap(), "bluefinMusicQueueItems");
            if missing {
                assert!(result.is_err());
            } else {
                let value = result.unwrap();
                assert_eq!(
                    value["Items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|item| item["Name"].as_str().unwrap())
                        .collect::<Vec<_>>(),
                    ["A", "B", "A"]
                );
                assert!(!value.to_string().contains("private"));
            }
            assert!(worker.join().unwrap().starts_with("GET /media/Users/"));
        }
    }
}
