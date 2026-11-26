//! Account-specific Music Assistant control through its documented HTTP API.
//! This adapter never resolves stream URLs, forwards media credentials, impersonates
//! another Music Assistant user, or redistributes provider audio.
#![forbid(unsafe_code)]

use super::{err, response_json, text, Api, Result};
use serde_json::{json, Value};
use std::collections::HashSet;

const PAGE_LIMIT: usize = 50;
const PLAYER_LIMIT: usize = 200;
const OFFSET_LIMIT: u64 = 100_000;

fn identifier<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    let id = text(value, key)?;
    if id.len() > 512 || matches!(id, "*" | "all") || id.contains("://") {
        return Err(err("Choose one Music Assistant item or player"));
    }
    Ok(id)
}

fn display(value: &Value, fallback: &str, max: usize) -> String {
    value.as_str().unwrap_or(fallback).chars()
        .filter(|c| !c.is_control()).take(max).collect()
}

fn token(value: &Value) -> Result<&str> {
    let token = text(value, "token")?;
    if !token.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(&b)) {
        return Err(err("Invalid Music Assistant access token"));
    }
    Ok(token)
}

fn integration_api(value: &Value) -> Result<Api<'_>> {
    if value.get("mesh").is_some_and(|v| !v.is_null()) {
        return Err(err("Use a separate Music Assistant connection"));
    }
    token(value)?;
    Api::new(value)
}

fn get(api: &Api<'_>, parts: &[&str], authenticated: bool) -> Result<Value> {
    let mut request = api.client.get(api.url(parts)?);
    if authenticated { request = request.bearer_auth(token(api.v)?); }
    let response = request.send()
        .map_err(|_| err("Music Assistant could not be reached. Check the connection and retry."))?;
    checked_response(response)
}

fn checked_response(response: reqwest::blocking::Response) -> Result<Value> {
    match response.status().as_u16() {
        401 | 403 => return Err(err("Music Assistant refused this account or action. Reconnect or check its permissions.")),
        404 => return Err(err("This Music Assistant server does not support the required API.")),
        429 => return Err(err("Music Assistant is busy. Wait before trying again.")),
        _ => {}
    }
    // The shared reader enforces a timeout and an 8 MiB maximum, including
    // responses without Content-Length. Error response bodies are never surfaced.
    let result = response_json(response)?;
    if result.get("error_code").is_some() || result.get("error").is_some() {
        return Err(err("Music Assistant refused this request. Check the selected account and item."));
    }
    Ok(result)
}

fn command(api: &Api<'_>, name: &str, args: Value, changes_playback: bool) -> Result<Value> {
    let request = json!({"message_id":"bluefin-music-assistant","command":name,"args":args});
    let response = api.client.post(api.url(&["api"])?)
        .bearer_auth(token(api.v)?).json(&request).send().map_err(|_| {
            err(if changes_playback {
                "The playback request may have reached Music Assistant. Refresh the queue before trying again."
            } else {
                "Music Assistant could not be reached. Check the connection and retry."
            })
        })?;
    checked_response(response)
}

fn page(value: &Value) -> Result<(usize, u64)> {
    let limit = value.get("limit").map_or(Ok(25), |v| v.as_u64().ok_or_else(|| err("Invalid Music Assistant page size")))?;
    let offset = value.get("offset").map_or(Ok(0), |v| v.as_u64().ok_or_else(|| err("Invalid Music Assistant page offset")))?;
    if limit == 0 || limit > PAGE_LIMIT as u64 || offset > OFFSET_LIMIT {
        return Err(err("Music Assistant pages contain at most 50 items"));
    }
    Ok((limit as usize, offset))
}

fn track_uri(uri: &str) -> bool {
    if uri.is_empty() || uri.len() > 2048 || uri.chars().any(char::is_control)
        || uri.contains(['\\', '?', '#']) {
        return false;
    }
    let Some((provider, item)) = uri.split_once("://") else { return false };
    if provider.is_empty() || provider.len() > 128
        || !provider.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        || matches!(provider.to_ascii_lowercase().as_str(), "http" | "https" | "file" | "data" | "ftp" | "url") {
        return false;
    }
    item.strip_prefix("track/").is_some_and(|id| !id.is_empty())
}

fn normalized_track(item: &Value) -> Result<Value> {
    let uri = item["uri"].as_str().ok_or_else(|| err("Music Assistant returned an invalid track"))?;
    if !track_uri(uri) || item["media_type"].as_str() != Some("track") {
        return Err(err("Music Assistant returned an unsupported track"));
    }
    let title = display(&item["name"], "Untitled track", 500);
    let artists = item["artists"].as_array().map(|items| items.iter().take(8)
        .map(|artist| display(&artist["name"], "", 200))
        .filter(|name| !name.is_empty()).collect::<Vec<_>>()).unwrap_or_default();
    let duration = item["duration"].as_f64().filter(|n| n.is_finite() && (0.0..=604_800.0).contains(n));
    Ok(json!({
        "id":uri, "uri":uri, "title":title, "artist":artists.join(", "),
        "album":display(&item["album"]["name"], "", 300),
        "duration":duration, "available":item["available"].as_bool().unwrap_or(true),
        "provider":display(&item["provider"], "", 128)
    }))
}

fn track_page(result: &Value, limit: usize, offset: u64) -> Result<Value> {
    let items = result.as_array().ok_or_else(|| err("Music Assistant returned an invalid track list"))?;
    // Some versions append artist matches to library search results. Return a
    // bounded page while advancing over the entries actually considered.
    let considered = items.len().min(limit);
    let mut tracks = Vec::with_capacity(considered);
    let mut seen = HashSet::new();
    for item in items.iter().take(considered) {
        let track = normalized_track(item)?;
        if seen.insert(track["id"].as_str().unwrap_or_default().to_owned()) {
            tracks.push(track);
        }
    }
    Ok(json!({"items":tracks,"nextOffset":offset + considered as u64,"hasMore":items.len() >= limit}))
}

fn players(result: &Value) -> Result<Value> {
    let items = result.as_array().ok_or_else(|| err("Music Assistant returned an invalid player list"))?;
    if items.len() > PLAYER_LIMIT {
        return Err(err("Music Assistant returned too many players"));
    }
    let mut output = Vec::new();
    let mut seen = HashSet::new();
    for item in items {
        // A device-private player is not a general remote control target.
        if item["enabled"] == false || item["hide_in_ui"] == true || item["private"] == true { continue; }
        let id = identifier(item, "player_id")?;
        if !seen.insert(id) { return Err(err("Music Assistant returned duplicate players")); }
        output.push(json!({
            "id":id, "name":display(&item["name"], id, 200),
            "available":item["available"].as_bool().unwrap_or(false),
            "state":display(&item["playback_state"], "unknown", 40),
            "grouped":item["synced_to"].is_string() || item["active_group"].is_string()
                || item["group_members"].as_array().is_some_and(|items| items.len() > 1),
            "volume":item["volume_level"].as_u64().filter(|n| *n <= 100),
        }));
    }
    Ok(json!(output))
}

fn supports(commands: &[Value], name: &str, supplied: &[&str]) -> bool {
    let mut matches = commands.iter().filter(|v| v["command"].as_str() == Some(name));
    let Some(command) = matches.next() else { return false };
    if matches.next().is_some() { return false; }
    let Some(params) = command["parameters"].as_array() else { return false };
    if params.len() > 64 { return false; }
    supplied.iter().all(|name| params.iter().any(|v| v["name"].as_str() == Some(name)))
        && params.iter().all(|param| param["required"] != true
            || param["name"].as_str().is_some_and(|name| supplied.contains(&name)))
}

fn capabilities(value: &Value) -> Result<Value> {
    let commands = value.as_array().ok_or_else(|| err("Music Assistant API description is unavailable"))?;
    if commands.len() > 2048 { return Err(err("Music Assistant API description is too large")); }
    let mut result = json!({
        "search":supports(commands,"music/search",&["search_query","media_types","limit","library_only"]),
        "library":supports(commands,"music/tracks/library_items",&["limit","offset"]),
        "players":supports(commands,"players/all",&["return_unavailable","return_disabled"]),
        "queue":supports(commands,"player_queues/get_active_queue",&["player_id"])
            && supports(commands,"player_queues/items",&["queue_id","limit","offset"]),
        "play":supports(commands,"player_queues/play_media",&["queue_id","media","option"])
    });
    for (key, name) in [
        ("resume","player_queues/play"),("pause","player_queues/pause"),
        ("next","player_queues/next"),("previous","player_queues/previous"),
        ("stop","player_queues/stop")
    ] {
        result[key] = json!(supports(commands,name,&["queue_id"]));
    }
    Ok(result)
}

fn connect(api: &Api<'_>) -> Result<Value> {
    let info = get(api, &["info"], false)?;
    let server_id = identifier(&info, "server_id")?;
    if let Some(expected) = api.v.get("expectedServerId") {
        if expected.as_str() != Some(server_id) {
            return Err(err("This address now identifies a different Music Assistant server. Disconnect before connecting it."));
        }
    }
    // Validate the public server identity before sending saved credentials.
    let account = get(api, &["auth", "me"], true)?;
    let user_id = identifier(&account, "user_id")?;
    if account["enabled"] == false {
        return Err(err("This Music Assistant account is disabled"));
    }
    if let Some(expected) = api.v.get("expectedUserId") {
        if expected.as_str() != Some(user_id) {
            return Err(err("This token now identifies a different Music Assistant account. Disconnect before connecting it."));
        }
    }
    let schema = get(api, &["api-docs", "commands.json"], true)?;
    let capabilities = capabilities(&schema)?;
    if capabilities["players"] != true || (capabilities["search"] != true && capabilities["library"] != true) {
        return Err(err("This Music Assistant version does not expose the supported library and player commands."));
    }
    Ok(json!({
        "server":api.base.as_str(), "serverId":server_id, "userId":user_id,
        "name":display(&info["name"], "Music Assistant", 200),
        "username":display(&account["display_name"], account["username"].as_str().unwrap_or("Music Assistant account"), 200),
        "version":display(&info["server_version"], "unknown", 80),
        "schemaVersion":info["schema_version"].as_u64(),
        "capabilities":capabilities
    }))
}

fn queue(api: &Api<'_>) -> Result<Value> {
    let player = identifier(api.v, "playerId")?;
    let state = command(api, "player_queues/get_active_queue", json!({"player_id":player}), false)?;
    if state.is_null() { return Ok(json!({"id":null,"name":"","state":"idle","items":[]})); }
    let queue_id = identifier(&state, "queue_id")?;
    let result = command(api, "player_queues/items", json!({"queue_id":queue_id,"limit":50,"offset":0}), false)?;
    let items = result.as_array().ok_or_else(|| err("Music Assistant returned an invalid queue"))?;
    let mut seen = HashSet::new();
    let output = items.iter().take(PAGE_LIMIT).map(|item| {
        let id = identifier(item, "queue_item_id")?;
        if !seen.insert(id) { return Err(err("Music Assistant returned duplicate queue items")); }
        Ok(json!({
            "id":id, "title":display(&item["name"], "Queued item", 500),
            "artist":item["media_item"]["artists"].as_array().map(|artists| artists.iter().take(8)
                .map(|artist| display(&artist["name"],"",200)).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
            "current":state["current_item"]["queue_item_id"].as_str() == Some(id)
        }))
    }).collect::<Result<Vec<_>>>()?;
    Ok(json!({
        "id":queue_id, "name":display(&state["display_name"],"Queue",200),
        "state":display(&state["state"],"unknown",40),
        "items":output, "total":state["items"].as_u64(),
        "truncated":items.len() >= PAGE_LIMIT
    }))
}

pub(super) fn call(value: &Value, operation: &str) -> Result<Value> {
    let api = integration_api(value)?;
    match operation {
        "musicAssistantConnect" => connect(&api),
        "musicAssistantPlayers" => players(&command(&api, "players/all",
            json!({"return_unavailable":true,"return_disabled":false}), false)?),
        "musicAssistantLibrary" => {
            let (limit, offset) = page(value)?;
            let result = command(&api, "music/tracks/library_items", json!({"limit":limit,"offset":offset}), false)?;
            track_page(&result, limit, offset)
        },
        "musicAssistantSearch" => {
            let query = text(value, "query")?.trim();
            if query.is_empty() || query.len() > 512 { return Err(err("Enter a search of up to 512 bytes")); }
            let (limit, _) = page(value)?;
            let result = command(&api, "music/search",
                json!({"search_query":query,"media_types":["track"],"limit":limit,"library_only":true}), false)?;
            let mut result = track_page(&result["tracks"], limit, 0)?;
            // Upstream search has no offset. Do not invent a next page.
            result["hasMore"] = json!(false);
            Ok(result)
        },
        "musicAssistantQueue" => queue(&api),
        "musicAssistantPlay" => {
            let player = identifier(value, "playerId")?;
            let uri = text(value, "uri")?;
            if !track_uri(uri) { return Err(err("Choose a track from the Music Assistant library")); }
            let option = text(value, "mode")?;
            if !matches!(option, "play" | "add") { return Err(err("Choose Play now or Add to queue")); }
            let result = command(&api, "player_queues/play_media",
                json!({"queue_id":player,"media":uri,"option":option}), true)?;
            if !result.is_null() {
                return Err(err("Music Assistant returned an unexpected playback response. Refresh the queue before trying again."));
            }
            Ok(Value::Null)
        },
        "musicAssistantControl" => {
            let queue = identifier(value, "queueId")?;
            let command_name = match text(value, "action")? {
                "play" => "player_queues/play", "pause" => "player_queues/pause",
                "next" => "player_queues/next", "previous" => "player_queues/previous",
                "stop" => "player_queues/stop",
                _ => return Err(err("Unsupported Music Assistant playback action"))
            };
            let result = command(&api, command_name, json!({"queue_id":queue}), true)?;
            if !result.is_null() {
                return Err(err("Music Assistant returned an unexpected playback response. Refresh the queue before trying again."));
            }
            Ok(Value::Null)
        },
        _ => Err(err("Unsupported Music Assistant operation"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::mock;

    fn request(operation: &str, server: &str) -> Value {
        json!({"operation":operation,"server":server,"allowLocalHttp":true,"token":"fixture-ma-token"})
    }

    fn fixture_track() -> Value {
        json!({"uri":"library://track/42","name":"Original song","media_type":"track",
            "provider":"library","artists":[{"name":"Independent artist"}],"duration":120,
            "metadata":{"private_notes":"not for the UI"},"provider_mappings":[{"access_token":"secret"}]})
    }

    fn sequence(responses: Vec<Value>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::{Duration, Instant};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let worker = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "Missing Music Assistant request");
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("Fixture accept failed: {error}"),
                    }
                };
                stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 4096];
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0, "Incomplete Music Assistant request");
                    bytes.extend_from_slice(&buffer[..count]);
                    assert!(bytes.len() <= 65_536, "Music Assistant fixture request too large");
                    let request = String::from_utf8_lossy(&bytes);
                    if let Some(end) = request.find("\r\n\r\n") {
                        let length = request[..end].lines().find_map(|line| {
                            line.to_ascii_lowercase().strip_prefix("content-length: ")?
                                .parse::<usize>().ok()
                        }).unwrap_or(0);
                        if bytes.len() >= end + 4 + length { break; }
                    }
                }
                requests.push(String::from_utf8(bytes).unwrap());
                let body = response.to_string();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
            requests
        });
        (format!("http://{address}/ma/"), worker)
    }

    fn fixture_schema() -> Value {
        json!([
            {"command":"players/all","parameters":[
                {"name":"return_unavailable","required":false},{"name":"return_disabled","required":false}]},
            {"command":"music/tracks/library_items","parameters":[
                {"name":"limit","required":false},{"name":"offset","required":false}]}
        ])
    }

    #[test]
    fn connection_checks_live_identity_and_schema_before_enabling_actions() {
        let (server, worker) = sequence(vec![
            json!({"server_id":"server-one","server_version":"fixture","schema_version":31,"name":"Library"}),
            json!({"user_id":"listener-one","username":"listener","enabled":true,"role":"admin"}),
            fixture_schema(),
        ]);
        let result = crate::call(&request("musicAssistantConnect", &server)).unwrap();
        assert_eq!(result["serverId"], "server-one");
        assert_eq!(result["userId"], "listener-one");
        assert_eq!(result["capabilities"]["players"], true);
        assert_eq!(result["capabilities"]["library"], true);
        assert_eq!(result["capabilities"]["play"], false);
        assert!(result.get("role").is_none());
        assert!(result.get("token").is_none());
        let requests = worker.join().unwrap();
        assert!(requests[0].starts_with("GET /ma/info "));
        assert!(!requests[0].to_ascii_lowercase().contains("authorization:"));
        assert!(requests[1].starts_with("GET /ma/auth/me "));
        assert!(requests[2].starts_with("GET /ma/api-docs/commands.json "));
        for request in &requests[1..] {
            assert!(request.to_ascii_lowercase().contains("authorization: bearer fixture-ma-token"));
        }
    }

    #[test]
    fn restored_connections_reject_changed_server_and_user_identity() {
        let (server, worker) = mock("200 OK", r#"{"server_id":"replacement"}"#);
        let mut input = request("musicAssistantConnect", &server);
        input["expectedServerId"] = json!("original");
        let error = crate::call(&input).unwrap_err();
        assert!(error.contains("different Music Assistant server"));
        let received = worker.join().unwrap();
        assert!(received.starts_with("GET /jellyfin/info "));
        assert!(!received.to_ascii_lowercase().contains("authorization:"));

        let (server, worker) = sequence(vec![
            json!({"server_id":"original"}), json!({"user_id":"different-user","enabled":true}),
        ]);
        let mut input = request("musicAssistantConnect", &server);
        input["expectedServerId"] = json!("original");
        input["expectedUserId"] = json!("original-user");
        let error = crate::call(&input).unwrap_err();
        assert!(error.contains("different Music Assistant account"));
        assert_eq!(worker.join().unwrap().len(), 2);
    }

    #[test]
    fn active_queue_resolves_selected_player_and_hides_stream_details() {
        let (server, worker) = sequence(vec![
            json!({"queue_id":"active-queue","display_name":"Selected room","state":"playing","items":1,
                "current_item":{"queue_item_id":"entry-one"}}),
            json!([{"queue_item_id":"entry-one","name":"Original song","media_item":fixture_track(),
                "streamdetails":{"path":"private-stream-url","token":"secret"}}]),
        ]);
        let mut input = request("musicAssistantQueue", &server);
        input["playerId"] = json!("selected-player");
        let result = crate::call(&input).unwrap();
        assert_eq!(result["id"], "active-queue");
        assert_eq!(result["items"][0]["current"], true);
        assert!(!result.to_string().contains("private-stream-url"));
        assert!(!result.to_string().contains("secret"));
        let requests = worker.join().unwrap();
        let bodies: Vec<Value> = requests.iter().map(|request| {
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap()
        }).collect();
        assert_eq!(bodies[0]["command"], "player_queues/get_active_queue");
        assert_eq!(bodies[0]["args"], json!({"player_id":"selected-player"}));
        assert_eq!(bodies[1]["command"], "player_queues/items");
        assert_eq!(bodies[1]["args"], json!({"queue_id":"active-queue","limit":50,"offset":0}));
    }

    #[test]
    fn duplicate_tracks_do_not_create_duplicate_view_identities() {
        let track = fixture_track();
        let result = track_page(&json!([track.clone(), track]), 2, 20).unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 1);
        assert_eq!(result["nextOffset"], 22);
        assert_eq!(result["hasMore"], true);
        assert!(players(&json!([{"player_id":"duplicate"},{"player_id":"duplicate"}])).is_err());
    }

    #[test]
    fn search_uses_authorized_library_and_drops_private_fields() {
        let (server, worker) = mock("200 OK", &json!({"tracks":[fixture_track()]}).to_string());
        let mut input = request("musicAssistantSearch", &server);
        input["query"] = json!("Original song");
        let result = crate::call(&input).unwrap();
        assert_eq!(result["items"][0]["title"], "Original song");
        assert!(!result.to_string().contains("private_notes"));
        assert!(!result.to_string().contains("access_token"));
        let received = worker.join().unwrap();
        let (headers, body) = received.split_once("\r\n\r\n").unwrap();
        assert!(headers.starts_with("POST /jellyfin/api "));
        assert!(headers.to_ascii_lowercase().contains("authorization: bearer fixture-ma-token"));
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["command"], "music/search");
        assert_eq!(body["args"], json!({"search_query":"Original song","media_types":["track"],"limit":25,"library_only":true}));
        assert!(body.get("token").is_none());
        assert!(body["args"].get("user").is_none());
        assert_eq!(result["hasMore"], false);
    }

    #[test]
    fn play_and_add_use_the_selected_player_and_never_extract_a_stream() {
        for mode in ["play", "add"] {
            let (server, worker) = mock("200 OK", "null");
            let mut input = request("musicAssistantPlay", &server);
            input["playerId"] = json!("selected-speaker");
            input["uri"] = json!("library://track/42");
            input["mode"] = json!(mode);
            assert!(crate::call(&input).unwrap().is_null());
            let received = worker.join().unwrap();
            let (_, body) = received.split_once("\r\n\r\n").unwrap();
            let body: Value = serde_json::from_str(body).unwrap();
            assert_eq!(body["command"], "player_queues/play_media");
            assert_eq!(body["args"], json!({"queue_id":"selected-speaker","media":"library://track/42","option":mode}));
        }
    }

    #[test]
    fn unsafe_targets_urls_and_impersonation_actions_are_not_dispatched() {
        let mut input = request("musicAssistantPlay", "https://example.invalid");
        input["playerId"] = json!("all");
        input["uri"] = json!("library://track/42");
        input["mode"] = json!("play");
        assert!(crate::call(&input).is_err());
        input["playerId"] = json!("speaker");
        for uri in ["https://example.test/song","file://track/private","data://track/secret","library://album/42","library://track/42?token=secret"] {
            input["uri"] = json!(uri);
            assert!(crate::call(&input).is_err());
        }
        input["operation"] = json!("musicAssistantControl");
        input["queueId"] = json!("speaker");
        input["action"] = json!("users/impersonate");
        assert!(crate::call(&input).is_err());
        input["operation"] = json!("musicAssistantPlayers");
        input["mesh"] = json!({});
        assert!(crate::call(&input).is_err());
    }

    #[test]
    fn players_are_bounded_and_private_players_are_not_remote_targets() {
        let result = players(&json!([
            {"player_id":"speaker","name":"Room","available":true,"playback_state":"playing","device_info":{"ip_address":"private"}},
            {"player_id":"browser","name":"Other browser","available":true,"private":true},
            {"player_id":"disabled","name":"Disabled","available":true,"enabled":false}
        ])).unwrap();
        assert_eq!(result.as_array().unwrap().len(), 1);
        assert_eq!(result[0]["id"], "speaker");
        assert!(!result.to_string().contains("ip_address"));
        assert!(players(&json!(vec![json!({"player_id":"x"}); PLAYER_LIMIT + 1])).is_err());
    }

    #[test]
    fn incompatible_required_arguments_disable_capability() {
        let mut entry = json!({"command":"player_queues/play_media","parameters":[
            {"name":"queue_id","required":true},{"name":"media","required":true},{"name":"option","required":false}
        ]});
        assert!(supports(&[entry.clone()], "player_queues/play_media", &["queue_id","media","option"]));
        entry["parameters"].as_array_mut().unwrap().push(json!({"name":"new_required","required":true}));
        assert!(!supports(&[entry], "player_queues/play_media", &["queue_id","media","option"]));
    }

    #[test]
    fn page_limits_and_error_responses_do_not_leak_server_details() {
        assert!(page(&json!({"limit":51})).is_err());
        assert!(page(&json!({"limit":0})).is_err());
        assert!(page(&json!({"offset":OFFSET_LIMIT+1})).is_err());
        let (server, worker) = mock("401 Unauthorized", r#"{"error":"secret-token"}"#);
        let error = crate::call(&request("musicAssistantPlayers", &server)).unwrap_err();
        worker.join().unwrap();
        assert!(error.contains("refused"));
        assert!(!error.contains("secret-token"));
    }
}
