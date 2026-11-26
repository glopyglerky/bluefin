//! Bluefin client protocol over pinned, mutually authenticated PQ TLS.
//! Native adapters keep the device seed in their device-only secure store.
//! This module never accepts a QR relay as permission to contact that relay.
use super::{err, text, Result, RESPONSE_MAX};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use bluefin_mesh_connect::{ServerAddress, protocol::{Envelope,Request,Response}};
use bluefin_mesh_connect::native::{DeviceIdentity,PqcEndpoint,RelayPolicy};
use serde_json::{json,Value};
use sha2::{Digest,Sha256};
use std::{sync::OnceLock,time::Duration};
use tokio::runtime::Runtime;
use url::Url;
static RUNTIME:OnceLock<std::result::Result<Runtime,String>>=OnceLock::new();
pub(super) fn runtime()->Result<&'static Runtime>{
    RUNTIME.get_or_init(||tokio::runtime::Builder::new_multi_thread().worker_threads(2)
        .thread_name("bluefin-mesh").enable_all().build().map_err(|_|err("The mesh runtime is unavailable.")))
        .as_ref().map_err(Clone::clone)
}
fn failure()->String{err("The secure Bluefin connection failed. Check that the server is reachable and the connection is still approved.")}
fn hex(bytes:&[u8])->String{
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len()*2),|mut out,b|{let _=write!(out,"{b:02x}");out})
}
fn bytes32(value:&str)->Result<[u8;32]>{
    if value.len()!=64 || !value.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)){
        return Err(err("Invalid saved connection identity."));
    }
    let mut out=[0;32];
    for (i,b) in out.iter_mut().enumerate(){*b=u8::from_str_radix(&value[i*2..i*2+2],16).map_err(|_|failure())?;}
    Ok(out)
}
fn random()->Result<[u8;32]>{
    let mut bytes=[0;32];getrandom::fill(&mut bytes).map_err(|_|failure())?;Ok(bytes)
}
pub(super) fn identity(v:&Value)->Result<DeviceIdentity>{
    DeviceIdentity::from_seed(bytes32(text(v,"deviceSeed")?)?).map_err(|_|failure())
}
pub(super) fn address(v:&Value)->Result<ServerAddress>{
    let address:ServerAddress=serde_json::from_value(v.clone()).map_err(|_|err("The saved server address is invalid."))?;
    address.validate().map_err(|_|failure())?;
    if address.service!="bluefin/connect/1"{return Err(err("This connection belongs to another application."));}
    Ok(address)
}
fn policy(v:&Value)->Result<RelayPolicy>{
    let urls=match v.get("relayAllowlist"){
        None|Some(Value::Null)=>Vec::new(),
        Some(Value::Array(values)) if values.len()<=16=>values.iter()
            .map(|v|v.as_str().map(str::to_owned).ok_or_else(failure)).collect::<Result<Vec<_>>>()?,
        _=>return Err(err("Invalid application relay policy.")),
    };
    RelayPolicy::allowlist(urls).map_err(|_|err("The configured relay policy is invalid."))
}
pub(super) async fn endpoint(v:&Value,identity:&DeviceIdentity)->Result<PqcEndpoint>{
    PqcEndpoint::client_with_policy(identity,policy(v)?).await.map_err(|_|failure())
}
async fn exchange(v:&Value,identity:&DeviceIdentity,server:&ServerAddress,message:&Value)->Result<Value>{
    let endpoint=endpoint(v,identity).await?;
    let result=tokio::time::timeout(Duration::from_secs(45),async{
        let mut connection=endpoint.connect_server(server).await.map_err(|_|failure())?;
        let result=async{
            connection.send(message).await.map_err(|_|failure())?;
            connection.receive::<Value>().await.map_err(|_|failure())
        }.await;
        connection.close().await;
        result
    }).await.map_err(|_|err("The server did not answer in time. You can retry this request."));
    endpoint.close().await;
    result?
}
pub(super) fn call(v:&Value,operation:&str)->Result<Value>{
    match operation{
        "meshRequestIdCreate"=>Ok(json!({"requestId":hex(&random()?)})),
        "meshRevoke"=>runtime()?.block_on(revoke(v)),
        "meshIdentityCreate"=>{
            let device=DeviceIdentity::generate().map_err(|_|failure())?;
            let seed=device.export_seed();
            let fingerprint: [u8;32]=Sha256::digest(device.public_key()).into();
            Ok(json!({"deviceSeed":hex(seed.as_ref()),"deviceId":format!("mesh-{}",hex(&fingerprint)),
                "beginRequestId":hex(&random()?),"finishRequestId":hex(&random()?)}))
        },
        "meshBegin"=>{
            let device=identity(v)?;
            let invite=super::mesh_onboarding::parse(text(v,"code")?)?;
            let server=invite.server_address();
            let request_id=bytes32(text(v,"requestId")?)?;
            let account_name=v.get("accountName").and_then(Value::as_str).filter(|s|!s.is_empty()).map(str::to_owned);
            let request=Request::Begin{request_id,invitation:invite,device_public_key:device.public_key(),
                device_label:text(v,"deviceLabel")?.into(),account_name};
            request.validate().map_err(|_|err("Invalid connection request."))?;
            let message=serde_json::to_value(Envelope::new(request)).map_err(|_|failure())?;
            let value=runtime()?.block_on(exchange(v,&device,&server,&message))?;
            let response:Envelope<Response>=serde_json::from_value(value).map_err(|_|failure())?;
            response.require_version().map_err(|_|failure())?;
            match response.body {
                Response::BeginResult{request_id:reply,challenge_id,expires_at_unix_ms,passkey_options,device_proof_message} if reply==request_id=>{
                    let origin=passkey_options["origin"].as_str().ok_or_else(failure)?.to_owned();
                    let parsed=Url::parse(&origin).map_err(|_|failure())?;
                    if parsed.scheme()!="https" || parsed.domain().is_none() || parsed.origin().ascii_serialization()!=origin {
                        return Err(err("The server has no valid passkey origin."));
                    }
                    Ok(json!({"challengeId":challenge_id,"expiresAtUnixMs":expires_at_unix_ms,
                        "passkey":passkey_options,"deviceProofMessage":device_proof_message,"serverAddress":server,"apiOrigin":origin}))
                },
                _=>Err(err("This connection code is unavailable. Ask for a new code.")),
            }
        },
        "meshFinishPrepare"=>{
            let device=identity(v)?;
            let proof=text(v,"deviceProofMessage")?;
            if !proof.starts_with("bluefin-mesh-device-v1:"){return Err(failure());}
            let request=Request::Finish{request_id:bytes32(text(v,"requestId")?)?,
                challenge_id:text(v,"challengeId")?.into(),passkey_response:v["passkeyResponse"].clone(),
                device_signature:device.sign(proof.as_bytes()).map_err(|_|failure())?};
            request.validate().map_err(|_|failure())?;
            // Save these exact bytes before sending. ML-DSA may randomize a signature;
            // re-signing a retry would change the durable request digest.
            let wire=serde_json::to_value(Envelope::new(request)).map_err(|_|failure())?;
            Ok(json!({"request":wire}))
        },
        "meshFinish"=>{
            let device=identity(v)?;let server=address(&v["serverAddress"])?;
            let request:Envelope<Request>=serde_json::from_value(v["request"].clone()).map_err(|_|failure())?;
            request.require_version().map_err(|_|failure())?;request.body.validate().map_err(|_|failure())?;
            if !matches!(request.body,Request::Finish{..}){return Err(failure());}
            let expected=request.body.request_id();
            let value=runtime()?.block_on(exchange(v,&device,&server,&v["request"]))?;
            let response:Envelope<Response>=serde_json::from_value(value).map_err(|_|failure())?;
            response.require_version().map_err(|_|failure())?;
            match response.body {
                Response::FinishResult{request_id,session} if request_id==expected=>{
                    if session["kind"]!="session"&&session["kind"]!="signup"{return Err(failure());}
                    Ok(session)
                },
                _=>Err(err("This connection request could not be completed. Retry the saved request or create a new code.")),
            }
        },
        "meshStatus"=>{
            let device=identity(v)?;let server=address(&v["serverAddress"])?;
            let handle=text(v,"handle")?;bytes32(handle)?;
            let request=json!({"version":1,"body":{"type":"status","handle":handle}});
            let response=runtime()?.block_on(exchange(v,&device,&server,&request))?;
            if response["version"]!=1||response["body"]["type"]!="status_result"||response["body"]["handle"]!=handle{return Err(failure());}
            Ok(response["body"].clone())
        },
        "meshImage"=>{
            let config=Config::from_request(v)?;
            let target=Url::parse(text(v,"resource")?).map_err(|_|failure())?;
            let path=config.path(&target)?;
            if !super::mesh_proxy::media_path(&path){return Err(err("Unsupported mesh image route."));}
            let response=runtime()?.block_on(fetch(v,&config,"GET",&path,&[],None,true))?;
            if response.status!=200||!response.mime.starts_with("image/")||response.bytes.len()>5*1024*1024{return Err(failure());}
            Ok(json!({"bytesBase64":STANDARD.encode(response.bytes),"mime":response.mime}))
        },
        "meshCloseResources"=>{
            super::mesh_proxy::stop_all()?;Ok(Value::Null)
        },
        _=>Err(err("Unsupported mesh operation.")),
    }
}
#[derive(Clone)]
pub(super) struct Config {
    pub identity:DeviceIdentity, pub server:ServerAddress, pub session_id:String,
    pub origin:Url, pub relay_policy:RelayPolicy,
}
impl Config{
    pub fn from_request(v:&Value)->Result<Self>{
        let mesh=v.get("mesh").filter(|m|m.is_object()).ok_or_else(||err("No saved mesh connection."))?;
        let identity=identity(mesh)?;let server=address(&mesh["serverAddress"])?;
        let session_id=text(mesh,"sessionId")?.to_owned();bytes32(&session_id)?;
        let origin=Url::parse(text(mesh,"apiOrigin")?).map_err(|_|failure())?;
        if origin.scheme()!="https"||origin.domain().is_none()||!origin.username().is_empty()||origin.password().is_some()
            ||origin.query().is_some()||origin.fragment().is_some()||origin.path()!="/"{return Err(failure());}
        Ok(Self{identity,server,session_id,origin,relay_policy:policy(v)?})
    }
    pub fn path(&self,target:&Url)->Result<String>{
        if target.origin()!=self.origin.origin()||!target.username().is_empty()||target.password().is_some()
            ||target.fragment().is_some(){return Err(err("This resource is outside the paired server."));}
        let mut result=target.path().to_owned();
        if let Some(query)=target.query(){result.push('?');result.push_str(query);}
        if result.len()>8192{return Err(err("The resource path is too long."));}
        Ok(result)
    }
}
pub(super) struct HttpResponse{pub status:u16,pub mime:String,pub bytes:Vec<u8>,pub etag:Option<String>}
pub(super) async fn fetch(_v:&Value,config:&Config,method:&str,path:&str,headers:&[(String,String)],body:Option<Vec<u8>>,media:bool)->Result<HttpResponse>{
    let endpoint=PqcEndpoint::client_with_policy(&config.identity,config.relay_policy.clone()).await.map_err(|_|failure())?;
    let result=tokio::time::timeout(Duration::from_secs(45),async{
        let mut connection=endpoint.connect_server(&config.server).await.map_err(|_|failure())?;
        let result=async{
            connection.send(&json!({"version":1,"body":{"type":if media{"media"}else{"control"},
                "session_id":config.session_id,"binding":connection.binding(),
                "request":{"method":method,"path":path,"headers":headers,"body":body.unwrap_or_default()}}}))
                .await.map_err(|_|failure())?;
            let head:Value=connection.receive().await.map_err(|_|failure())?;
            if head["version"]!=1||head["body"]["type"]!="response_head"{return Err(failure());}
            let status=head["body"]["status"].as_u64().filter(|s|(100..=599).contains(s)).ok_or_else(failure)? as u16;
            let headers:Vec<(String,String)>=serde_json::from_value(head["body"]["headers"].clone()).map_err(|_|failure())?;
            let mime=headers.iter().find(|(key,_)|key.eq_ignore_ascii_case("content-type")).map(|(_,v)|v.clone()).unwrap_or_default();
            let etag=headers.iter().find(|(key,_)|key.eq_ignore_ascii_case("etag")).map(|(_,v)|v.clone());
            let mut bytes=Vec::new();
            loop{
                let frame:Value=connection.receive().await.map_err(|_|failure())?;
                if frame["version"]!=1{return Err(failure());}
                match frame["body"]["type"].as_str(){
                    Some("response_end")=>break,
                    Some("response_chunk")=>{
                        let chunk:Vec<u8>=serde_json::from_value(frame["body"]["bytes"].clone()).map_err(|_|failure())?;
                        if chunk.len()>8192||bytes.len()+chunk.len()>RESPONSE_MAX{return Err(err("The server response is too large."));}
                        bytes.extend(chunk);
                    },
                    _=>return Err(failure()),
                }
            }
            Ok(HttpResponse{status,mime,bytes,etag})
        }.await;connection.close().await;result
    }).await.map_err(|_|err("The server request timed out."));
    endpoint.close().await;
    result?
}
pub(super) fn request(v:&Value,method:&str,target:&Url,body:Option<Value>)->Result<Value>{
    request_with_headers(v,method,target,body,Vec::new()).map(|(value,_)|value)
}
pub(super) fn request_with_headers(v:&Value,method:&str,target:&Url,body:Option<Value>,mut extra:Vec<(String,String)>)->Result<(Value,Option<String>)>{
    let config=Config::from_request(v)?;let path=config.path(target)?;
    let bytes=body.map(|b|serde_json::to_vec(&b).map_err(|_|failure())).transpose()?;
    if bytes.as_ref().is_some_and(|b|b.len()>8192){return Err(err("This mesh request is too large."));}
    if bytes.is_some(){extra.push(("content-type".into(),"application/json".into()));}
    let headers=extra;
    let response=runtime()?.block_on(fetch(v,&config,method,&path,&headers,bytes,false))?;
    if !(200..300).contains(&response.status){return Err(format!("The server refused this request (HTTP {}).",response.status));}
    let value=if response.bytes.is_empty(){Value::Null}else{serde_json::from_slice(&response.bytes).map_err(|_|err("The server returned an invalid response."))?};
    Ok((value,response.etag))
}

pub(super) async fn revoke(v:&Value)->Result<Value>{
    let config=Config::from_request(v)?;
    let request_id=bytes32(text(v,"requestId")?)?;
    let endpoint=PqcEndpoint::client_with_policy(&config.identity,config.relay_policy.clone()).await.map_err(|_|failure())?;
    let result=tokio::time::timeout(Duration::from_secs(20),async{
        let mut connection=endpoint.connect_server(&config.server).await.map_err(|_|failure())?;
        let result=async{
            connection.send(&json!({"version":1,"body":{"type":"revoke_session",
                "session_id":config.session_id,"request_id":request_id,"binding":connection.binding()}}))
                .await.map_err(|_|failure())?;
            let response:Value=connection.receive().await.map_err(|_|failure())?;
            if response["version"]!=1||response["body"]["type"]!="session_revoked"
                ||response["body"]["session_id"]!=config.session_id
                ||response["body"]["request_id"]!=json!(request_id){
                return Err(err("The server did not confirm revocation."));
            }
            Ok(Value::Null)
        }.await;
        connection.close().await;
        result
    }).await.map_err(|_|err("Remote sign-out is not confirmed yet."));
    endpoint.close().await;
    result?
}
