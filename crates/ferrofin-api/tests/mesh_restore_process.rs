//! Actual candidate-process acceptance over loopback HTTP and strict PQ iroh.
//!
//! The authenticator is software, and both peers run on the qualification host.
//! This is not browser secure-context, physical iPhone/TV, relay, or federation
//! acceptance. Account/grant/session rows are created only by the server APIs;
//! the driver's SQLite connections are read-only evidence checks.
//!
//! Intentionally ignored by routine suites. An admitted Forge invocation must
//! select this exact test with --ignored --exact and supply the candidate
//! executable plus its retained SHA-256; there is no fallback binary.

#![cfg(unix)]

use std::{
    io::Read as _,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use bluefin_mesh_connect::{
    ConnectInvite, Error as MeshError, ServerAddress,
    native::{DeviceIdentity, PqcConnection, PqcEndpoint},
    protocol::{Envelope, Request},
};
use reqwest::Client;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{
    Connection as _,
    sqlite::{SqliteConnectOptions, SqliteConnection},
};
use tokio::{io::AsyncBufReadExt as _, sync::mpsc};
use webauthn_authenticator_rs::{WebauthnAuthenticator, softpasskey::SoftPasskey};

const ORIGIN: &str = "https://bluefin-restore.test";
const ADMIN: &str = "restore-fixture-admin";
const PASSWORD: &str = "Synthetic restore fixture password";
const CLIENT: &str = r#"MediaBrowser Client="mesh-restore-acceptance", Device="fixture", DeviceId="restore-http-admin", Version="1""#;
const DEADLINE: Duration = Duration::from_secs(120);

fn candidate() -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os("BLUEFIN_TEST_SERVER_BINARY")
            .expect("the admitted job must supply BLUEFIN_TEST_SERVER_BINARY"),
    )
    .canonicalize()
    .expect("candidate executable");
    let expected = std::env::var("BLUEFIN_TEST_SERVER_SHA256")
        .expect("the admitted job must supply the retained BLUEFIN_TEST_SERVER_SHA256");
    assert!(
        expected.len() == 64
            && expected
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "candidate SHA-256 must be lowercase hexadecimal"
    );
    let mut file = std::fs::File::open(&path).unwrap();
    assert!(file.metadata().unwrap().is_file());
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    let actual = format!("{:x}", hash.finalize());
    assert_eq!(
        actual, expected,
        "the selected executable must match the qualified artifact"
    );
    eprintln!(
        "actual-binary mesh restore: executable={} sha256={actual}; loopback peers; software authenticator",
        path.display()
    );
    path
}

fn envelope(request: Request) -> Value {
    serde_json::to_value(Envelope::new(request)).unwrap()
}

async fn http(
    client: &Client,
    base: &str,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> (u16, Value) {
    let mut request = client
        .request(method.parse().unwrap(), format!("{base}{path}"))
        .header("Authorization", format!("{CLIENT}, Token=\"{token}\""))
        .header("Origin", ORIGIN);
    if !body.is_null() {
        request = request.json(&body);
    }
    let response = request.send().await.expect("candidate HTTP request");
    let status = response.status().as_u16();
    let bytes = response.bytes().await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn expect_http(
    client: &Client,
    base: &str,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
    expected: u16,
) -> Value {
    let (status, value) = http(client, base, method, path, token, body).await;
    assert_eq!(status, expected, "{method} {path}");
    value
}

struct Process {
    child: tokio::process::Child,
    output: tokio::task::JoinHandle<String>,
    ready: mpsc::UnboundedReceiver<()>,
    base: String,
    name: String,
    lifetimes: usize,
}

impl Process {
    async fn start(root: &Path) -> Self {
        let executable = candidate();
        for directory in ["data", "config", "cache", "web", "identity"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        std::fs::set_permissions(
            root.join("identity"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        std::fs::write(
            root.join("config/network.json"),
            br#"{"AutoDiscovery":false,"EnableRemoteAccess":true}"#,
        )
        .unwrap();
        // Keep the selected UDP port across server lifetimes. The real owner
        // retains its key under identity/, outside the database backup tree.
        let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = udp.local_addr().unwrap();
        let mesh_config = root.join("mesh.json");
        std::fs::write(
            &mesh_config,
            serde_json::to_vec(&json!({
                "secret_seed_file": root.join("identity/key"),
                "bind_address": address,
                "relay_urls": [],
                "allow_experimental_pqc": true
            }))
            .unwrap(),
        )
        .unwrap();
        let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = tcp.local_addr().unwrap().port();
        let name = format!("mesh-restore-{}", uuid::Uuid::new_v4().simple());
        let mut command = tokio::process::Command::new(executable);
        command
            .env_clear()
            .current_dir(root)
            .kill_on_drop(true)
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("TMPDIR", root)
            .env("FERROFIN_DATA_DIR", root.join("data"))
            .env("FERROFIN_CONFIG_DIR", root.join("config"))
            .env("FERROFIN_CACHE_DIR", root.join("cache"))
            .env("FERROFIN_WEB_DIR", root.join("web"))
            .env("FERROFIN_BIND_ADDR", "127.0.0.1")
            .env("FERROFIN_PORT", port.to_string())
            .env("FERROFIN_SERVER_NAME", &name)
            .env("FERROFIN_PUBLISHED_URL", ORIGIN)
            .env("FERROFIN_ADMIN_USER", ADMIN)
            .env("FERROFIN_ADMIN_PASSWORD", PASSWORD)
            .env("FERROFIN_LOG_FORMAT", "json")
            .env("FERROFIN_STUDIOS_REPO_URL", "http://127.0.0.1:9")
            .env("FERROFIN_MUSICBRAINZ_URL", "http://127.0.0.1:9")
            .env("BLUEFIN_MESH_CONNECT_CONFIG", mesh_config)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (fixture, setting) in [
            ("BLUEFIN_TEST_FFMPEG", "FERROFIN_FFMPEG_PATH"),
            ("BLUEFIN_TEST_FFPROBE", "FERROFIN_FFPROBE_PATH"),
        ] {
            if let Some(path) = std::env::var_os(fixture) {
                command.env(setting, path);
            }
        }
        drop((tcp, udp));
        let mut child = command
            .spawn()
            .expect("start the exact candidate executable");
        let mut lines = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
        let (ready, receive) = mpsc::unbounded_channel();
        let output = tokio::spawn(async move {
            let mut log = String::new();
            while let Some(line) = lines.next_line().await.unwrap() {
                if line.contains("ferrofin-server ready") {
                    let _ = ready.send(());
                }
                if log.len() < 1024 * 1024 {
                    log.push_str(&line);
                    log.push('\n');
                }
            }
            log
        });
        let mut process = Self {
            child,
            output,
            ready: receive,
            base: format!("http://127.0.0.1:{port}"),
            name,
            lifetimes: 0,
        };
        process.next_lifetime().await;
        process
    }

    async fn next_lifetime(&mut self) {
        tokio::time::timeout(DEADLINE, self.ready.recv())
            .await
            .expect("candidate did not finish its next server lifetime")
            .expect("candidate exited before its next server lifetime");
        assert!(self.child.try_wait().unwrap().is_none());
        self.lifetimes += 1;
    }

    async fn stop(mut self, client: &Client, admin: &str) {
        expect_http(
            client,
            &self.base,
            "POST",
            "/System/Shutdown",
            admin,
            Value::Null,
            204,
        )
        .await;
        let status = tokio::time::timeout(DEADLINE, self.child.wait())
            .await
            .expect("candidate did not shut down")
            .unwrap();
        let log = self.output.await.unwrap();
        assert!(status.success(), "candidate process failed: {log}");
        assert_eq!(
            self.lifetimes, 3,
            "initial, restored, and ordinary-restart lifetimes"
        );
        assert!(
            log.contains("PQ mesh transport stopped"),
            "mesh shutdown receipt: {log}"
        );
    }
}

async fn issue(
    client: &Client,
    base: &str,
    admin: &str,
    intent: &str,
    user: Option<&str>,
) -> ConnectInvite {
    let value = expect_http(
        client,
        base,
        "POST",
        "/Bluefin/Accounts/Mesh/Invite",
        admin,
        json!({"intent":intent,"accountId":user,"lifetimeSeconds":600}),
        200,
    )
    .await;
    ConnectInvite::parse(value["code"].as_str().unwrap()).unwrap()
}

async fn rpc(endpoint: &PqcEndpoint, address: &ServerAddress, request: &Value) -> Value {
    let mut connection = endpoint.connect_server(address).await.unwrap();
    connection.send(request).await.unwrap();
    let response = connection.receive().await.unwrap();
    connection.close().await;
    response
}

struct Pairing {
    finish: Value,
    receipt: Value,
}

async fn pair(
    client: &Client,
    base: &str,
    endpoint: &PqcEndpoint,
    device: &DeviceIdentity,
    authenticator: &mut WebauthnAuthenticator<SoftPasskey>,
    invite: &ConnectInvite,
    request_tag: u8,
) -> Pairing {
    let mut connection = endpoint.connect(invite).await.unwrap();
    connection
        .send(&envelope(Request::Begin {
            request_id: [request_tag; 32],
            invitation: invite.clone(),
            device_public_key: device.public_key(),
            device_label: "Actual binary restore fixture".into(),
            account_name: None,
        }))
        .await
        .unwrap();
    let begin: Value = connection.receive().await.unwrap();
    assert_eq!(begin["body"]["type"], "begin_result");
    let browser = reqwest::Url::parse(
        begin["body"]["passkey_options"]["browserUrl"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(browser.origin().ascii_serialization(), ORIGIN);
    let (challenge, secret) = browser.fragment().unwrap().split_once('.').unwrap();
    let options = expect_http(
        client,
        base,
        "POST",
        "/Bluefin/Accounts/Mesh/Passkey/Options",
        "",
        json!({"challenge":challenge,"secret":secret}),
        200,
    )
    .await;
    // Exercise the real browser-proof adapter. This software authenticator
    // supplies a valid WebAuthn registration/assertion; it is not a browser.
    let public = json!({"publicKey":options["publicKey"]});
    let origin = reqwest::Url::parse(ORIGIN).unwrap();
    let credential = match options["kind"].as_str().unwrap() {
        "registration" => serde_json::to_value(
            authenticator
                .do_registration(origin, serde_json::from_value(public).unwrap())
                .unwrap(),
        )
        .unwrap(),
        "assertion" => serde_json::to_value(
            authenticator
                .do_authentication(origin, serde_json::from_value(public).unwrap())
                .unwrap(),
        )
        .unwrap(),
        _ => panic!("unknown passkey ceremony"),
    };
    expect_http(
        client,
        base,
        "POST",
        "/Bluefin/Accounts/Mesh/Passkey/Verify",
        "",
        json!({"challenge":challenge,"secret":secret,"credential":credential}),
        204,
    )
    .await;
    let finish = envelope(Request::Finish {
        request_id: [request_tag + 1; 32],
        challenge_id: begin["body"]["challenge_id"].as_str().unwrap().into(),
        passkey_response: json!({"browser":true}),
        device_signature: device
            .sign(
                begin["body"]["device_proof_message"]
                    .as_str()
                    .unwrap()
                    .as_bytes(),
            )
            .unwrap(),
    });
    connection.send(&finish).await.unwrap();
    let receipt: Value = connection.receive().await.unwrap();
    assert_eq!(receipt["body"]["type"], "finish_result");
    connection.close().await;
    Pairing { finish, receipt }
}

fn control(connection: &PqcConnection, session: &str) -> Value {
    json!({"version":1,"body":{
        "type":"control","session_id":session,"binding":connection.binding(),
        "request":{"method":"GET","path":"/Users/Me","headers":[],"body":[]}
    }})
}

async fn me(endpoint: &PqcEndpoint, address: &ServerAddress, session: &str) -> Value {
    let mut connection = endpoint.connect_server(address).await.unwrap();
    let request = control(&connection, session);
    connection.send(&request).await.unwrap();
    let head: Value = connection.receive().await.unwrap();
    assert_eq!(head["body"]["type"], "response_head");
    assert_eq!(head["body"]["status"], 200);
    let mut body = Vec::new();
    let mut ended = false;
    for _ in 0..128 {
        let frame: Value = connection.receive().await.unwrap();
        if frame["body"]["type"] == "response_end" {
            ended = true;
            break;
        }
        assert_eq!(frame["body"]["type"], "response_chunk");
        body.extend(serde_json::from_value::<Vec<u8>>(frame["body"]["bytes"].clone()).unwrap());
        assert!(body.len() <= 1024 * 1024);
    }
    assert!(ended, "bounded response must complete");
    connection.close().await;
    serde_json::from_slice(&body).unwrap()
}

async fn denied(endpoint: &PqcEndpoint, address: &ServerAddress, session: &str) {
    // Require an application reply after a successful fresh PQ handshake.
    // This differs from the committed revocation's [71; 32] request ID. A
    // restored live grant would be revoked by this probe and return success,
    // which must fail immediately rather than making later checks pass.
    let mut proof = endpoint.connect_server(address).await.unwrap();
    let request_id = [72u8; 32];
    let request = json!({"version":1,"body":{
        "type":"revoke_session","session_id":session,
        "request_id":request_id,"binding":proof.binding()
    }});
    proof.send(&request).await.unwrap();
    let rejected: Value = proof
        .receive()
        .await
        .expect("authorization evidence requires an explicit application reply");
    assert_eq!(
        rejected["body"]["type"], "error",
        "a revived grant must fail this probe"
    );
    assert_eq!(rejected["body"]["code"], "denied");
    proof.close().await;

    // The control route must also return no account data. Network includes
    // underlying IO errors, so this check alone is not authorization evidence.
    let mut connection = endpoint.connect_server(address).await.unwrap();
    let request = control(&connection, session);
    connection.send(&request).await.unwrap();
    assert!(
        matches!(
            connection.receive::<Value>().await,
            Err(MeshError::Closed | MeshError::Network)
        ),
        "old session returned control data or failed to complete its closure check"
    );
    connection.close().await;
}

async fn revoke(endpoint: &PqcEndpoint, address: &ServerAddress, session: &str) -> Value {
    let mut connection = endpoint.connect_server(address).await.unwrap();
    let request_id = [71u8; 32];
    let request = json!({"version":1,"body":{
        "type":"revoke_session","session_id":session,
        "request_id":request_id,"binding":connection.binding()
    }});
    connection.send(&request).await.unwrap();
    let response: Value = connection.receive().await.unwrap();
    assert_eq!(response["body"]["type"], "session_revoked");
    assert_eq!(response["body"]["session_id"], session);
    connection.close().await;
    response
}

async fn read_database(root: &Path) -> SqliteConnection {
    let _ = ferrofin_db::init_sqlite();
    SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(root.join("data/ferrofin.db"))
            .create_if_missing(false)
            .read_only(true),
    )
    .await
    .unwrap()
}

fn assert_rejected(response: &Value) {
    assert_eq!(response["body"]["type"], "error");
    assert!(matches!(
        response["body"]["code"].as_str(),
        Some("denied" | "used")
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires an admitted Forge job with an exact candidate executable and SHA-256"]
async fn actual_binary_restore_retires_old_mesh_authority() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .no_proxy()
        .build()
        .unwrap();
    let mut process = Process::start(root).await;
    let base = process.base.clone();
    let public = expect_http(
        &client,
        &base,
        "GET",
        "/System/Info/Public",
        "",
        Value::Null,
        200,
    )
    .await;
    assert_eq!(public["ServerName"], process.name);
    let authentication = expect_http(
        &client,
        &base,
        "POST",
        "/Users/AuthenticateByName",
        "",
        json!({"Username":ADMIN,"Pw":PASSWORD}),
        200,
    )
    .await;
    let admin = authentication["AccessToken"].as_str().unwrap();
    expect_http(
        &client,
        &base,
        "PUT",
        "/Bluefin/Accounts/Settings",
        admin,
        json!({"allowSignups":true,"meshSignupApproval":"manual"}),
        204,
    )
    .await;
    let invitation = issue(&client, &base, admin, "signup", None).await;
    let address = invitation.server_address();
    let device = DeviceIdentity::from_seed([61; 32]).unwrap();
    let endpoint = PqcEndpoint::client(&device).await.unwrap();
    let mut authenticator = WebauthnAuthenticator::new(SoftPasskey::new(true));
    let paired = pair(
        &client,
        &base,
        &endpoint,
        &device,
        &mut authenticator,
        &invitation,
        21,
    )
    .await;
    assert_eq!(paired.receipt["body"]["session"]["state"], "pending");
    let handle = paired.receipt["body"]["session"]["handle"]
        .as_str()
        .unwrap();
    expect_http(
        &client,
        &base,
        "POST",
        &format!("/Bluefin/Accounts/Mesh/Requests/{handle}/Approve"),
        admin,
        Value::Null,
        200,
    )
    .await;
    let approved = rpc(
        &endpoint,
        &address,
        &json!({"version":1,"body":{"type":"status","handle":handle}}),
    )
    .await;
    assert_eq!(approved["body"]["state"], "approved");
    let session = approved["body"]["session"]["id"].as_str().unwrap();
    let viewer = me(&endpoint, &address, session).await;
    let user = viewer["Id"].as_str().unwrap();
    assert!(viewer["Name"].as_str().unwrap().starts_with("bluefin-"));
    assert_eq!(
        rpc(&endpoint, &address, &paired.finish).await,
        paired.receipt,
        "exact Finish retry works before restore"
    );

    let mut database = read_database(root).await;
    let bearer: String = sqlx::query_scalar("SELECT AccessToken FROM Devices WHERE DeviceId=?")
        .bind(format!("bluefin-mesh:{session}"))
        .fetch_one(&mut database)
        .await
        .unwrap();
    database.close().await.unwrap();
    expect_http(
        &client,
        &base,
        "GET",
        "/Users/Me",
        &bearer,
        Value::Null,
        401,
    )
    .await;
    let unused = issue(&client, &base, admin, "login", Some(user)).await;
    let manifest = expect_http(
        &client,
        &base,
        "POST",
        "/Backup/Create",
        admin,
        json!({"Database":true}),
        200,
    )
    .await;
    let archive = Path::new(manifest["Path"].as_str().unwrap())
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    // A real post-snapshot database mutation proves the archive was applied;
    // an ordinary restart cannot satisfy the later missing-user assertion.
    let created = expect_http(
        &client,
        &base,
        "POST",
        "/Users/New",
        admin,
        json!({"Name":"post-backup-sentinel","Password":"Synthetic sentinel password"}),
        200,
    )
    .await;
    let sentinel = created["Id"].as_str().unwrap();
    let revoked = revoke(&endpoint, &address, session).await;
    assert_eq!(revoke(&endpoint, &address, session).await, revoked);
    denied(&endpoint, &address, session).await;

    expect_http(
        &client,
        &base,
        "POST",
        "/Backup/Restore",
        admin,
        json!({"ArchiveFileName":archive}),
        204,
    )
    .await;
    process.next_lifetime().await;
    let info = expect_http(
        &client,
        &base,
        "GET",
        "/System/Info",
        admin,
        Value::Null,
        200,
    )
    .await;
    assert_eq!(info["ServerName"], process.name);
    assert_eq!(info["HasPendingRestart"], false);
    expect_http(
        &client,
        &base,
        "GET",
        &format!("/Users/{sentinel}"),
        admin,
        Value::Null,
        404,
    )
    .await;
    let ordinary = expect_http(&client, &base, "GET", "/Users/Me", admin, Value::Null, 200).await;
    assert_eq!(ordinary["Name"], ADMIN, "unrelated HTTP session survives");

    // Authenticate to the restarted owner before testing denial: a dead
    // listener or failed handshake cannot masquerade as revoked authority.
    let login = issue(&client, &base, admin, "login", Some(user)).await;
    let current_address = login.server_address();
    assert_eq!(current_address.endpoint_id, address.endpoint_id);
    assert_eq!(
        current_address.server_certificate_sha256,
        address.server_certificate_sha256
    );
    denied(&endpoint, &current_address, session).await;
    assert_rejected(&rpc(&endpoint, &current_address, &paired.finish).await);
    assert_rejected(
        &rpc(
            &endpoint,
            &current_address,
            &envelope(Request::Begin {
                request_id: [81; 32],
                invitation: unused,
                device_public_key: device.public_key(),
                device_label: "Restored stale invitation".into(),
                account_name: None,
            }),
        )
        .await,
    );
    let mut database = read_database(root).await;
    let retained: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM Devices WHERE AccessToken=?")
        .bind(&bearer)
        .fetch_one(&mut database)
        .await
        .unwrap();
    assert_eq!(retained, 0, "restored internal bearer must be removed");
    let retained: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM BluefinMeshSessions WHERE Id=?")
        .bind(session)
        .fetch_one(&mut database)
        .await
        .unwrap();
    assert_eq!(retained, 0);
    database.close().await.unwrap();
    expect_http(
        &client,
        &base,
        "GET",
        "/Users/Me",
        &bearer,
        Value::Null,
        401,
    )
    .await;

    let fresh = pair(
        &client,
        &base,
        &endpoint,
        &device,
        &mut authenticator,
        &login,
        41,
    )
    .await;
    let fresh_session = fresh.receipt["body"]["session"]["id"].as_str().unwrap();
    assert_ne!(fresh_session, session);
    assert_eq!(
        me(&endpoint, &current_address, fresh_session).await["Id"],
        user
    );
    expect_http(
        &client,
        &base,
        "POST",
        "/System/Restart",
        admin,
        Value::Null,
        204,
    )
    .await;
    process.next_lifetime().await;
    assert_eq!(
        me(&endpoint, &current_address, fresh_session).await["Id"],
        user,
        "ordinary restart preserves the new device session and restored account"
    );
    denied(&endpoint, &current_address, session).await;
    endpoint.close().await;
    process.stop(&client, admin).await;
    eprintln!(
        "actual-binary mesh restore passed: HTTP backup/restore; owner-approved signup; real PQ/WebAuthn; \
         pre-revocation snapshot; old grant/Finish/invite/bearer refused; original HTTP token retained; \
         fresh passkey login and ordinary restart preserved"
    );
}
