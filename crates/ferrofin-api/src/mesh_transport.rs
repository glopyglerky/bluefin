//! Bridge the existing API/router over a live device-authenticated PQ channel.
//! No JSON field or HTTP header can construct the private transport origin.
use std::{sync::Arc,time::Duration};
use axum::{Router,body::{Body,to_bytes},http::{Request,HeaderName,HeaderValue,Method,Uri,header}};
use bluefin_mesh_connect::native::{PqcConnection,PqcEndpoint};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value,json};
use tokio::{sync::{Semaphore,watch},task::JoinSet};
use tower::ServiceExt;
use crate::{ApiError,AppState,handlers::bluefin_mesh_accounts::Service};

const MAX_CONTROL_BYTES:usize=8*1024*1024;
const MAX_MANIFEST_BYTES:usize=1024*1024;
const CHUNK:usize=8192;
const MAX_CONNECTIONS:usize=64;

/// Only this module constructs it from a completed inner TLS connection.
/// Mesh is conservatively remote even when a current UDP hop happens to be LAN.
#[derive(Clone,Debug)]
pub(crate) struct MeshOrigin { remote_endpoint:String }
impl MeshOrigin {
    pub(crate) fn remote_endpoint(&self)->&str{&self.remote_endpoint}
}
/// Existing remote-IP rules cannot evaluate an opaque relayed origin, so a
/// configured IP filter fails closed. Account-specific remote access is also
/// checked by the account owner and ordinary API authorization.
pub(crate) fn require_network(state:&AppState)->Result<(),ApiError>{
    if let Some(network)=&state.network {
        let network=network.read().map_err(|_|unavailable())?;
        let policy=network.configuration();
        if !policy.enable_remote_access || policy.remote_ip_filter.iter().any(|v|!v.trim().is_empty()){
            return Err(unavailable());
        }
    }
    Ok(())
}
fn unavailable()->ApiError{ApiError::ServiceUnavailable("Mesh access is unavailable under the current network policy.".into())}
fn denied()->ApiError{ApiError::Forbidden("This operation is unavailable through this mesh connection.".into())}
fn malformed()->ApiError{ApiError::BadRequest("Invalid mesh request.".into())}
fn transport(_:bluefin_mesh_connect::Error)->ApiError{unavailable()}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlEnvelope {version:u16,body:Control}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Control {
    #[serde(rename="type")]
    kind:ControlKind,
    session_id:String,
    binding:[u8;32],
    request:HttpRequest,
}
#[derive(Clone,Copy,Deserialize,PartialEq,Eq)]
#[serde(rename_all="snake_case")]
enum ControlKind {Control,Media}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HttpRequest {
    method:String,path:String,
    #[serde(default)] headers:Vec<(String,String)>,
    #[serde(default)] body:Vec<u8>,
    #[serde(default)] json_body:Option<Value>,
}


#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeEnvelope { version: u16, body: RevokeSession }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeSession {
    #[serde(rename = "type")]
    _kind: RevokeKind,
    session_id: String,
    request_id: [u8; 32],
    binding: [u8; 32],
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum RevokeKind { RevokeSession }

/// Joined listener receipt, emitted only after endpoint and tasks stop.
#[derive(Debug)]
pub struct Receipt {pub joined_connections:u64,pub endpoint_closed:bool}

/// Runs the established account service and normal production router.
/// The composition root retains the identity file lease until this returns.
pub async fn run(endpoint:PqcEndpoint,state:AppState,app:Router,mut stop:watch::Receiver<bool>)->Result<Receipt,ApiError>{
    let slots=Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let mut jobs=JoinSet::new();let mut joined=0;
    loop{
        tokio::select!{
            biased;
            _=stop.changed()=>{break;}
            Some(_result)=jobs.join_next(),if !jobs.is_empty()=>{joined+=1;}
            accepted=endpoint.incoming()=>{
                let Ok(pending)=accepted else {if *stop.borrow(){break;}continue;};
                let Ok(permit)=slots.clone().try_acquire_owned()else{
                    pending.refuse();continue;
                };
                let state=state.clone();let app=app.clone();
                jobs.spawn(async move{
                    let _permit=permit;
                    let Ok(mut connection) = pending.authenticate().await else { return; };
                    // Bound each device connection lifetime independently from
                    // individual frame deadlines; a reconnect proves the key again.
                    let _=tokio::time::timeout(Duration::from_hours(6),serve_connection(&mut connection,state,app)).await;
                    connection.close().await;
                });
            }
        }
    }
    endpoint.close().await;
    let drain=async{while jobs.join_next().await.is_some(){joined+=1;}};
    if tokio::time::timeout(Duration::from_secs(5),drain).await.is_err(){
        jobs.abort_all();while jobs.join_next().await.is_some(){joined+=1;}
    }
    Ok(Receipt{joined_connections:joined,endpoint_closed:true})
}
async fn serve_connection(connection:&mut PqcConnection,state:AppState,app:Router)->Result<(),ApiError>{
    let service=state.mesh_accounts().cloned().ok_or_else(unavailable)?;
    let peer=connection.peer_device().ok_or_else(denied)?.public_key().to_vec();
    let origin=format!("iroh:{}",connection.remote_endpoint());
    for _ in 0..4096 {
        require_network(&state)?;
        let request:Value=connection.receive().await.map_err(transport)?;
        let kind=request.get("body").and_then(|v|v.get("type")).and_then(Value::as_str);
        if kind == Some("revoke_session") {
            let request: RevokeEnvelope = serde_json::from_value(request).map_err(|_| malformed())?;
            if request.version != 1 || request.body.binding != connection.binding() {
                return Err(denied());
            }
            let response = match service.revoke_session(
                &state, &peer, &request.body.session_id, request.body.request_id,
            ).await {
                Ok(response) => response,
                Err(error) => json!({"version":1,"body":{"type":"error",
                    "request_id":request.body.request_id,
                    "code":if matches!(error, ApiError::ServiceUnavailable(_)) {"retry_later"} else {"denied"}}}),
            };
            connection.send(&response).await.map_err(transport)?;
        } else if matches!(kind,Some("control"|"media")){
            let request:ControlEnvelope=serde_json::from_value(request).map_err(|_|malformed())?;
            if request.version!=1 || request.body.binding!=connection.binding(){return Err(denied());}
            forward(connection,&state,&service,&peer,&origin,&app,request.body).await?;
        }else{
            let request_id=request.get("body").and_then(|v|v.get("request_id")).cloned().unwrap_or(json!(vec![0u8;32]));
            let response=match service.handle(&state,&peer,request).await{
                Ok(value)=>value,
                Err(error)=>{
                    let code=match error{
                        ApiError::Conflict(_)=>"used",
                        ApiError::ServiceUnavailable(_)=>"retry_later",
                        ApiError::BadRequest(_)=>"invalid",
                        _=>"denied",
                    };
                    json!({"version":1,"body":{"type":"error","request_id":request_id,"code":code}})
                }
            };
            connection.send(&response).await.map_err(transport)?;
        }
    }
    Ok(())
}
fn credential_name(value:&str)->bool{
    let key:String=value.chars().filter(|c|c.is_ascii_alphanumeric()).flat_map(char::to_lowercase).collect();
    matches!(key.as_str(),"accesstoken"|"refreshtoken"|"apikey"|"apitoken"|"password"|"secret"|"deviceseed"|"authorization")
}

fn exact_binary_resource(segments:&[&str])->bool {
    fn digest(value:&str)->bool {
        value.len()==64 && value.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))
    }
    match segments {
        ["sports","api","v1","logos",asset] => digest(asset),
        ["bluefin","karaoke",item,revision,"stems",role] =>
            uuid::Uuid::parse_str(item).is_ok_and(|id|!id.is_nil())
                && digest(revision) && matches!(*role,"accompaniment"|"lead-vocal"),
        _ => false,
    }
}
fn validate_request(input:&HttpRequest,media:bool)->Result<(Method,Uri),ApiError>{
    if input.method.len()>12 || input.path.len()>8192 || input.headers.len()>32{return Err(malformed());}
    let method=Method::from_bytes(input.method.as_bytes()).map_err(|_|malformed())?;
    if !matches!(method,Method::GET|Method::HEAD|Method::POST|Method::PUT|Method::PATCH|Method::DELETE){return Err(denied());}
    let uri:Uri=input.path.parse().map_err(|_|malformed())?;
    if uri.scheme().is_some() || uri.authority().is_some() || !uri.path().starts_with('/')
        || uri.path().starts_with("//") || uri.path().contains(['\\','%'])
        || uri.path().split('/').any(|p|p=="."||p==".."){return Err(malformed());}
    let body_limit=if method==Method::PUT && uri.path()=="/sports/api/v1/preferences" { bluefin_contract::sports::MAX_BODY_BYTES } else {8192};
    if input.body.len()>body_limit{return Err(malformed());}
    if let Some(value)=&input.json_body {
        if method!=Method::PUT || uri.path()!="/sports/api/v1/preferences" || !input.body.is_empty()
            || serde_json::to_vec(value).map_err(|_|malformed())?.len()>body_limit {
            return Err(malformed());
        }
    }
    let lower=uri.path().to_ascii_lowercase();
    let segments:Vec<_>=lower.split('/').filter(|s|!s.is_empty()).collect();
    if segments.iter().any(|s|s.contains("authenticate")||s.contains("password")||s.contains("passkey")
        || matches!(*s,"quickconnect"|"apikeys"|"apikey"|"tokens"|"auth"))
        || lower.starts_with("/startup/") {
        return Err(denied());
    }
    if let Some(query)=uri.query(){
        if url::form_urlencoded::parse(query.as_bytes()).any(|(k,_)|credential_name(&k)){return Err(denied());}
    }
    if media && (!matches!(method,Method::GET|Method::HEAD)
        || !(matches!(segments.first().copied(),Some("videos"|"audio"|"items"|"livetv")) || exact_binary_resource(&segments))){
        return Err(denied());
    }
    Ok((method,uri))
}
async fn forward(connection:&mut PqcConnection,state:&AppState,service:&Service,peer:&[u8],origin:&str,
    app:&Router,control:Control)->Result<(),ApiError>{
    require_network(state)?;
    let authorization=service.authorization(state,peer,&control.session_id,origin).await?;
    let (method,uri)=validate_request(&control.request,control.kind==ControlKind::Media)?;
    let body=if let Some(value)=control.request.json_body {
        serde_json::to_vec(&value).map_err(|_|malformed())?
    }else{control.request.body};
    let mut request=Request::builder().method(method).uri(uri.clone()).body(Body::from(body)).map_err(|_|malformed())?;
    for (name,value) in control.request.headers{
        let lower=name.to_ascii_lowercase();
        if !matches!(lower.as_str(),"accept"|"content-type"|"range"|"if-range"|"if-none-match"|"if-modified-since"|"accept-language")
            && !(lower=="if-match" && request.method()==Method::PUT && uri.path()=="/sports/api/v1/preferences"){return Err(denied());}
        if value.len()>4096{return Err(malformed());}
        let name=HeaderName::from_bytes(name.as_bytes()).map_err(|_|malformed())?;
        let value=HeaderValue::from_str(&value).map_err(|_|malformed())?;
        request.headers_mut().append(name,value);
    }
    if !authorization.token.expose().bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)) {
        return Err(denied());
    }
    request.headers_mut().insert(header::AUTHORIZATION,
        HeaderValue::from_str(&format!("MediaBrowser Token=\"{}\", Client=\"Bluefin\", Version=\"1\", Device=\"Bluefin\", DeviceId=\"{}\"",
            authorization.token.expose(), authorization.device_id)).map_err(|_|denied())?);
    request.headers_mut().insert(header::HOST,HeaderValue::from_static("mesh-connect.invalid"));
    // Forwarded-* and claimed ConnectInfo are never imported from the client.
    request.extensions_mut().insert(MeshOrigin{remote_endpoint:origin.into()});
    request.extensions_mut().insert(crate::forwarded_headers::Transport::Tls);
    let response=app.clone().oneshot(request).await.unwrap_or_else(|never|match never{});
    let status=response.status().as_u16();
    if response.status().is_redirection(){return Err(denied());}
    let content_type=response.headers().get(header::CONTENT_TYPE).and_then(|v|v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
    let mut headers=public_headers(response.headers());
    if headers.iter().any(|(_, value)| value.contains(authorization.token.expose())) { return Err(denied()); }
    let manifest=uri.path().to_ascii_lowercase().ends_with(".m3u8")||content_type.contains("mpegurl");
    let buffered=control.kind==ControlKind::Control||manifest||content_type.contains("json");
    if buffered{
        let cap=if manifest{MAX_MANIFEST_BYTES}else{MAX_CONTROL_BYTES};
        let bytes=tokio::time::timeout(Duration::from_secs(30),to_bytes(response.into_body(),cap))
            .await.map_err(|_|unavailable())?.map_err(|_|unavailable())?;
        let bytes=if content_type.contains("json"){
            let mut value:Value=serde_json::from_slice(&bytes).map_err(|_|denied())?;
            sanitize_json(&mut value,authorization.token.expose())?;
            serde_json::to_vec(&value).map_err(|_|denied())?
        }else if manifest{
            sanitize_manifest(&bytes,authorization.token.expose())?
        }else{
            if contains(&bytes,authorization.token.expose().as_bytes()){return Err(denied());}
            bytes.to_vec()
        };
        headers.retain(|(k,_)|k!="content-length");
        headers.push(("content-length".into(),bytes.len().to_string()));
        connection.send(&json!({"version":1,"body":{"type":"response_head","status":status,"headers":headers}})).await.map_err(transport)?;
        for chunk in bytes.chunks(CHUNK){
            require_network(state)?;service.authorization(state,peer,&control.session_id,origin).await?;
            connection.send(&json!({"version":1,"body":{"type":"response_chunk","bytes":chunk}})).await.map_err(transport)?;
        }
    }else{
        connection.send(&json!({"version":1,"body":{"type":"response_head","status":status,"headers":headers}})).await.map_err(transport)?;
        let secret=authorization.token.expose().as_bytes();
        let mut pending=Vec::with_capacity(CHUNK+secret.len());
        let mut stream=response.into_body().into_data_stream();
        loop{
            let next=tokio::time::timeout(Duration::from_secs(30),stream.next()).await.map_err(|_|unavailable())?;
            let Some(data)=next else{break};
            let data=data.map_err(|_|unavailable())?;
            for piece in data.chunks(CHUNK){
                pending.extend_from_slice(piece);
                if contains(&pending,secret){return Err(denied());}
                let send_len=pending.len().saturating_sub(secret.len().saturating_sub(1));
                if send_len>0{
                    require_network(state)?;service.authorization(state,peer,&control.session_id,origin).await?;
                    connection.send(&json!({"version":1,"body":{"type":"response_chunk","bytes":&pending[..send_len]}})).await.map_err(transport)?;
                    pending.drain(..send_len);
                }
            }
        }
        if !pending.is_empty(){
            require_network(state)?;service.authorization(state,peer,&control.session_id,origin).await?;
            connection.send(&json!({"version":1,"body":{"type":"response_chunk","bytes":pending}})).await.map_err(transport)?;
        }
    }
    connection.send(&json!({"version":1,"body":{"type":"response_end"}})).await.map_err(transport)
}
fn public_headers(input:&axum::http::HeaderMap)->Vec<(String,String)>{
    input.iter().filter_map(|(k,v)|{
        matches!(k.as_str(),"content-type"|"content-length"|"content-range"|"accept-ranges"|"etag"|"last-modified"|"cache-control"|"expires"|"vary")
            .then(||v.to_str().ok().filter(|v|v.len()<=4096).map(|v|(k.to_string(),v.to_owned()))).flatten()
    }).take(32).collect()
}
fn contains(bytes:&[u8],needle:&[u8])->bool{!needle.is_empty()&&bytes.windows(needle.len()).any(|w|w==needle)}
fn sanitize_json(value:&mut Value,secret:&str)->Result<(),ApiError>{
    match value{
        Value::Object(map)=>for(k,v)in map{
            if credential_name(k){return Err(denied());}sanitize_json(v,secret)?;
        },
        Value::Array(items)=>for v in items{sanitize_json(v,secret)?;},
        Value::String(s)=>{if s.contains(secret){*s=clean_url(s,secret)?;}},
        _=>{}
    }
    Ok(())
}
fn clean_url(text: &str, secret: &str) -> Result<String, ApiError> {
    if !text.contains(secret) { return Ok(text.to_owned()); }
    // Preserve relative media paths: resolving against a synthetic origin would
    // change a playlist-relative segment into a root-relative request.
    let (without_fragment, fragment) = text.split_once('#').map_or((text, None), |(a, b)| (a, Some(b)));
    let (path, query) = without_fragment.split_once('?').ok_or_else(denied)?;
    let pairs: Vec<_> = url::form_urlencoded::parse(query.as_bytes())
        .filter(|(key, _)| !credential_name(key))
        .map(|(key, value)| (key.into_owned(), value.into_owned())).collect();
    let mut out = path.to_owned();
    if !pairs.is_empty() {
        out.push('?');
        out.push_str(&url::form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs).finish());
    }
    if let Some(fragment) = fragment { out.push('#'); out.push_str(fragment); }
    if out.contains(secret) { return Err(denied()); }
    Ok(out)
}
fn sanitize_manifest(bytes:&[u8],secret:&str)->Result<Vec<u8>,ApiError>{
    let text=std::str::from_utf8(bytes).map_err(|_|denied())?;
    let mut out=String::with_capacity(text.len());
    for line in text.lines(){
        let mut line=line.to_owned();
        if !line.starts_with('#'){line=clean_url(&line,secret)?;}
        else if let Some(start)=line.find("URI=\""){
            let start=start+5;let end=line[start..].find('"').ok_or_else(denied)?+start;
            let url=clean_url(&line[start..end],secret)?;line.replace_range(start..end,&url);
        }
        if line.contains(secret){return Err(denied());}
        out.push_str(&line);out.push('\n');
    }
    Ok(out.into_bytes())
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn claimed_http_origin_and_credential_routes_are_rejected(){
        for path in ["/Users/AuthenticateByName","/Auth/Keys","/QuickConnect/Connect","/u%73ers/Authenticate","//remote/path","https://remote/path","/Items?api_key=x"]{
            let request=HttpRequest{method:"GET".into(),path:path.into(),headers:vec![],body:vec![],json_body:None};
            assert!(validate_request(&request,false).is_err(),"{path}");
        }
    }
    #[test]
    fn playback_urls_are_credential_free_and_bearer_fields_fail_closed(){
        let mut value=json!({"DirectStreamUrl":"/Videos/item/stream?api_key=private-bearer&static=true"});
        sanitize_json(&mut value,"private-bearer").unwrap();
        assert_eq!(value["DirectStreamUrl"],"/Videos/item/stream?static=true");
        let mut bearer=json!({"AccessToken":"private-bearer"});
        assert!(sanitize_json(&mut bearer,"private-bearer").is_err());
        let playlist=b"#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key?api_key=private-bearer\"\nsegment.ts?api_key=private-bearer\n";
        let out=String::from_utf8(sanitize_manifest(playlist,"private-bearer").unwrap()).unwrap();
        assert!(!out.contains("private-bearer"));assert!(!out.contains("api_key"));
        assert!(out.contains("URI=\"key\"")); assert!(out.ends_with("segment.ts\n"));
    }
    #[test]
    fn binary_sing_and_sports_paths_remain_narrow_and_read_only() {
        let revision="a".repeat(64);
        for path in [format!("/sports/api/v1/logos/{revision}"),
            format!("/Bluefin/Karaoke/00000000000000000000000000000001/{revision}/Stems/accompaniment"),
            format!("/Bluefin/Karaoke/00000000000000000000000000000001/{revision}/Stems/lead-vocal")] {
            let mut request=HttpRequest{method:"GET".into(),path,headers:vec![],body:vec![],json_body:None};
            assert!(validate_request(&request,true).is_ok());
            request.method="POST".into();
            assert!(validate_request(&request,true).is_err());
        }
        for path in ["/sports/api/v1/preferences".to_owned(),"/sports/api/v1/logos/../secret".into(),
            "/sports/api/v1/logos/https://other.example/a".into(),
            format!("/Bluefin/Karaoke/00000000000000000000000000000000/{revision}/Stems/accompaniment"),
            format!("/Bluefin/Karaoke/00000000000000000000000000000001/{revision}/Stems/other"),
            format!("/Bluefin/Karaoke/00000000000000000000000000000001/{revision}/Preparation")] {
            let request=HttpRequest{method:"GET".into(),path,headers:vec![],body:vec![],json_body:None};
            assert!(validate_request(&request,true).is_err());
        }
    }
    #[test]
    fn large_preference_body_allowance_does_not_apply_to_other_routes_or_methods() {
        let mut request=HttpRequest{method:"PUT".into(),path:"/sports/api/v1/preferences".into(),
            headers:vec![],body:vec![b' ';60_000],json_body:None};
        assert!(validate_request(&request,false).is_ok());
        request.body.push(b' ');
        assert!(validate_request(&request,false).is_err());
        request.body.truncate(8193);
        request.method="POST".into();
        assert!(validate_request(&request,false).is_err());
        request.method="PUT".into();request.path="/Users/Me".into();
        assert!(validate_request(&request,false).is_err());
    }

    #[test]
    fn compact_json_body_is_only_admitted_for_account_sports_preferences() {
        let mut request=HttpRequest{method:"PUT".into(),path:"/sports/api/v1/preferences".into(),
            headers:vec![],body:vec![],json_body:Some(json!({"teams":["nhl:1"],"schema":1}))};
        assert!(validate_request(&request,false).is_ok());
        request.body=vec![b'x'];
        assert!(validate_request(&request,false).is_err());
        request.body.clear();request.path="/Users/Me".into();
        assert!(validate_request(&request,false).is_err());
        request.path="/sports/api/v1/preferences".into();request.method="POST".into();
        assert!(validate_request(&request,false).is_err());
        request.method="PUT".into();request.json_body=Some(json!({"value":"a".repeat(60_000)}));
        assert!(validate_request(&request,false).is_err());
    }

}
