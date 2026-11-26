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

pub(super) fn call(api: &Api<'_>, operation: &str) -> Result<Value> {
    let preferences = operation != "sportsHome";
    let url = api
        .base
        .join(if preferences {
            "sports/api/v1/preferences"
        } else {
            "sports/api/v1/home"
        })
        .map_err(|_| err("Invalid sports endpoint"))?;
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
    let mut value = response_json(response)?;
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
}
