//! Optional same-origin sports presentation API. Native playback remains authoritative.
use super::*;

fn revision(tag: &str) -> Option<u64> {
    let value = tag.strip_prefix('"')?.strip_suffix('"')?;
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let version = value.parse::<u64>().ok()?;
    (tag == format!("\"{version}\"")).then_some(version)
}

fn provider_call(api: &Api<'_>, operation: &str) -> Result<Value> {
    let league = text(api.v, "league")?;
    if !["nhl", "nfl", "nba", "mlb", "wnba", "epl", "mls"].contains(&league) {
        return Err(err("Unsupported sports league"));
    }
    let value = match operation {
        "sportsScoreboard" => {
            let date = text(api.v, "date")?;
            if !valid_date(date) { return Err(err("Invalid sports date")); }
            api.get(&["sports", "api", "v1", "scoreboard"],
                    vec![("league", league.to_owned()), ("date", date.to_owned())])?
        }
        "sportsEvent" => {
            let id = text(api.v, "eventId")?;
            if id.is_empty() || id.len() > 20 || !id.bytes().all(|b| b.is_ascii_digit()) {
                return Err(err("Invalid sports event"));
            }
            api.get(&["sports", "api", "v1", "events", league, id], vec![])?
        }
        "sportsStandings" => api.get(&["sports", "api", "v1", "standings", league], vec![])?,
        _ => return Err(err("Unsupported sports operation")),
    };
    let content_valid = match operation {
        "sportsScoreboard" => value["events"].is_array(),
        "sportsEvent" => value["event"].is_object()
            && value["event"]["id"].as_str() == api.v["eventId"].as_str()
            && value["event"]["league"].as_str() == Some(league),
        "sportsStandings" => value["groups"].is_array(),
        _ => false,
    };
    if value["schema"] != 1 || value["provider"] != "ESPN"
        || value["league"].as_str() != Some(league) || !content_valid {
        return Err(err("Unsupported sports response"));
    }
    Ok(value)
}

fn valid_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-'
        || !bytes.iter().enumerate().all(|(i,b)| i == 4 || i == 7 || b.is_ascii_digit()) {
        return false;
    }
    let year = value[..4].parse::<u32>().unwrap_or(0);
    let month = value[5..7].parse::<u32>().unwrap_or(0);
    let day = value[8..].parse::<u32>().unwrap_or(0);
    if !(1900..=2200).contains(&year) { return false; }
    let max = match month {
        1|3|5|7|8|10|12 => 31,
        4|6|9|11 => 30,
        2 => if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) { 29 } else { 28 },
        _ => return false,
    };
    (1..=max).contains(&day)
}

pub(super) fn call(api: &Api<'_>, operation: &str) -> Result<Value> {
    if matches!(operation, "sportsScoreboard" | "sportsEvent" | "sportsStandings") {
        return provider_call(api, operation);
    }

    let preferences = operation != "sportsHome";
    let url = api
        .base
        .join(if preferences {
            "sports/api/v1/preferences"
        } else {
            "sports/api/v1/home"
        })
        .map_err(|_| err("Invalid sports endpoint"))?;
    if api.v["mesh"].is_object() {
        let mut headers=Vec::new();
        let body=if operation=="saveSportsPreferences" {
            let tag=text(api.v,"etag")?;
            if revision(tag).is_none(){return Err(err("Invalid preference revision"));}
            let value=api.v.get("preferences").filter(|p|p.is_object()).ok_or_else(||err("Missing preferences"))?;
            headers.push(("if-match".into(),tag.to_owned()));
            Some(value.clone())
        } else {None};
        let (value,tag)=mesh_connection::request_with_headers(api.v,
            if operation=="saveSportsPreferences" {"PUT"}else{"GET"},&url,body,headers)?;
        return finish(value,tag,preferences);
    }
    let mut request = api
        .client
        .request(
            if operation == "saveSportsPreferences" {
                Method::PUT
            } else {
                Method::GET
            },
            url,
        )
        .header("Authorization", api.auth(true)?);
    if operation == "saveSportsPreferences" {
        let tag = api.v["etag"]
            .as_str()
            .ok_or_else(|| err("Invalid preference revision"))?;
        if revision(tag).is_none() {
            return Err(err("Invalid preference revision"));
        }
        let value = api
            .v
            .get("preferences")
            .filter(|p| p.is_object())
            .ok_or_else(|| err("Missing preferences"))?;
        if value.to_string().len() > 60_000 {
            return Err(err("Preferences too large"));
        }
        // The API accepts only the preferences object, never a caller UID/version envelope.
        request = request.header("If-Match", tag).json(value);
    }
    let response = request
        .send()
        .map_err(|_| err("Sports request failed or timed out"))?;
    let tag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    finish(response_json(response)?,tag,preferences)
}
fn finish(mut value:Value,tag:Option<String>,preferences:bool)->Result<Value>{
    if preferences {
        if !value["version"].is_u64() || !value["preferences"].is_object() {
            return Err(err("Invalid preference response"));
        }
        let tag = tag
            .filter(|tag| revision(tag) == value["version"].as_u64())
            .ok_or_else(|| err("Missing or inconsistent preference revision"))?;
        value["etag"] = json!(tag);
    } else if value["schema"] != 1
        || !value["games"].is_array()
        || !value["local_broadcasts"].is_array()
        || value["presentation_only"] != true
    {
        return Err(err("Unsupported sports response"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener, thread};
    fn fixture(
        status: &str,
        tag: Option<&str>,
        body: Value,
    ) -> (Value, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_string();
        let tag = tag
            .map(|tag| format!("ETag: {tag}\r\n"))
            .unwrap_or_default();
        let request = json!({"server":format!("http://{address}/media"),"allowLocalHttp":true,"token":"viewer-token","userId":"caller-ignored"});
        (
            request,
            thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 4096];
                    let n = socket.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    let request = String::from_utf8_lossy(&bytes);
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
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let body = body.to_string();
                write!(socket, "HTTP/1.1 {status}\r\n{tag}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                String::from_utf8(bytes).unwrap()
            }),
        )
    }
    #[test]
    fn operations_respect_configured_server_mount() {
        for mount in ["/", "/media", "/nested/media/"] {
            for operation in ["sportsPreferences", "saveSportsPreferences", "sportsHome"] {
                let home = operation == "sportsHome";
                let body = if home {
                    json!({"schema":1,"games":[],"local_broadcasts":[],"presentation_only":true})
                } else {
                    json!({"version":7,"preferences":{}})
                };
                let (mut request, worker) = fixture("200 OK", Some("\"7\""), body);
                let mut origin = Url::parse(request["server"].as_str().unwrap()).unwrap();
                origin.set_path(mount);
                request["server"] = json!(origin.as_str());
                if operation == "saveSportsPreferences" {
                    request["etag"] = json!("\"6\"");
                    request["preferences"] = json!({"teams":[]});
                }
                assert!(call(&Api::new(&request).unwrap(), operation).is_ok());
                let observed = worker.join().unwrap();
                let method = if operation == "saveSportsPreferences" {
                    "PUT"
                } else {
                    "GET"
                };
                let resource = if home { "home" } else { "preferences" };
                let expected = format!(
                    "{method} {}/sports/api/v1/{resource} HTTP/1.1",
                    mount.trim_end_matches('/')
                );
                assert!(
                    observed.starts_with(&expected),
                    "{operation} lost mount {mount}"
                );
            }
        }
    }
    #[test]
    fn read_uses_server_mount_auth_and_matching_revision() {
        for tag in [Some("\"7\""), Some("\"8\""), None] {
            let (request, worker) = fixture(
                "200 OK",
                tag,
                json!({"version":7,"preferences":{"teams":["specific-team"],"auto_record":false}}),
            );
            let result = call(&Api::new(&request).unwrap(), "sportsPreferences");
            if tag == Some("\"7\"") {
                let value = result.unwrap();
                assert_eq!(value["etag"], "\"7\"");
                assert_eq!(value["preferences"]["teams"], json!(["specific-team"]));
            } else {
                assert!(result.is_err());
            }
            let request = worker.join().unwrap();
            assert!(request.starts_with("GET /media/sports/api/v1/preferences HTTP/1.1"));
            assert!(request.contains("Token=\"viewer-token\""));
            assert!(
                !request.contains("caller-ignored")
                    && !request.to_ascii_lowercase().contains("cookie:")
            );
        }
    }
    #[test]
    fn save_sends_preferences_only_and_preserves_conflict() {
        let (mut request, worker) = fixture(
            "412 Precondition Failed",
            None,
            json!({"private":"not exposed"}),
        );
        request["etag"] = json!("\"7\"");
        request["preferences"] = json!({"teams":["specific-team"],"auto_record":false});
        let error = call(&Api::new(&request).unwrap(), "saveSportsPreferences").unwrap_err();
        assert!(error.contains("412") && !error.contains("private"));
        let observed = worker.join().unwrap();
        assert!(observed.starts_with("PUT /media/sports/api/v1/preferences HTTP/1.1"));
        assert!(observed.to_ascii_lowercase().contains("if-match: \"7\""));
        assert_eq!(
            serde_json::from_str::<Value>(observed.split("\r\n\r\n").nth(1).unwrap()).unwrap(),
            request["preferences"]
        );
    }
    #[test]
    fn malformed_revision_is_rejected_before_network() {
        for tag in [
            "",
            "\"",
            "\"\"",
            "W/\"1\"",
            "\"+1\"",
            "\"01\"",
            "\"é\"",
            "\"18446744073709551616\"",
        ] {
            let request = json!({"server":"https://example.invalid","token":"viewer-token","etag":tag,"preferences":{}});
            assert_eq!(
                call(&Api::new(&request).unwrap(), "saveSportsPreferences").unwrap_err(),
                "Invalid preference revision"
            );
        }
    }
    #[test]
    fn home_preserves_unknown_identity_without_inventing_playback() {
        let home = json!({"schema":1,"games":[],"local_broadcasts":[{"kind":"ON_NOW","event_identity":null,"playback_verified":false}],"presentation_only":true});
        let (request, worker) = fixture("200 OK", None, home.clone());
        assert_eq!(
            call(&Api::new(&request).unwrap(), "sportsHome").unwrap(),
            home
        );
        assert!(worker
            .join()
            .unwrap()
            .starts_with("GET /media/sports/api/v1/home HTTP/1.1"));
    }
    #[test]
    fn provider_routes_keep_mount_and_auth_and_bind_response() {
        for (op, resource, content) in [
            ("sportsScoreboard", "scoreboard?league=nhl&date=2026-11-27", json!({"events":[]})),
            ("sportsEvent", "events/nhl/401000123", json!({"event":{"id":"401000123","league":"nhl"}})),
            ("sportsStandings", "standings/nhl", json!({"groups":[]})),
        ] {
            let mut body=json!({"schema":1,"provider":"ESPN","league":"nhl","fetchedAt":"2026-11-27T00:00:00Z","stale":false});
            body.as_object_mut().unwrap().extend(content.as_object().unwrap().clone());
            let (mut request, observed)=fixture("200 OK",None,body);
            request["league"]=json!("nhl"); request["date"]=json!("2026-11-27");
            request["eventId"]=json!("401000123");
            assert!(call(&Api::new(&request).unwrap(),op).is_ok());
            let observed=observed.join().unwrap();
            assert!(observed.starts_with(&format!("GET /media/sports/api/v1/{resource} HTTP/1.1")));
            assert!(observed.to_ascii_lowercase().contains("authorization: mediabrowser"));
            assert!(!observed.lines().next().unwrap().contains("viewer-token"));
        }
        let (mut request, observed)=fixture("200 OK",None,json!({"schema":1,"provider":"ESPN","league":"nfl","events":[]}));
        request["league"]=json!("nhl");request["date"]=json!("2026-11-27");
        assert!(call(&Api::new(&request).unwrap(),"sportsScoreboard").is_err());
        observed.join().unwrap();
    }

    #[test]
    fn provider_routes_reject_invalid_selectors_before_network() {
        let mut request=json!({"server":"http://127.0.0.1:1/media","allowLocalHttp":true,"token":"viewer-token","league":"nhl","date":"2026-11-27","eventId":"401000123"});
        for date in ["2026-02-29","2026-04-31","2026-00-01","2026-11-00","2026-1-01","2026-11-27&url=evil","２０２６-11-27"] {
            request["date"]=json!(date);
            assert_eq!(call(&Api::new(&request).unwrap(),"sportsScoreboard").unwrap_err(),"Invalid sports date");
        }
        assert!(valid_date("2024-02-29"));
        assert!(!valid_date("2100-02-29"));
        request["date"]=json!("2026-11-27");
        for league in ["NHL","hockey/nhl","../nhl","https://example.com"] {
            request["league"]=json!(league);
            assert_eq!(call(&Api::new(&request).unwrap(),"sportsScoreboard").unwrap_err(),"Unsupported sports league");
        }
        request["league"]=json!("nhl");
        for id in ["","1/2","%2e%2e","１２３","123?x=1"] {
            request["eventId"]=json!(id);
            assert_eq!(call(&Api::new(&request).unwrap(),"sportsEvent").unwrap_err(),"Invalid sports event");
        }
    }

}
