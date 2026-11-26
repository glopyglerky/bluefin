//! Generic Bluefin endpoints only. Native caller owns the temporary directory;
//! synchronous completion precedes native cancellation cleanup.
use super::*;
use sha2::{Digest, Sha256};
use std::io::Write;
const MAX: u64 = 64 * 1024 * 1024;
fn media_source(v: &Value) -> Result<Option<&str>> {
    match v.get("mediaSourceId").filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(v) => v
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 1024 && !s.chars().any(char::is_control))
            .map(Some)
            .ok_or_else(|| err("Invalid karaoke source")),
    }
}
fn same_source(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let a = a.replace('-', "");
    let b = b.replace('-', "");
    a.len() == 32
        && b.len() == 32
        && a.bytes().chain(b.bytes()).all(|v| v.is_ascii_hexdigit())
        && a.eq_ignore_ascii_case(&b)
}
fn request_id(v: &Value) -> Result<&str> {
    let id = text(v, "requestId")?;
    if id.len() != 36
        || !id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
        || !id.bytes().any(|b| b != b'0' && b != b'-')
    {
        return Err(err("Invalid preparation identifier"));
    }
    Ok(id)
}
fn prepared_status(value: Value, api: &Api<'_>) -> Result<Value> {
    let expected_item = text(api.v, "itemId")?;
    let expected_job = request_id(api.v)?;
    let valid_item = value["itemId"].as_str().is_some_and(|v| {
        v.replace('-', "")
            .eq_ignore_ascii_case(&expected_item.replace('-', ""))
    });
    let valid_job = value["jobId"]
        .as_str()
        .is_some_and(|v| same_source(v, expected_job));
    if !valid_item
        || !valid_job
        || !matches!(
            value["state"].as_str(),
            Some(
                "queued"
                    | "preparing"
                    | "ready"
                    | "unavailable"
                    | "failed"
                    | "cancelling"
                    | "cancelled"
            )
        )
    {
        return Err(err("Invalid preparation response"));
    }
    let actual = media_source(&value)?;
    if media_source(api.v)?
        .zip(actual)
        .is_some_and(|(a, b)| !same_source(a, b))
        || (value["state"] == "ready" && actual.is_none())
        || value
            .get("retryAfterMilliseconds")
            .is_some_and(|v| v.as_u64().is_none_or(|n| !(500..=5000).contains(&n)))
    {
        return Err(err("Invalid preparation response"));
    }
    Ok(value)
}
pub(super) fn call(api: &Api<'_>, operation: &str) -> Result<Value> {
    let id = text(api.v, "itemId")?;
    match operation {
        "karaokeManifest" => {
            let query = media_source(api.v)?
                .map(|s| vec![("MediaSourceId", s.to_owned())])
                .unwrap_or_default();
            let value = api.get(&["Bluefin", "Karaoke", id], query)?;
            let processed = value["processing"] == "modelSeparatedStems";
            let valid_reference = value["processingReference"].as_str().is_some_and(|s| {
                !s.trim().is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
            });
            let actual = media_source(&value)?;
            if value["itemId"].as_str().is_none_or(|v| {
                !v.replace('-', "")
                    .eq_ignore_ascii_case(&id.replace('-', ""))
            }) || (!processed && value["processing"] != "suppliedStems")
                || (processed && (!valid_reference || actual.is_none()))
                || media_source(api.v)?
                    .zip(actual)
                    .is_some_and(|(a, b)| !same_source(a, b))
                || value["revision"]
                    .as_str()
                    .is_none_or(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err(err("Invalid karaoke package"));
            }
            Ok(value)
        }
        "prepareKaraoke" => {
            let mut body = json!({"requestId":request_id(api.v)?});
            if let Some(source) = media_source(api.v)? {
                body["mediaSourceId"] = json!(source);
            }
            prepared_status(
                api.post(&["Bluefin", "Karaoke", id, "Preparation"], body)?,
                api,
            )
        }
        "karaokePreparationStatus" => prepared_status(
            api.get(
                &["Bluefin", "Karaoke", id, "Preparation", request_id(api.v)?],
                vec![],
            )?,
            api,
        ),
        "cancelKaraokePreparation" => {
            api.cleanup_request(
                Method::DELETE,
                &["Bluefin", "Karaoke", id, "Preparation", request_id(api.v)?],
                vec![],
            )?;
            // HTTP acceptance is not proof the server's worker has joined.
            Ok(json!({"cancelRequested":true}))
        }
        "karaokeStem" => {
            let role = text(api.v, "role")?;
            let revision = text(api.v, "revision")?;
            let expected = text(api.v, "sha256")?;
            let size = api.v["bytes"]
                .as_u64()
                .filter(|n| *n > 44 && *n <= MAX)
                .ok_or_else(|| err("Invalid stem size"))?;
            if !matches!(role, "accompaniment" | "lead-vocal")
                || revision.len() != 64
                || expected.len() != 64
                || !revision
                    .bytes()
                    .chain(expected.bytes())
                    .all(|b| b.is_ascii_hexdigit())
            {
                return Err(err("Invalid stem selection"));
            }
            let destination = std::path::Path::new(text(api.v, "destination")?);
            if !destination.is_absolute() {
                return Err(err("Invalid temporary destination"));
            }
            let mut response = api
                .client
                .get(api.url(&["Bluefin", "Karaoke", id, revision, "Stems", role])?)
                .query(
                    &media_source(api.v)?
                        .map(|s| vec![("MediaSourceId", s)])
                        .unwrap_or_default(),
                )
                .header("Authorization", api.auth(true)?)
                .timeout(Duration::from_secs(120))
                .send()
                .map_err(|_| err("Karaoke download failed"))?;
            if !response.status().is_success()
                || response.content_length().is_some_and(|n| n != size)
            {
                return Err(err("Karaoke stem unavailable"));
            }
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
            }
            let mut file = options
                .open(destination)
                .map_err(|_| err("Cannot create temporary stem"))?;
            let result = (|| {
                let mut hash = Sha256::new();
                let mut total = 0u64;
                let mut buffer = [0; 65536];
                loop {
                    let n = response
                        .read(&mut buffer)
                        .map_err(|_| err("Karaoke download failed"))?;
                    if n == 0 {
                        break;
                    }
                    total += n as u64;
                    if total > size {
                        return Err(err("Stem exceeds bound"));
                    }
                    hash.update(&buffer[..n]);
                    file.write_all(&buffer[..n])
                        .map_err(|_| err("Temporary stem write failed"))?;
                }
                if total != size || format!("{:x}", hash.finalize()) != expected {
                    return Err(err("Stem integrity mismatch"));
                }
                file.sync_all()
                    .map_err(|_| err("Temporary stem write failed"))?;
                Ok(json!({}))
            })();
            drop(file);
            if result.is_err() {
                let _ = std::fs::remove_file(destination);
            }
            result
        }
        _ => Err(err("Unknown karaoke operation")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(
        status: &str,
        body: Vec<u8>,
        extra: &str,
    ) -> (Value, std::thread::JoinHandle<String>) {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_owned();
        let extra = extra.to_owned();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut b = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut b).unwrap();
                request.push(b[0]);
                assert!(request.len() < 16384);
            }
            let headers = String::from_utf8(request.clone()).unwrap();
            let size = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            assert!(size <= 65536);
            let mut payload = vec![0; size];
            stream.read_exact(&mut payload).unwrap();
            request.extend(payload);
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
            String::from_utf8(request).unwrap()
        });
        (
            json!({"server":format!("http://{address}/media"),"allowLocalHttp":true,"token":"fixture-token","itemId":"song","revision":"a".repeat(64),"role":"accompaniment","sha256":format!("{:x}",Sha256::digest(body_placeholder())),"bytes":48}),
            thread,
        )
    }
    fn body_placeholder() -> Vec<u8> {
        vec![42; 48]
    }
    #[test]
    fn processed_manifest_binds_selected_source_and_processing_reference() {
        for (source, reference, accepted) in [
            ("lossless-source", "deezer/spleeter/2stems@v1.4.0", true),
            ("other-source", "deezer/spleeter/2stems@v1.4.0", false),
            ("lossless-source", "", false),
        ] {
            let body = json!({"itemId":"song","mediaSourceId":source,
                "processing":"modelSeparatedStems","processingReference":reference,"revision":"a".repeat(64)});
            let (mut v, thread) = fixture("200 OK", serde_json::to_vec(&body).unwrap(), "");
            v["mediaSourceId"] = json!("lossless-source");
            let result = call(&Api::new(&v).unwrap(), "karaokeManifest");
            let request = thread.join().unwrap();
            assert_eq!(
                result.is_ok(),
                accepted,
                "{source}, {reference}: {result:?}"
            );
            assert!(request.starts_with(
                "GET /media/Bluefin/Karaoke/song?MediaSourceId=lossless-source HTTP/1.1"
            ));
            assert!(request.contains("Token=\"fixture-token\""));
        }
    }
    #[test]
    fn preparation_http_contract_binds_job_source_and_method() {
        let request_id = "55555555-5555-4555-8555-555555555555";
        for (operation, method, state) in [
            ("prepareKaraoke", "POST", "queued"),
            ("karaokePreparationStatus", "GET", "ready"),
            ("cancelKaraokePreparation", "DELETE", "cancelling"),
        ] {
            let body = json!({"itemId":"song","jobId":request_id,"mediaSourceId":"lossless-source",
                "state":state,"retryAfterMilliseconds":1000});
            let (mut v, thread) = fixture("200 OK", serde_json::to_vec(&body).unwrap(), "");
            v["requestId"] = json!(request_id);
            v["mediaSourceId"] = json!("lossless-source");
            v["operation"] = json!(operation);
            let result = super::super::call(&v).unwrap();
            let request = thread.join().unwrap();
            let suffix = if method == "POST" {
                String::new()
            } else {
                format!("/{request_id}")
            };
            assert!(request.starts_with(&format!(
                "{method} /media/Bluefin/Karaoke/song/Preparation{suffix} HTTP/1.1"
            )));
            assert!(request.contains("Token=\"fixture-token\""));
            if method == "POST" {
                let sent: Value =
                    serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
                assert_eq!(
                    sent,
                    json!({"requestId":request_id,"mediaSourceId":"lossless-source"})
                );
            }
            if method == "DELETE" {
                assert_eq!(result, json!({"cancelRequested":true}));
            } else {
                assert_eq!(result["state"], state);
            }
        }
    }
    #[test]
    fn preparation_responses_never_adopt_another_job_or_source() {
        let request_id = "55555555-5555-4555-8555-555555555555";
        for (field, value) in [
            ("jobId", json!("66666666-6666-4666-8666-666666666666")),
            ("itemId", json!("other")),
            ("mediaSourceId", json!("other-source")),
            ("mediaSourceId", Value::Null),
            ("state", json!("inventedReady")),
            ("retryAfterMilliseconds", json!(0)),
        ] {
            let mut body = json!({"itemId":"song","jobId":request_id,"mediaSourceId":"lossless-source","state":"ready","retryAfterMilliseconds":1000});
            body[field] = value;
            let (mut v, thread) = fixture("200 OK", serde_json::to_vec(&body).unwrap(), "");
            v["requestId"] = json!(request_id);
            v["mediaSourceId"] = json!("lossless-source");
            assert!(
                call(&Api::new(&v).unwrap(), "karaokePreparationStatus").is_err(),
                "{field}"
            );
            thread.join().unwrap();
        }
    }
    #[test]
    fn authenticated_bounded_download_and_hash_cleanup() {
        for corrupt in [false, true] {
            let (mut v, thread) = fixture("200 OK", body_placeholder(), "");
            if corrupt {
                v["sha256"] = json!("0".repeat(64));
            }
            let mut random = [0u8; 16];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "bluefin-karaoke-test-{:x}",
                u128::from_le_bytes(random)
            ));
            v["destination"] = json!(path.to_str().unwrap());
            let result = call(&Api::new(&v).unwrap(), "karaokeStem");
            let request = thread.join().unwrap();
            assert!(request.starts_with(&format!(
                "GET /media/Bluefin/Karaoke/song/{}/Stems/accompaniment",
                "a".repeat(64)
            )));
            assert!(request.contains("Token=\"fixture-token\""));
            if corrupt {
                assert!(result.is_err());
                assert!(!path.exists());
            } else {
                result.unwrap();
                assert_eq!(std::fs::read(&path).unwrap(), body_placeholder());
                std::fs::remove_file(path).unwrap();
            }
        }
    }
    #[test]
    fn forbidden_redirect_and_owner_mismatch() {
        for (status, body, extra, op) in [
            ("403 Forbidden", b"{}".to_vec(), "", "karaokeManifest"),
            (
                "302 Found",
                b"{}".to_vec(),
                "Location: http://127.0.0.1:1/leak\r\n",
                "karaokeManifest",
            ),
            (
                "200 OK",
                br#"{"itemId":"other","processing":"suppliedStems","revision":"invalid"}"#.to_vec(),
                "",
                "karaokeManifest",
            ),
        ] {
            let (v, thread) = fixture(status, body, extra);
            assert!(call(&Api::new(&v).unwrap(), op).is_err());
            thread.join().unwrap();
        }
    }
}
