//! Authenticated library trailer discovery. No external stream resolution.
use super::*;
use std::borrow::Cow;

pub(super) fn call(api: &Api<'_>, operation: &str) -> Result<Value> {
    let user = text(api.v, "userId")?;
    match operation {
        "trailersCatalog" => api.get(
            &["Items"],
            vec![
                ("UserId", user.into()),
                ("Recursive", "true".into()),
                ("IncludeItemTypes", "Movie,Series".into()),
                ("HasTrailer", "true".into()),
                ("Fields", "Overview,RemoteTrailers,LocalTrailerCount".into()),
                ("SortBy", "SortName".into()),
                (
                    "StartIndex",
                    count(api.v, "startIndex", 0, 1000000)?.to_string(),
                ),
                ("Limit", count(api.v, "limit", 30, 60)?.to_string()),
            ],
        ),
        "trailersDetail" => {
            let id = text(api.v, "itemId")?;
            let item = api.get(
                &["Items", id],
                vec![
                    ("UserId", user.into()),
                    (
                        "Fields",
                        "Overview,RemoteTrailers,ExternalUrls,LocalTrailerCount".into(),
                    ),
                ],
            )?;
            if !item["Id"]
                .as_str()
                .is_some_and(|owner| canonical_media_id(owner) == canonical_media_id(id))
            {
                return Err(err("Trailer owner mismatch"));
            }
            // Failure is surfaced, never silently reclassified as an empty local list.
            let local = api.get(
                &["Items", id, "LocalTrailers"],
                vec![("UserId", user.into())],
            )?;
            validate_detail(item, local)
        }
        _ => Err(err("Unknown trailers operation")),
    }
}

fn canonical_media_id(value: &str) -> Cow<'_, str> {
    let guid = match value.len() {
        32 => value.bytes().all(|byte| byte.is_ascii_hexdigit()),
        36 => value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        }),
        _ => false,
    };
    if guid {
        Cow::Owned(value.replace('-', "").to_ascii_lowercase())
    } else {
        Cow::Borrowed(value)
    }
}

fn validate_detail(item: Value, local: Value) -> Result<Value> {
    let owner = item["Id"]
        .as_str()
        .map(canonical_media_id)
        .ok_or_else(|| err("Missing trailer owner"))?;
    let values = local
        .as_array()
        .ok_or_else(|| err("Invalid local trailers response"))?;
    let mut ids = std::collections::HashSet::new();
    for value in values {
        let id = value["Id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(canonical_media_id)
            .filter(|id| *id != owner)
            .ok_or_else(|| err("Invalid local trailer ID"))?;
        if !ids.insert(id) {
            return Err(err("Duplicate local trailer ID"));
        }
    }
    Ok(json!({"item":item,"localTrailers":local}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_owner_and_local_ids() {
        let detail = validate_detail(json!({"Id":"film-a","RemoteTrailers":[{"Url":"https://www.youtube.com/watch?v=synthetic01"}]}), json!([{"Id":"trailer-a"}])).unwrap();
        assert_eq!(detail["item"]["Id"], "film-a");
        assert_eq!(detail["localTrailers"][0]["Id"], "trailer-a");
    }
    #[test]
    fn rejects_ambiguous_local_responses() {
        for local in [
            json!({}),
            json!([{}]),
            json!([{"Id":"film"}]),
            json!([{"Id":"t"},{"Id":"t"}]),
        ] {
            assert!(validate_detail(json!({"Id":"film"}), local).is_err());
        }
    }
    #[test]
    fn compact_and_hyphenated_guids_preserve_exact_associations() {
        let owner = "a0b1c2d3-e4f5-6789-abcd-1234567890ef";
        let compact = "A0B1C2D3E4F56789ABCD1234567890EF";
        assert_eq!(canonical_media_id(owner), canonical_media_id(compact));
        let (mut request, server) = fixture(vec![
            ("200 OK", json!({"Id":owner})),
            ("200 OK", json!([{"Id":"original-trailer"}])),
        ]);
        request["itemId"] = json!(compact);
        let api = Api::new(&request).unwrap();
        let detail = call(&api, "trailersDetail").unwrap();
        assert_eq!(detail["localTrailers"][0]["Id"], "original-trailer");
        assert_eq!(server.join().unwrap().len(), 2);
        assert!(validate_detail(json!({"Id":owner}), json!([{"Id":compact}])).is_err());
        assert_ne!(canonical_media_id(owner), canonical_media_id("different"));
    }

    fn fixture(responses: Vec<(&str, Value)>) -> (Value, std::thread::JoinHandle<Vec<String>>) {
        use std::{io::Write, net::TcpListener, thread, time::Instant};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let responses: Vec<_> = responses
            .into_iter()
            .map(|(status, body)| (status.to_string(), body.to_string()))
            .collect();
        let task = thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    if let Ok((stream, _)) = listener.accept() {
                        break stream;
                    }
                    assert!(Instant::now() < deadline, "mock request timed out");
                    thread::sleep(Duration::from_millis(5));
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                    assert!(request.len() < 16384);
                }
                requests.push(String::from_utf8(request).unwrap());
                write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            requests
        });
        (
            json!({"server":format!("http://{address}/media"),"allowLocalHttp":true,
            "token":"synthetic-token","userId":"viewer","itemId":"film"}),
            task,
        )
    }
    #[test]
    fn catalog_scopes_user_paging_and_auth_header() {
        let (v, task) = fixture(vec![("200 OK", json!({"Items":[],"TotalRecordCount":0}))]);
        let api = Api::new(&v).unwrap();
        assert!(call(&api, "trailersCatalog").is_ok());
        let requests = task.join().unwrap();
        let request = &requests[0];
        let line = request.lines().next().unwrap();
        assert!(line.starts_with("GET /media/Items?"));
        for expected in [
            "UserId=viewer",
            "HasTrailer=true",
            "StartIndex=0",
            "Limit=30",
        ] {
            assert!(line.contains(expected));
        }
        assert!(!line.contains("synthetic-token"));
        assert!(request.contains("Token=\"synthetic-token\""));
    }
    #[test]
    fn detail_uses_exact_owner_route_and_preserves_extras() {
        let (v, task) = fixture(vec![
            ("200 OK", json!({"Id":"film"})),
            ("200 OK", json!([{"Id":"trailer"}])),
        ]);
        let result = call(&Api::new(&v).unwrap(), "trailersDetail").unwrap();
        assert_eq!(result["localTrailers"][0]["Id"], "trailer");
        let requests = task.join().unwrap();
        assert!(requests[0].starts_with("GET /media/Items/film?UserId=viewer"));
        assert!(requests[1].starts_with("GET /media/Items/film/LocalTrailers?UserId=viewer"));
    }
    #[test]
    fn forbidden_and_mismatched_owner_fail_closed() {
        for (status, body) in [
            ("403 Forbidden", json!({})),
            ("200 OK", json!({"Id":"another-film"})),
        ] {
            let (v, task) = fixture(vec![(status, body)]);
            assert!(call(&Api::new(&v).unwrap(), "trailersDetail").is_err());
            assert_eq!(task.join().unwrap().len(), 1);
        }
    }
}
