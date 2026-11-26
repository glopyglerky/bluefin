//! Browser WebAuthn adapter for relying parties outside a native app's signed
//! associated domains. Browser success stores a proof; only the original mTLS
//! device can finish the invitation and obtain its device-bound descriptor.
use super::*;
use axum::{extract::DefaultBodyLimit,http::{header,StatusCode,request::Parts},response::{Html,IntoResponse,Response as HttpResponse}};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserProof {challenge:String,secret:String}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserFinish {challenge:String,secret:String,credential:Value}
fn browser_origin(service:&Service,parts:&Parts)->Result<(),ApiError>{
    if parts.headers.get(header::ORIGIN).and_then(|v|v.to_str().ok())!=Some(service.origin.as_str()){return Err(failed());}Ok(())
}
fn permits(pending:&Pending,secret:&str)->bool{
    if !id_valid(secret)||pending.expires<=now()||pending.browser_used{return false;}
    let candidate=digest(secret.as_bytes());
    candidate.iter().zip(pending.browser_secret_hash.iter()).fold(0u8,|diff,(a,b)|diff|(a^b))==0
}
async fn options(State(state):State<AppState>,parts:Parts,Json(input):Json<BrowserProof>)->Result<Json<Value>,ApiError>{
    let service=owner(&state)?;browser_origin(service,&parts)?;service.live().await?;
    if !id_valid(&input.challenge){return Err(failed());}
    let pending=service.pending.lock().await;
    let request=pending.get(&input.challenge).filter(|p|permits(p,&input.secret)).ok_or_else(failed)?;
    let options=&request.response["body"]["passkey_options"];
    Ok(Json(json!({"kind":options["kind"],"publicKey":options["publicKey"]})))
}
async fn verify(State(state):State<AppState>,parts:Parts,Json(input):Json<BrowserFinish>)->Result<StatusCode,ApiError>{
    let service=owner(&state)?;browser_origin(service,&parts)?;service.live().await?;
    if !id_valid(&input.challenge){return Err(failed());}
    let mut pending=service.pending.lock().await;
    let request=pending.get_mut(&input.challenge).filter(|p|permits(p,&input.secret)).ok_or_else(failed)?;
    match &request.ceremony {
        Ceremony::Login{state:ceremony,..}=>{
            let credential=serde_json::from_value(input.credential.clone()).map_err(|_|failed())?;
            service.webauthn.finish_passkey_authentication(&credential,ceremony).map_err(|_|failed())?;
        },
        Ceremony::Signup{state:ceremony,..}=>{
            let credential=serde_json::from_value(input.credential.clone()).map_err(|_|failed())?;
            service.webauthn.finish_passkey_registration(&credential,ceremony).map_err(|_|failed())?;
        },
    }
    request.browser_response=Some(input.credential);
    request.browser_used=true;
    Ok(StatusCode::NO_CONTENT)
}
async fn page()->HttpResponse{
    ([
        (header::CONTENT_SECURITY_POLICY,"default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"),
        (header::REFERRER_POLICY,"no-referrer"),
    ],Html(include_str!("bluefin_account_web/mesh-passkey.html"))).into_response()
}
async fn script()->impl IntoResponse{
    ([(header::CONTENT_TYPE,"text/javascript; charset=utf-8")],include_str!("bluefin_account_web/mesh-passkey.js"))
}
pub(super) fn routes()->Router<AppState>{
    Router::new().route("/Bluefin/Accounts/Mesh/Passkey",get(page))
        .route("/Bluefin/Accounts/Mesh/Passkey/client.js",get(script))
        .route("/Bluefin/Accounts/Mesh/Passkey/Options",post(options))
        .route("/Bluefin/Accounts/Mesh/Passkey/Verify",post(verify))
        .layer(DefaultBodyLimit::max(32768))
}
