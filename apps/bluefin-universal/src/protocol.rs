use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, String>;
pub const MAX_RESPONSE: usize = 4 * 1024 * 1024;

/// Credentials live only in memory and are never Debug/Serialize.
#[derive(Clone)]
pub struct Session {
    pub base: Url,
    pub token: String,
    pub user: Uuid,
}

pub fn server(input: &str) -> Result<Url> {
    let mut base = Url::parse(input).map_err(|_| "Enter an HTTPS server URL")?;
    if base.scheme() != "https"
        || base.host_str().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
    {
        return Err("Use HTTPS without credentials, query or fragment".into());
    }
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    Ok(base)
}

/// The hosting server publishes this bundle under its existing web mount.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn hosted_server(page: &str) -> Option<Url> {
    let page = Url::parse(page).ok()?;
    let path = page.path().strip_suffix("index.html").unwrap_or(page.path());
    let base_path = path.strip_suffix("/web/bluefin/").or_else(|| path.strip_suffix("/web/"))?;
    let mut base = page.join("/") .ok()?;
    base.set_path(&format!("{base_path}/"));
    server(base.as_str()).ok()
}

#[cfg(test)]
#[test]
fn hosted_web_preserves_the_server_base_path() {
    for (page, expected) in [
        ("https://media.example/web/bluefin/?presentation=canvas", "https://media.example/"),
        ("https://media.example/media/web/bluefin/index.html#account", "https://media.example/media/"),
        ("https://media.example/media/web/", "https://media.example/media/"),
    ] {
        assert_eq!(hosted_server(page).unwrap().as_str(), expected);
    }
    assert!(hosted_server("https://media.example/elsewhere/").is_none());
    assert!(hosted_server("http://media.example/web/bluefin/").is_none());
}

/// A per-process/browser-instance ID, never a fabricated hardware identity.
pub fn device_id() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        ID.get_or_init(|| Uuid::new_v4().to_string()).clone()
    }
    #[cfg(target_arch = "wasm32")]
    {
        std::thread_local! { static ID: String = Uuid::new_v4().to_string(); }
        ID.with(Clone::clone)
    }
}
pub fn authorization(token: &str) -> Result<String> {
    if token.starts_with("bluefin-mesh-v1:") {
        return Err("A device mesh connection cannot use an HTTP bearer fallback".into());
    }
    if token.len() > 4096 || token.bytes().any(|c| !(0x20..=0x7e).contains(&c)) {
        return Err("Invalid account token".into());
    }
    #[cfg(target_arch = "wasm32")]
    let device = "Browser".to_owned();
    #[cfg(not(target_arch = "wasm32"))]
    let device = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
    let mut auth = format!(
        "MediaBrowser Client=\"Bluefin Universal\", Device=\"{device}\", DeviceId=\"{}\", Version=\"{}\"",
        device_id(),
        env!("CARGO_PKG_VERSION")
    );
    if !token.is_empty() {
        let token = token.replace('\\', "\\\\").replace('"', "\\\"");
        auth.push_str(&format!(", Token=\"{token}\""));
    }
    Ok(auth)
}
pub fn credential(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "api_key"
            | "apikey"
            | "token"
            | "access_token"
            | "authorization"
            | "x-emby-token"
            | "x-emby-authorization"
    )
}
pub fn api_url(base: &Url, path: &str, query: &[(&str, String)]) -> Result<Url> {
    if path.starts_with('/')
        || path.contains("..")
        || path.contains([':', '?', '#', '\\', '%'])
        || query.iter().any(|(k, _)| credential(k))
    {
        return Err("Invalid API path or credential query".into());
    }
    let mut url = base.join(path).map_err(|_| "Invalid API path")?;
    if url.origin() != base.origin() || !url.path().starts_with(base.path()) {
        return Err("Invalid API origin".into());
    }
    if !query.is_empty() {
        url.query_pairs_mut()
            .extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
    }
    Ok(url)
}

pub fn client() -> reqwest::Client {
    let builder = reqwest::Client::builder();
    #[cfg(not(target_arch = "wasm32"))]
    let builder = builder
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(25));
    builder.build().expect("HTTP client")
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn request(
    base: &Url,
    token: &str,
    method: &str,
    path: &str,
    query: &[(&str, String)],
    body: Option<Value>,
) -> Result<Value> {
    let url = api_url(base, path, query)?;
    let mut request = client()
        .request(method.parse().map_err(|_| "Invalid method")?, url)
        .header("Authorization", authorization(token)?);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|_| "Connection failed. Check HTTPS, certificate and browser CORS policy.")?;
    match response.status().as_u16() {
        401 | 403 => return Err("Account access denied. Sign in again.".into()),
        409 => return Err("Queue changed on another client. Reload before saving.".into()),
        n if !(200..300).contains(&n) => return Err(format!("Server returned HTTP {n}")),
        _ => {}
    }
    use futures_util::StreamExt;
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "Response interrupted")?;
        if bytes.len() + chunk.len() > MAX_RESPONSE {
            return Err("Server response exceeds client limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        Ok(Value::Null)
    } else {
        serde_json::from_slice(&bytes).map_err(|_| "Invalid server response".into())
    }
}

pub async fn login(base: Url, name: String, password: String) -> Result<Session> {
    let value = request(
        &base,
        "",
        "POST",
        "Users/AuthenticateByName",
        &[],
        Some(json!({"Username":name,"Pw":password})),
    )
    .await?;
    session_from_reply(base, value)
}

fn session_from_reply(base: Url, value: Value) -> Result<Session> {
    let token = value["AccessToken"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 4096)
        .ok_or("Missing access token")?
        .to_owned();
    authorization(&token)?;
    let user = value["User"]["Id"]
        .as_str()
        .and_then(|v| Uuid::parse_str(v).ok())
        .filter(|id| !id.is_nil())
        .ok_or("Missing account id")?;
    Ok(Session { base, token, user })
}

#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountOptions {
    pub allow_signups: bool,
    pub quick_connect: bool,
    pub passkeys: bool,
}

pub struct ConnectedServer {
    pub base: Url,
    pub name: String,
    pub options: AccountOptions,
}

pub async fn connect(base: Url) -> Result<ConnectedServer> {
    let info = request(&base, "", "GET", "System/Info/Public", &[], None).await?;
    let name = info["ServerName"].as_str().ok_or("No media server at this address")?.to_owned();
    let options = match request(&base, "", "GET", "Bluefin/Accounts/Options", &[], None).await {
        Ok(reply) => serde_json::from_value(reply).map_err(|_| "Invalid account options")?,
        Err(_) => AccountOptions {
            quick_connect: request(&base, "", "GET", "QuickConnect/Enabled", &[], None).await
                .ok().and_then(|v| v.as_bool()).unwrap_or(false),
            ..Default::default()
        },
    };
    Ok(ConnectedServer { base, name, options })
}

pub async fn signup(base: Url, name: String, password: String) -> Result<Session> {
    request(&base, "", "POST", "Bluefin/Accounts/Create", &[],
        Some(json!({"username":name, "password":password}))).await?;
    login(base, name, password).await
}

#[derive(Clone)]
pub struct DeviceSignIn {
    pub base: Url,
    pub code: String,
    pub page: Option<Url>,
    secret: Value,
}

pub async fn begin_device_sign_in(base: Url, passkey: bool) -> Result<DeviceSignIn> {
    let method = if passkey { "Bluefin/Accounts/Passkey/Initiate" } else { "QuickConnect/Initiate" };
    let mut reply = request(&base, "", "POST", method, &[], Some(json!({}))).await?;
    let mut page = None;
    let (field, secret) = if passkey {
        let expected = api_url(&base, "Bluefin/Accounts/Passkey", &[])?;
        let url = Url::parse(reply["url"].as_str().ok_or("Invalid passkey link")?).map_err(|_| "Invalid passkey link")?;
        let mut without_ticket = url.clone();
        without_ticket.set_fragment(None);
        if expected != without_ticket || !url.fragment().is_some_and(|f| f.len() == 32 && f.bytes().all(|b| b.is_ascii_hexdigit())) {
            let _ = request(&base, "", "POST", "Bluefin/Accounts/Passkey/Cancel", &[], Some(reply["proof"].take())).await;
            return Err("Invalid passkey link".into());
        }
        page = Some(url);
        ("code", reply["proof"].take())
    } else {
        ("Code", reply["Secret"].take())
    };
    let code = reply[field].as_str().ok_or("Missing confirmation code")?.to_owned();
    Ok(DeviceSignIn { base, code, page, secret })
}

impl DeviceSignIn {
    pub async fn finish(&self) -> Result<Option<Session>> {
        let value = if self.page.is_some() {
            request(&self.base, "", "POST", "Bluefin/Accounts/Passkey/Exchange", &[], Some(self.secret.clone())).await?
        } else {
            let status = request(&self.base, "", "GET", "QuickConnect/Connect",
                &[("Secret", self.secret.as_str().ok_or("Invalid sign-in request")?.to_owned())], None).await?;
            if status["Authenticated"] != true { return Ok(None); }
            request(&self.base, "", "POST", "Users/AuthenticateWithQuickConnect", &[],
                Some(json!({"Secret": self.secret}))).await?
        };
        if value["pending"] == true { return Ok(None); }
        session_from_reply(self.base.clone(), value).map(Some)
    }
    pub async fn cancel(self) -> Result<()> {
        if self.page.is_some() {
            request(&self.base, "", "POST", "Bluefin/Accounts/Passkey/Cancel", &[], Some(self.secret)).await?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Item {
    pub id: Uuid,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "Type", default)]
    pub kind: String,
    #[serde(default)]
    pub overview: String,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub channel_name: Option<String>,
    #[serde(default)]
    pub channel_id: Option<Uuid>,
}
impl Item {
    pub fn playable(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "Movie" | "Episode" | "Audio" | "TvChannel"
        ) || (self.kind == "Program" && self.channel_id.is_some_and(|id| !id.is_nil()))
    }
}
pub fn items(value: Value) -> Result<Vec<Item>> {
    let rows = value.get("Items").cloned().unwrap_or(value);
    let items: Vec<Item> = serde_json::from_value(rows).map_err(|_| "Invalid item list")?;
    if items.len() > 200
        || items
            .iter()
            .any(|i| i.id.is_nil() || i.name.len() > 4096 || i.overview.len() > 65536)
    {
        return Err("Item list exceeds bounds".into());
    }
    Ok(items)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    #[test]
    fn real_http_sign_in_and_conflict() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let join = std::thread::spawn(move || {
            let mut request = server.recv().unwrap();
            assert_eq!(request.url(), "/Users/AuthenticateByName");
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&body).unwrap(),
                json!({"Username":"fixture","Pw":"original-fixture-password"})
            );
            request.respond(tiny_http::Response::from_string(r#"{"AccessToken":"fixture-session","User":{"Id":"00000000000000000000000000000001"}}"#)).unwrap();
            let request = server.recv().unwrap();
            assert!(
                request
                    .headers()
                    .iter()
                    .any(|h| h.field.equiv("Authorization")
                        && h.value.as_str().contains("Token=\"fixture-session\""))
            );
            request.respond(tiny_http::Response::empty(409)).unwrap();
        });
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = login(
                Url::parse(&format!("http://{address}/")).unwrap(),
                "fixture".into(),
                "original-fixture-password".into(),
            )
            .await
            .unwrap();
            assert_eq!(session.user, Uuid::from_u128(1));
            let err = request(
                &session.base,
                &session.token,
                "PUT",
                "Bluefin/Music/Queue",
                &[],
                Some(json!({})),
            )
            .await
            .unwrap_err();
            assert!(err.contains("Reload"));
        });
        join.join().unwrap();
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export async function boundedFetch(url, token, method, body, signal) {
  const controller=new AbortController(),abort=()=>controller.abort();
  signal.addEventListener('abort',abort,{once:true});
  if(signal.aborted)controller.abort();
  const timer=setTimeout(abort,25000);
  try {
    const headers={'Authorization':token};
    if(body!==null)headers['Content-Type']='application/json';
    const response=await fetch(url,{method,headers,body,signal:controller.signal,redirect:'error',credentials:'omit',cache:'no-store',referrerPolicy:'no-referrer'});
    if(!response.ok)return JSON.stringify({status:response.status,text:''});
    const reader=response.body?.getReader();const chunks=[];let total=0;
    if(reader){try{for(;;){const {done,value}=await reader.read();if(done)break;total+=value.length;if(total>4194304)throw Error('Response limit');chunks.push(value);}}catch(e){await reader.cancel();throw e;}}
    const bytes=new Uint8Array(total);let offset=0;for(const chunk of chunks){bytes.set(chunk,offset);offset+=chunk.length;}
    return JSON.stringify({status:response.status,text:new TextDecoder().decode(bytes)});
  } finally {clearTimeout(timer);signal.removeEventListener('abort',abort);}
}
"#)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(catch, js_name=boundedFetch)]
    async fn bounded_fetch(
        url: &str,
        token: &str,
        method: &str,
        body: Option<String>,
        signal: &web_sys::AbortSignal,
    ) -> std::result::Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>;
}
#[cfg(target_arch = "wasm32")]
pub async fn request(
    base: &Url,
    token: &str,
    method: &str,
    path: &str,
    query: &[(&str, String)],
    body: Option<Value>,
) -> Result<Value> {
    if crate::mesh::is_mesh(token) {
        return crate::mesh::request(base, token, method, path, query, body).await;
    }
    struct Guard(web_sys::AbortController);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let url = api_url(base, path, query)?;
    let auth = authorization(token)?;
    let guard =
        Guard(web_sys::AbortController::new().map_err(|_| "Browser cancellation unavailable")?);
    let value = bounded_fetch(
        url.as_str(),
        &auth,
        method,
        body.map(|v| v.to_string()),
        &guard.0.signal(),
    )
    .await
    .map_err(|_| "Connection failed. Check HTTPS, certificate, CORS or response limit.")?;
    let envelope: Value =
        serde_json::from_str(&value.as_string().ok_or("Invalid browser response")?)
            .map_err(|_| "Invalid browser response")?;
    match envelope["status"].as_u64().unwrap_or(0) {
        401 | 403 => return Err("Account access denied. Sign in again.".into()),
        409 => return Err("Queue changed on another client. Reload before saving.".into()),
        n if !(200..300).contains(&n) => return Err(format!("Server returned HTTP {n}")),
        _ => {}
    }
    let text = envelope["text"]
        .as_str()
        .ok_or("Invalid browser response")?;
    if text.is_empty() {
        Ok(Value::Null)
    } else {
        serde_json::from_str(text).map_err(|_| "Invalid server response".into())
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod standard_auth_regression {
    #[test]
    fn server_disables_legacy_auth() {
        let listener = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = url::Url::parse(&format!("http://{}/", listener.server_addr())).unwrap();
        let task = std::thread::spawn(move || {
            let request = listener.recv().unwrap();
            let standard = request.headers().iter().any(|h| {
                h.field.equiv("Authorization")
                    && h.value.as_str().starts_with("MediaBrowser Client=")
            });
            let legacy = request
                .headers()
                .iter()
                .any(|h| h.field.equiv("X-Emby-Token") || h.field.equiv("X-Emby-Authorization"));
            request
                .respond(tiny_http::Response::empty(if standard && !legacy {
                    204
                } else {
                    401
                }))
                .unwrap();
        });
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(super::request(
                &base,
                "synthetic-token",
                "GET",
                "Bluefin/Capabilities",
                &[],
                None,
            ));
        task.join().unwrap();
        assert!(
            result.is_ok(),
            "server with legacy auth disabled must accept client"
        );
    }
}

/// Original fixtures are served only from a task-owned loopback test server.
pub fn fixture_base() -> Result<Url> {
    #[cfg(not(target_arch = "wasm32"))]
    let input = std::env::var("BLUEFIN_FIXTURE_BASE")
        .unwrap_or_else(|_| "http://127.0.0.1:18461/fixture/".into());
    #[cfg(target_arch = "wasm32")]
    let input = web_sys::window()
        .ok_or("Missing browser")?
        .location()
        .origin()
        .map_err(|_| "Missing browser origin")?
        + "/fixture/";
    let base = Url::parse(&input).map_err(|_| "Invalid fixture URL")?;
    if !matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
        || base.path() != "/fixture/"
        || base.query().is_some()
        || base.fragment().is_some()
        || !base.username().is_empty()
        || base.password().is_some()
    {
        return Err("Fixtures require the task-owned loopback server".into());
    }
    Ok(base)
}
