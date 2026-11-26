//! Browser pairing and requests. Browser APIs are narrow platform bindings;
//! policy, retries, device identity and authorization framing remain in Rust.
use std::{cell::{Cell, RefCell}, collections::HashMap, rc::Rc, time::Duration};
use mesh_connect::{
    ConnectInvite, InviteIntent, ServerAddress,
    browser::{DeviceIdentity, PqcConnection, PqcEndpoint, RelayPolicy, now_ms},
    protocol::{Envelope, Request, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;
use wasm_bindgen::prelude::*;
use zeroize::Zeroize;
use crate::protocol::{self, Result, Session};

const HANDLE_PREFIX: &str = "bluefin-mesh-v1:";
const SERVICE: &str = "bluefin/connect/1";

#[wasm_bindgen(module = "/mesh-platform.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = loadMeshVault)]
    async fn load_vault() -> std::result::Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = saveMeshVault)]
    async fn save_vault(revision: f64, value: Option<String>) -> std::result::Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = takeMeshLaunch)]
    fn take_launch() -> String;
    #[wasm_bindgen(catch, js_name = readMeshPolicy)]
    async fn read_policy() -> std::result::Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = selectMeshStorage)]
    async fn select_storage(remember: bool) -> std::result::Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = meshPasskey)]
    async fn passkey(options: &str) -> std::result::Result<JsValue, JsValue>;
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy { version: u16, enabled: bool, relays: Vec<String> }
async fn policy() -> Result<RelayPolicy> {
    let value = read_policy().await.map_err(|_| "Browser mesh policy is unavailable.")?;
    let value: Policy = serde_json::from_str(&value.as_string().ok_or("Invalid browser mesh policy.")?)
        .map_err(|_| "Invalid browser mesh policy.")?;
    if value.version != 1 || !value.enabled {
        return Err("Secure browser pairing is not enabled for this installation.".into());
    }
    RelayPolicy::allowlist(value.relays)
        .map_err(|_| "No approved browser relay is configured.".into())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BeginReply {
    challenge: String,
    expires: u64,
    options: Value,
    proof: String,
    origin: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Descriptor {
    kind: String,
    id: String,
    user_id: String,
    username: String,
    expires_at_unix_ms: u64,
    server: ServerAddress,
    api_origin: String,
    authentication: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
enum Phase {
    Pairing {
        invite: ConnectInvite,
        begin_id: [u8; 32],
        finish_id: [u8; 32],
        account_name: Option<String>,
        reply: Option<BeginReply>,
        finish: Option<Value>,
    },
    Signup { server: ServerAddress, origin: String, handle: String },
    Connected { descriptor: Descriptor, logout_id: Option<[u8; 32]> },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved { version: u16, seed: [u8; 32], state: Phase }
impl Drop for Saved { fn drop(&mut self) { self.seed.zeroize(); } }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VaultReply { revision: u64, value: Option<String> }
struct Vault { revision: u64, saved: Option<Saved> }
impl Vault {
    async fn load() -> Result<Self> {
        let value = load_vault().await.map_err(|_| "Browser connection state is unavailable.")?;
        let record: VaultReply = serde_json::from_str(&value.as_string().ok_or("Invalid connection journal.")?)
            .map_err(|_| "Invalid connection journal.")?;
        if record.revision > 9_007_199_254_740_990 { return Err("Connection revision limit.".into()); }
        let saved: Option<Saved> = record.value.map(|text| serde_json::from_str(&text)
            .map_err(|_| "Invalid saved connection.")).transpose()?;
        if saved.as_ref().is_some_and(|saved| saved.version != 1 || saved.seed == [0; 32]) {
            return Err("Invalid saved connection.".into());
        }
        Ok(Self { revision: record.revision, saved })
    }
    async fn save(&mut self) -> Result<()> {
        let text = self.saved.as_ref().map(serde_json::to_string).transpose()
            .map_err(|_| "Connection could not be encoded.")?;
        save_vault(self.revision as f64, text).await
            .map_err(|_| "Connection changed in another tab or could not be saved. Resume the saved request.")?;
        self.revision += 1;
        Ok(())
    }
}

thread_local! {
    static BUSY: Cell<bool> = const { Cell::new(false) };
    static SESSIONS: RefCell<HashMap<String, Rc<Connected>>> = RefCell::new(HashMap::new());
}
struct Operation;
impl Operation {
    fn enter() -> Result<Self> {
        if BUSY.with(|busy| busy.replace(true)) {
            return Err("Another connection operation is still finishing.".into());
        }
        Ok(Self)
    }
}
impl Drop for Operation { fn drop(&mut self) { BUSY.with(|busy| busy.set(false)); } }

fn failure() -> String { "The pinned secure connection failed. Check the peer and approved relay.".into() }
fn random() -> Result<[u8; 32]> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| "Browser secure randomness is unavailable.")?;
    if bytes == [0; 32] { return Err("Browser secure randomness failed.".into()); }
    Ok(bytes)
}
fn valid_id(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn origin(value: &str) -> Result<Url> {
    let parsed = Url::parse(value).map_err(|_| "Invalid paired origin.")?;
    if parsed.scheme() != "https" || parsed.domain().is_none()
        || parsed.origin().ascii_serialization() != value {
        return Err("The paired service has no valid HTTPS passkey origin.".into());
    }
    Ok(parsed)
}
fn page_origin() -> Result<String> {
    web_sys::window().ok_or("Browser unavailable.")?.location().origin()
        .map_err(|_| "Browser origin unavailable.".into())
}

#[derive(Clone)]
pub struct PairingStep {
    pub external_url: Option<String>,
    pub awaiting_approval: bool,
    pub needs_begin_retry: bool,
}
pub enum Progress { Empty, Pairing(PairingStep), Connected(Session) }

fn step(saved: &Saved) -> Result<PairingStep> {
    match &saved.state {
        Phase::Pairing { reply: Some(reply), .. } => {
            let external_url = if reply.origin == page_origin()? { None } else {
                let raw = reply.options["browserUrl"].as_str().ok_or("The paired service has no passkey page.")?;
                let mut url = Url::parse(raw).map_err(|_| "Invalid passkey page.")?;
                let expected = origin(&reply.origin)?.join("/Bluefin/Accounts/Mesh/Passkey").map_err(|_| failure())?;
                let fragment = url.fragment().ok_or("Invalid passkey capability.")?.to_owned();
                let Some((challenge, secret)) = fragment.split_once('.') else { return Err("Invalid passkey capability.".into()); };
                if challenge != reply.challenge || !valid_id(secret) || url.query().is_some()
                    || url.origin() != expected.origin() || url.path() != expected.path()
                    || !url.username().is_empty() || url.password().is_some() {
                    return Err("Invalid passkey page.".into());
                }
                // The page stores the proof for the original mTLS device. It
                // does not send an account bearer through a callback.
                url.set_query(Some("client=web"));
                Some(url.to_string())
            };
            Ok(PairingStep { external_url, awaiting_approval: false, needs_begin_retry: false })
        }
        Phase::Pairing { reply: None, .. } => Ok(PairingStep {
            external_url: None, awaiting_approval: false, needs_begin_retry: true,
        }),
        Phase::Signup { .. } => Ok(PairingStep { external_url: None, awaiting_approval: true, needs_begin_retry: false }),
        _ => Err("The saved connection is already complete.".into()),
    }
}

pub fn launch_code() -> String { take_launch() }
pub fn image_code(bytes: &[u8]) -> Result<String> {
    let invite = mesh_connect::decode_qr_image_at(bytes, now_ms().map_err(|_| failure())?)
        .map_err(|_| "Choose an image containing one valid unexpired Bluefin connection code.")?;
    invite.require_service(SERVICE).map_err(|_| "This code is for another application.")?;
    invite.to_code().map_err(|_| "Invalid connection code.".into())
}

async fn exchange(endpoint: &PqcEndpoint, server: &ServerAddress, message: &Value) -> Result<Value> {
    let mut connection = endpoint.connect_server(server).await.map_err(|_| failure())?;
    let result = async {
        connection.send(message).await.map_err(|_| failure())?;
        connection.receive().await.map_err(|_| failure())
    }.await;
    connection.close().await;
    result
}
async fn endpoint(saved: &Saved, server: &ServerAddress) -> Result<PqcEndpoint> {
    let policy = policy().await?;
    policy.permits(server).map_err(|_| "This invitation has no browser route through an approved relay.")?;
    let device = DeviceIdentity::from_seed(saved.seed).map_err(|_| failure())?;
    PqcEndpoint::client_with_policy(&device, policy).await.map_err(|_| failure())
}

pub async fn begin(code: String, account_name: String, remember: bool) -> Result<Progress> {
    let _operation = Operation::enter()?;
    select_storage(remember).await.map_err(|_| "Finish or sign out of this tab's current connection before changing storage.")?;
    let invite = ConnectInvite::parse_at(&code, now_ms().map_err(|_| failure())?)
        .map_err(|_| "This connection code is invalid or expired.")?;
    invite.require_service(SERVICE).map_err(|_| "This invitation belongs to another application.")?;
    let account_name = if invite.intent == InviteIntent::Signup {
        let name = account_name.trim();
        if name.is_empty() || name.len() > 64 || name.chars().any(char::is_control) {
            return Err("Enter a username for this signup request.".into());
        }
        Some(name.to_owned())
    } else { None };
    // Check routing before a key or request is admitted, and before Begin can
    // permanently claim the invitation at its existing authority.
    policy().await?.permits(&invite.server_address())
        .map_err(|_| "This invitation has no browser route through an approved relay.")?;
    let mut vault = Vault::load().await?;
    if let Some(saved) = &vault.saved {
        if !matches!(&saved.state, Phase::Pairing { invite: old, account_name: name, .. }
            if old == &invite && name == &account_name) {
            return Err("Resume the saved connection or sign out before using another invitation.".into());
        }
    } else {
        let device = DeviceIdentity::generate().map_err(|_| failure())?;
        vault.saved = Some(Saved { version: 1, seed: *device.export_seed(),
            state: Phase::Pairing { invite, begin_id: random()?, finish_id: random()?,
                account_name, reply: None, finish: None } });
        vault.save().await?;
    }
    begin_saved(&mut vault).await?;
    Ok(Progress::Pairing(step(vault.saved.as_ref().ok_or("Missing saved request.")?)?))
}

async fn begin_saved(vault: &mut Vault) -> Result<()> {
    let saved = vault.saved.as_ref().ok_or("No saved request.")?.clone();
    let Phase::Pairing { invite, begin_id, account_name, reply, .. } = &saved.state else {
        return Err("No pending connection request.".into());
    };
    if reply.is_some() { return Ok(()); }
    invite.validate_at(now_ms().map_err(|_| failure())?).map_err(|_| "This invitation has expired.")?;
    let device = DeviceIdentity::from_seed(saved.seed).map_err(|_| failure())?;
    let request = Request::Begin { request_id: *begin_id, invitation: invite.clone(),
        device_public_key: device.public_key(), device_label: "Bluefin browser".into(), account_name: account_name.clone() };
    request.validate().map_err(|_| "Invalid saved request.")?;
    let ep = endpoint(&saved, &invite.server_address()).await?;
    let response = exchange(&ep, &invite.server_address(), &serde_json::to_value(Envelope::new(request))
        .map_err(|_| failure())?).await;
    ep.close().await;
    let response: Envelope<Response> = serde_json::from_value(response?).map_err(|_| failure())?;
    response.require_version().map_err(|_| failure())?;
    let Response::BeginResult { request_id, challenge_id, expires_at_unix_ms, passkey_options,
        device_proof_message } = response.body else { return Err("The invitation is unavailable. Resume the saved request or ask for a new code.".into()); };
    if request_id != *begin_id || !valid_id(&challenge_id)
        || expires_at_unix_ms <= now_ms().map_err(|_| failure())?
        || expires_at_unix_ms > invite.expires_at_unix_ms
        || !device_proof_message.starts_with("bluefin-mesh-device-v1:")
        || device_proof_message.len() > 1024 {
        return Err("The server returned an invalid connection challenge.".into());
    }
    let api_origin = passkey_options["origin"].as_str().ok_or("Missing passkey origin.")?.to_owned();
    origin(&api_origin)?;
    let reply = BeginReply { challenge: challenge_id, expires: expires_at_unix_ms,
        options: passkey_options, proof: device_proof_message, origin: api_origin };
    if let Some(Saved { state: Phase::Pairing { reply: stored, .. }, .. }) = &mut vault.saved { *stored = Some(reply); }
    vault.save().await
}

pub async fn resume() -> Result<Progress> {
    resume_with_storage(None).await
}
pub async fn resume_remembered() -> Result<Progress> {
    resume_with_storage(Some(true)).await
}
async fn resume_with_storage(remember: Option<bool>) -> Result<Progress> {
    let _operation = Operation::enter()?;
    if let Some(remember) = remember {
        select_storage(remember).await.map_err(|_| "Finish this tab's current connection before restoring a remembered device.")?;
    }
    let mut vault = Vault::load().await?;
    let Some(saved) = vault.saved.clone() else { return Ok(Progress::Empty); };
    match &saved.state {
        Phase::Connected { descriptor, logout_id } => {
            if logout_id.is_some() { revoke_saved(&mut vault).await?; Ok(Progress::Empty) }
            else { install(saved.clone(), descriptor.clone()).await.map(Progress::Connected) }
        }
        _ => Ok(Progress::Pairing(step(&saved)?)),
    }
}

pub async fn proceed() -> Result<Progress> {
    let _operation = Operation::enter()?;
    let mut vault = Vault::load().await?;
    let saved = vault.saved.as_ref().ok_or("No saved connection.")?.clone();
    match &saved.state {
        Phase::Pairing { reply: None, .. } => {
            begin_saved(&mut vault).await?;
            return Ok(Progress::Pairing(step(vault.saved.as_ref().ok_or("No saved connection.")?)?));
        }
        Phase::Signup { server, origin: expected_origin, handle } => {
            let ep = endpoint(&saved, server).await?;
            let value = exchange(&ep, server, &json!({"version":1,"body":{"type":"status","handle":handle}})).await;
            ep.close().await;
            let value = value?;
            if value["version"] != 1 || value["body"]["type"] != "status_result" || value["body"]["handle"] != *handle {
                return Err(failure());
            }
            match value["body"]["state"].as_str() {
                Some("pending") => return Ok(Progress::Pairing(step(&saved)?)),
                Some("approved") => return accept_descriptor(&mut vault, &saved, server, expected_origin, value["body"]["session"].clone()).await,
                _ => return Err("The administrator has not approved this signup request.".into()),
            }
        }
        Phase::Connected { .. } => return Err("This connection is already complete. Resume it.".into()),
        _ => {}
    }
    let Phase::Pairing { invite, finish_id, reply: Some(reply), finish, .. } = &saved.state else { return Err(failure()); };
    if finish.is_none() && reply.expires <= now_ms().map_err(|_| failure())? { return Err("The passkey request has expired.".into()); }
    let wire = if let Some(wire) = finish { wire.clone() } else {
        let response = if reply.origin == page_origin()? {
            let text = passkey(&reply.options.to_string()).await
                .map_err(|_| "Passkey canceled or unavailable. You can retry this saved request.")?;
            serde_json::from_str(&text.as_string().ok_or("Invalid passkey response.")?)
                .map_err(|_| "Invalid passkey response.")?
        } else { json!({"browser":true}) };
        let device = DeviceIdentity::from_seed(saved.seed).map_err(|_| failure())?;
        let request = Request::Finish { request_id: *finish_id, challenge_id: reply.challenge.clone(),
            passkey_response: response, device_signature: device.sign(reply.proof.as_bytes()).map_err(|_| failure())? };
        request.validate().map_err(|_| failure())?;
        let wire = serde_json::to_value(Envelope::new(request)).map_err(|_| failure())?;
        if let Some(Saved { state: Phase::Pairing { finish, .. }, .. }) = &mut vault.saved { *finish = Some(wire.clone()); }
        // Exact signed Finish is protected before transmission. A lost response
        // retries these bytes; it never regenerates an ML-DSA signature.
        vault.save().await?;
        wire
    };
    let ep = endpoint(&saved, &invite.server_address()).await?;
    let value = exchange(&ep, &invite.server_address(), &wire).await;
    ep.close().await;
    let response: Envelope<Response> = serde_json::from_value(value?).map_err(|_| failure())?;
    response.require_version().map_err(|_| failure())?;
    let Response::FinishResult { request_id, session } = response.body else {
        return Err("The saved connection has not completed. Finish the passkey page and retry.".into());
    };
    if request_id != *finish_id { return Err(failure()); }
    if session["kind"] == "signup" {
        let handle = session["handle"].as_str().filter(|value| valid_id(value))
            .ok_or("Invalid signup handle.")?.to_owned();
        if let Some(stored) = &mut vault.saved {
            stored.state = Phase::Signup { server: invite.server_address(), origin: reply.origin.clone(), handle };
        }
        vault.save().await?;
        return Ok(Progress::Pairing(step(vault.saved.as_ref().ok_or("Missing saved request.")?)?));
    }
    accept_descriptor(&mut vault, &saved, &invite.server_address(), &reply.origin, session).await
}

async fn accept_descriptor(vault: &mut Vault, saved: &Saved, expected: &ServerAddress,
    expected_origin: &str, value: Value) -> Result<Progress> {
    let descriptor: Descriptor = serde_json::from_value(value).map_err(|_| "Invalid device session.")?;
    validate_descriptor(&descriptor)?;
    if descriptor.server.endpoint_id != expected.endpoint_id
        || descriptor.server.server_certificate_sha256 != expected.server_certificate_sha256
        || descriptor.server.service != expected.service || descriptor.api_origin != expected_origin {
        return Err("The session belongs to another peer.".into());
    }
    let mut connected = saved.clone();
    connected.state = Phase::Connected { descriptor: descriptor.clone(), logout_id: None };
    vault.saved = Some(connected.clone());
    vault.save().await?;
    // The spent invitation and its secret are absent from the saved session.
    install(connected, descriptor).await.map(Progress::Connected)
}

fn validate_descriptor(value: &Descriptor) -> Result<()> {
    if value.kind != "session" || value.authentication != "mldsa65-device" || !valid_id(&value.id)
        || value.server.service != SERVICE || value.expires_at_unix_ms <= now_ms().map_err(|_| failure())? {
        return Err("The saved device session is invalid or expired.".into());
    }
    value.server.validate().map_err(|_| failure())?;
    origin(&value.api_origin)?;
    uuid::Uuid::parse_str(&value.user_id).map_err(|_| "Invalid account identity.")?;
    Ok(())
}

struct Connected {
    saved: Saved,
    descriptor: Descriptor,
    endpoint: PqcEndpoint,
    closing: Cell<bool>,
    active: Cell<usize>,
}
async fn install(saved: Saved, descriptor: Descriptor) -> Result<Session> {
    validate_descriptor(&descriptor)?;
    install_media_bridge();
    let endpoint = endpoint(&saved, &descriptor.server).await?;
    let token = format!("{HANDLE_PREFIX}{}", descriptor.id);
    let session = Session { base: origin(&descriptor.api_origin)?, token: token.clone(),
        user: uuid::Uuid::parse_str(&descriptor.user_id).map_err(|_| "Invalid account identity.")? };
    let connected = Rc::new(Connected { saved, descriptor, endpoint, closing: Cell::new(false), active: Cell::new(0) });
    let old = SESSIONS.with(|sessions| sessions.borrow_mut().insert(token, connected));
    if let Some(old) = old { old.endpoint.close().await; }
    Ok(session)
}
pub fn is_mesh(handle: &str) -> bool { handle.starts_with(HANDLE_PREFIX) }
pub(crate) fn restored_session(handle: &str) -> Result<Session> {
    let current = connected(handle)?;
    if current.closing.get() { return Err("This device is signing out.".into()); }
    Ok(Session { base: origin(&current.descriptor.api_origin)?, token: handle.to_owned(),
        user: uuid::Uuid::parse_str(&current.descriptor.user_id).map_err(|_| failure())? })
}
fn connected(handle: &str) -> Result<Rc<Connected>> {
    SESSIONS.with(|sessions| sessions.borrow().get(handle).cloned())
        .ok_or_else(|| "The device connection is not restored. Resume it before continuing.".into())
}
async fn retire_local(handle: &str, session: &Connected) {
    session.closing.set(true);
    stop_mesh_output(handle);
    SESSIONS.with(|sessions| sessions.borrow_mut().remove(handle));
    session.endpoint.close().await;
}
async fn ensure_current(session: &Connected) -> Result<()> {
    let current = Vault::load().await;
    let matches = current.as_ref().ok().and_then(|vault| vault.saved.as_ref())
        .is_some_and(|saved| matches!(&saved.state,
            Phase::Connected { descriptor, logout_id: None }
                if descriptor.id == session.descriptor.id && saved.seed == session.saved.seed));
    if !matches {
        let handle = format!("{HANDLE_PREFIX}{}", session.descriptor.id);
        retire_local(&handle, session).await;
        return Err("This browser connection changed or is signing out. Resume its saved state.".into());
    }
    Ok(())
}
struct Permit(Rc<Connected>);
impl Permit {
    fn acquire(session: Rc<Connected>) -> Result<Self> {
        if session.closing.get() || session.active.get() >= 6 { return Err("The device connection is busy or closing.".into()); }
        validate_descriptor(&session.descriptor)?;
        session.active.set(session.active.get() + 1);
        Ok(Self(session))
    }
}
impl Drop for Permit { fn drop(&mut self) { self.0.active.set(self.0.active.get().saturating_sub(1)); } }

pub async fn request(base: &Url, handle: &str, method: &str, path: &str,
    query: &[(&str, String)], body: Option<Value>) -> Result<Value> {
    if path == "Sessions/Logout" && method == "POST" { return logout(handle).await.map(|()| Value::Null); }
    let session = connected(handle)?;
    let _permit = Permit::acquire(session.clone())?;
    ensure_current(&session).await?;
    if base.origin().ascii_serialization() != session.descriptor.api_origin { return Err("Paired origin mismatch.".into()); }
    let url = protocol::api_url(base, path, query)?;
    let body = body.map(|value| serde_json::to_vec(&value).map_err(|_| failure())).transpose()?;
    if body.as_ref().is_some_and(|bytes| bytes.len() > 8192) { return Err("The mesh request exceeds its size limit.".into()); }
    let mut headers = vec![];
    if body.is_some() { headers.push(("content-type".to_owned(), "application/json".to_owned())); }
    let (status, _, bytes) = fetch(&session, method, &url, &headers, body.unwrap_or_default(), false, protocol::MAX_RESPONSE).await?;
    match status {
        401 | 403 => return Err("Account access denied. Sign in again.".into()),
        409 => return Err("The server state changed. Reload before retrying.".into()),
        code if !(200..300).contains(&code) => return Err(format!("The peer returned HTTP {code}.")),
        _ => {}
    }
    if bytes.is_empty() { Ok(Value::Null) }
    else { serde_json::from_slice(&bytes).map_err(|_| "Invalid peer response.".into()) }
}

/// Artwork uses the same device proof and peer pin; no bearer or image URL
/// leaves the Rust transport boundary.
pub(crate) async fn artwork(handle: &str, item: &str, backdrop: bool) -> Result<Vec<u8>> {
    let item = uuid::Uuid::parse_str(item).map_err(|_| "Invalid artwork item.")?;
    if item.is_nil() { return Err("Invalid artwork item.".into()); }
    let session = connected(handle)?;
    let _permit = Permit::acquire(session.clone())?;
    ensure_current(&session).await?;
    let base = origin(&session.descriptor.api_origin)?;
    let path = format!("Items/{}/Images/{}", item.simple(), if backdrop { "Backdrop" } else { "Primary" });
    let url = protocol::api_url(&base,&path,&[("maxWidth",if backdrop { "1200" } else { "400" }.into()),("quality","85".into())])?;
    let (status,headers,bytes) = fetch(&session,"GET",&url,&[],Vec::new(),true,2*1024*1024).await?;
    let allowed=headers.iter().any(|(name,value)| name.eq_ignore_ascii_case("content-type")
        && ["image/jpeg","image/png","image/webp"].contains(&value.split(';').next().unwrap_or("").trim()));
    if status != 200 || !allowed { return Err("Artwork is unavailable.".into()); }
    Ok(bytes)
}

async fn open_request(session: &Connected, method: &str, url: &Url,
    headers: &[(String,String)], body: Vec<u8>, media: bool) -> Result<(PqcConnection, u16, Vec<(String,String)>)> {
    if url.origin().ascii_serialization() != session.descriptor.api_origin
        || !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err("Resource is outside the paired peer.".into());
    }
    let mut path = url.path().to_owned();
    if let Some(query) = url.query() { path.push('?'); path.push_str(query); }
    if path.len() > 8192 { return Err("Resource path limit.".into()); }
    let mut connection = session.endpoint.connect_server(&session.descriptor.server).await.map_err(|_| failure())?;
    connection.send(&json!({"version":1,"body":{
        "type":if media { "media" } else { "control" }, "session_id":session.descriptor.id,
        "binding":connection.binding(), "request":{"method":method,"path":path,"headers":headers,"body":body}
    }})).await.map_err(|_| failure())?;
    let head: Value = connection.receive().await.map_err(|_| failure())?;
    if head["version"] != 1 || head["body"]["type"] != "response_head" { return Err(failure()); }
    let status = head["body"]["status"].as_u64().filter(|status| (100..=599).contains(status))
        .ok_or_else(failure)? as u16;
    let headers: Vec<(String,String)> = serde_json::from_value(head["body"]["headers"].clone()).map_err(|_| failure())?;
    if headers.len() > 64 || headers.iter().any(|(key,value)| key.len() > 128 || value.len() > 8192
        || key.chars().any(char::is_control) || value.chars().any(char::is_control)) {
        return Err("Invalid peer response headers.".into());
    }
    Ok((connection, status, headers))
}
async fn fetch(session: &Connected, method: &str, url: &Url, headers: &[(String,String)],
    body: Vec<u8>, media: bool, limit: usize) -> Result<(u16, Vec<(String,String)>, Vec<u8>)> {
    let (mut connection, status, headers) = open_request(session,method,url,headers,body,media).await?;
    let result = n0_future::time::timeout(Duration::from_secs(45), async {
        let mut bytes = Vec::new();
        loop {
            let frame: Value = connection.receive().await.map_err(|_| failure())?;
            if frame["version"] != 1 { return Err(failure()); }
            match frame["body"]["type"].as_str() {
                Some("response_end") => break,
                Some("response_chunk") => {
                    let chunk: Vec<u8> = serde_json::from_value(frame["body"]["bytes"].clone()).map_err(|_| failure())?;
                    if chunk.len() > 8192 || bytes.len().saturating_add(chunk.len()) > limit { return Err("Peer response limit.".into()); }
                    bytes.extend_from_slice(&chunk);
                }
                _ => return Err(failure()),
            }
        }
        Ok((status,headers,bytes))
    }).await.map_err(|_| "The peer response timed out.")?;
    connection.close().await;
    result
}

async fn revoke_identity(saved: &Saved, descriptor: &Descriptor, request_id: &[u8;32]) -> Result<()> {
    let endpoint = endpoint(saved, &descriptor.server).await?;
    let result: Result<()> = async {
        let mut connection = endpoint.connect_server(&descriptor.server).await.map_err(|_| failure())?;
        connection.send(&json!({"version":1,"body":{"type":"revoke_session","session_id":descriptor.id,
            "request_id":request_id,"binding":connection.binding()}})).await.map_err(|_| failure())?;
        let response: Value = connection.receive().await.map_err(|_| failure())?;
        connection.close().await;
        if response["version"] != 1 || response["body"]["type"] != "session_revoked"
            || response["body"]["session_id"] != descriptor.id || response["body"]["request_id"] != json!(request_id) {
            return Err("The peer has not acknowledged sign-out.".into());
        }
        Ok(())
    }.await;
    endpoint.close().await;
    result
}
async fn revoke_saved(vault: &mut Vault) -> Result<()> {
    let saved = vault.saved.as_ref().ok_or("No saved connection.")?.clone();
    let Phase::Connected { descriptor, logout_id: Some(request_id) } = &saved.state else {
        return Err("No saved sign-out request.".into());
    };
    revoke_identity(&saved,descriptor,request_id).await?;
    vault.saved = None;
    vault.save().await
}

/// Page retirement fences every request synchronously. Browsers can suspend a
/// page before asynchronous work finishes, so this never claims remote revoke.
pub(crate) fn retire_page() {
    let remembered = mesh_remembered();
    let sessions: Vec<_> = SESSIONS.with(|sessions| sessions.borrow_mut().drain().collect());
    for (handle,session) in sessions {
        session.closing.set(true);
        stop_mesh_output(&handle);
        wasm_bindgen_futures::spawn_local(async move {
            session.endpoint.close().await;
            if !remembered {
                if let Ok(request_id) = random() {
                    let _ = revoke_identity(&session.saved,&session.descriptor,&request_id).await;
                }
            }
        });
    }
    clear_ephemeral_vault();
}

pub async fn logout(handle: &str) -> Result<()> {
    let session = connected(handle)?;
    // Stop local output and network resources before any fallible storage or
    // remote revocation step. Failure below retains an honest unknown outcome.
    notify_mesh_retiring(handle);
    session.closing.set(true);
    SESSIONS.with(|sessions| sessions.borrow_mut().remove(handle));
    session.endpoint.close().await;
    let _operation = Operation::enter()?;
    let mut vault = Vault::load().await?;
    let Some(Saved { state: Phase::Connected { descriptor, logout_id }, .. }) = &mut vault.saved else {
        return Err("The saved session changed before sign-out.".into());
    };
    if descriptor.id != session.descriptor.id { return Err("The saved account changed before sign-out.".into()); }
    if logout_id.is_none() { *logout_id = Some(random()?); vault.save().await?; }
    revoke_saved(&mut vault).await
}


/// One media response. The browser pulls one bounded chunk at a time. The
/// service worker owns no device seed, account bearer or cryptographic handle.
#[wasm_bindgen]
pub struct MeshMediaStream { inner: Rc<MediaStream> }
struct MediaStream {
    session: Rc<Connected>,
    _permit: Permit,
    connection: RefCell<Option<PqcConnection>>,
    cancel: mesh_connect::browser::CancelHandle,
    head: String,
    closed: Cell<bool>,
}
#[wasm_bindgen]
impl MeshMediaStream {
    pub fn head(&self) -> String { self.inner.head.clone() }
    pub fn cancel(&self) {
        self.inner.closed.set(true);
        self.inner.cancel.cancel();
        self.inner.connection.borrow_mut().take();
    }
    pub async fn next(&self) -> std::result::Result<JsValue, JsValue> {
        let stream = self.inner.clone();
        if stream.closed.get() || stream.session.closing.get()
            || validate_descriptor(&stream.session.descriptor).is_err() {
            self.cancel();
            return Err(JsValue::from_str("Media connection closed"));
        }
        let mut connection = stream.connection.borrow_mut().take()
            .ok_or_else(|| JsValue::from_str("A media read is already active"))?;
        let result: Result<Option<Vec<u8>>> = async {
            let frame: Value = connection.receive().await.map_err(|_| failure())?;
            if frame["version"] != 1 { return Err(failure()); }
            match frame["body"]["type"].as_str() {
                Some("response_end") => Ok(None),
                Some("response_chunk") => {
                    let chunk: Vec<u8> = serde_json::from_value(frame["body"]["bytes"].clone()).map_err(|_| failure())?;
                    if chunk.is_empty() || chunk.len() > 8192 { return Err("Invalid media chunk.".into()); }
                    Ok(Some(chunk))
                }
                _ => Err(failure()),
            }
        }.await;
        if stream.closed.get() || stream.session.closing.get() {
            connection.close().await;
            return Err(JsValue::from_str("Media connection canceled"));
        }
        match result {
            Ok(Some(chunk)) => {
                *stream.connection.borrow_mut() = Some(connection);
                Ok(js_sys::Uint8Array::from(chunk.as_slice()).into())
            }
            Ok(None) => {
                stream.closed.set(true);
                connection.close().await;
                Ok(JsValue::NULL)
            }
            Err(_) => {
                stream.closed.set(true);
                connection.close().await;
                Err(JsValue::from_str("Media connection interrupted"))
            }
        }
    }
}
impl Drop for MediaStream {
    fn drop(&mut self) { self.cancel.cancel(); }
}

#[wasm_bindgen(module = "/mesh-platform.js")]
extern "C" {
    #[wasm_bindgen(js_name = notifyMeshRetiring)]
    fn notify_mesh_retiring(handle: &str);
    #[wasm_bindgen(js_name = meshRemembered)]
    fn mesh_remembered() -> bool;
    #[wasm_bindgen(js_name = clearEphemeralMeshVault)]
    fn clear_ephemeral_vault();
    #[wasm_bindgen(js_name = stopMeshOutput)]
    fn stop_mesh_output(handle: &str);
    #[wasm_bindgen(js_name = setMeshLifecycleHandler)]
    fn set_lifecycle_handler(handler: &js_sys::Function);
    #[wasm_bindgen(js_name = setMeshMediaHandler)]
    fn set_media_handler(handler: &js_sys::Function);
}
thread_local! { static MEDIA_INSTALLED: Cell<bool> = const { Cell::new(false) }; }
fn install_media_bridge() {
    if MEDIA_INSTALLED.with(|installed| installed.replace(true)) { return; }
    let callback = Closure::<dyn Fn(String, String, String, String) -> js_sys::Promise>::new(
        |handle, method, url, range| {
            wasm_bindgen_futures::future_to_promise(async move {
                open_media(handle,method,url,range).await.map(JsValue::from)
                    .map_err(|_| JsValue::from_str("The paired media request was refused"))
            })
        });
    use wasm_bindgen::JsCast;
    set_media_handler(callback.as_ref().unchecked_ref());
    let lifecycle = Closure::<dyn Fn(String) -> js_sys::Promise>::new(|handle| {
        wasm_bindgen_futures::future_to_promise(async move {
            let sessions: Vec<_> = SESSIONS.with(|sessions| sessions.borrow().values().cloned().collect());
            for session in sessions {
                let candidate = format!("{HANDLE_PREFIX}{}", session.descriptor.id);
                if handle == candidate || (handle.is_empty() && ensure_current(&session).await.is_err()) {
                    retire_local(&candidate, &session).await;
                }
            }
            Ok(JsValue::UNDEFINED)
        })
    });
    set_lifecycle_handler(lifecycle.as_ref().unchecked_ref());
    lifecycle.forget();
    // One callback belongs to this document, which also owns its Wasm instance.
    callback.forget();
}
async fn open_media(handle: String, method: String, raw_url: String, range: String) -> Result<MeshMediaStream> {
    if !matches!(method.as_str(), "GET" | "HEAD") || raw_url.len() > 8192 || range.len() > 128
        || range.chars().any(char::is_control) {
        return Err("Invalid media request.".into());
    }
    let url = Url::parse(&raw_url).map_err(|_| "Invalid media URL.")?;
    let path = url.path().to_ascii_lowercase();
    if !["/videos/", "/audio/", "/items/", "/livetv/"].iter().any(|prefix| path.starts_with(prefix))
        || url.query_pairs().any(|(name, _)| protocol::credential(&name)) {
        return Err("Unsupported paired media route.".into());
    }
    let session = connected(&handle)?;
    let permit = Permit::acquire(session.clone())?;
    ensure_current(&session).await?;
    let headers = if range.is_empty() { vec![] } else { vec![("range".into(),range)] };
    let (connection,status,headers) = open_request(&session,&method,&url,&headers,vec![],true).await?;
    let cancel = connection.cancel_handle();
    let head = json!({"status":status,"headers":headers}).to_string();
    Ok(MeshMediaStream { inner: Rc::new(MediaStream { session, _permit: permit,
        connection: RefCell::new(Some(connection)), cancel, head, closed: Cell::new(false) }) })
}


/// A code for another device of the current account. This is deliberately not
/// labelled as granting another person access to the account or its resources.
pub struct IssuedInvite {
    pub code: String,
    pub link: String,
    pub image: String,
    pub handle: String,
    pub expires: u64,
}
impl IssuedInvite {
    pub fn from_response(value: Value) -> Result<Self> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let raw = value["code"].as_str().ok_or("Invalid invitation response.")?;
        let invite = ConnectInvite::parse_at(raw, now_ms().map_err(|_| failure())?)
            .map_err(|_| "The returned invitation is invalid or expired.")?;
        if invite.service != SERVICE || invite.intent != InviteIntent::Login
            || value["singleUse"] != true || value["intent"] != "login" {
            return Err("The returned invitation has the wrong purpose.".into());
        }
        let handle = value["handle"].as_str().filter(|handle| valid_id(handle))
            .ok_or("Invalid invitation reference.")?.to_owned();
        let mut page = Url::parse(&web_sys::window().ok_or("Browser unavailable.")?.location().href()
            .map_err(|_| "Browser URL unavailable.")?).map_err(|_| "Invalid browser URL.")?;
        page.set_query(None);
        page.set_fragment(None);
        let base = mesh_connect::launch::WebLaunchBase::parse(page.as_str())
            .map_err(|_| "Open this application over trusted HTTPS before making a launch QR.")?;
        let link = base.invitation_url(&invite).map_err(|_| "The launch link is too large.")?;
        let png = mesh_connect::qr_png_for_web(&invite,&base,&mesh_connect::QrTheme::bluefin())
            .map_err(|_| "This invitation is too large for a browser launch QR. Copy its code instead.")?;
        Ok(Self { code: invite.to_code().map_err(|_| failure())?, link,
            image: format!("data:image/png;base64,{}", STANDARD.encode(png)),
            handle, expires: invite.expires_at_unix_ms })
    }
    pub fn current(&self) -> bool { now_ms().is_ok_and(|now| now < self.expires) }
}
#[wasm_bindgen(module = "/mesh-platform.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = copyMeshCode)]
    pub async fn copy_code(value: &str) -> std::result::Result<JsValue, JsValue>;
}
