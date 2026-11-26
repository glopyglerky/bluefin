//! Literal-loopback URL adapter for native media readers. Control stays on Rust
//! PQ connections. Each route requires a fresh process capability and forwards
//! only to the captured paired server; callers cannot choose a destination.
use super::{err,Result,mesh_connection::Config};
use bluefin_mesh_connect::native::{PqcEndpoint,PqcConnection};
use serde_json::{json,Value};
use std::{collections::HashMap,sync::{Arc,Mutex,OnceLock,atomic::{AtomicU64,Ordering}},time::Duration};
use tokio::{io::{AsyncReadExt,AsyncWriteExt},net::{TcpListener,TcpStream},sync::{oneshot,Semaphore},task::JoinSet};
static NEXT:AtomicU64=AtomicU64::new(1);
static PROXIES:OnceLock<Mutex<HashMap<u64,Proxy>>>=OnceLock::new();
struct Proxy { stop:Option<oneshot::Sender<()>>,thread:Option<std::thread::JoinHandle<Result<()>>> }
fn proxies()->&'static Mutex<HashMap<u64,Proxy>>{PROXIES.get_or_init(Default::default)}
fn failure()->String{err("The mesh media stream is unavailable.")}
pub(super) fn media_path(path:&str)->bool{
    if path.len()>8192||!path.starts_with('/')||path.starts_with("//")||path.contains(['\\','\r','\n']){return false;}
    let route=path.split('?').next().unwrap_or("");
    let lower=route.to_ascii_lowercase();
    if ["%2f","%5c","%2e"].iter().any(|s|lower.contains(s)){return false;}
    let parts:Vec<_>=route.split('/').filter(|p|!p.is_empty()).collect();
    if parts.len()>12||parts.iter().any(|p|*p=="."||*p==".."){return false;}
    match parts.as_slice(){
        ["Items",id,"Images",..] if !id.is_empty()=>true,
        ["Items",id,"Download"] if !id.is_empty()=>true,
        ["Videos",id,tail @ ..]|["Audio",id,tail @ ..] if !id.is_empty()&&!tail.is_empty()=>true,
        ["LiveTv","LiveStreamFiles",..]=>true,
        ["Bluefin","Karaoke",..]=>true,
        _=>false,
    }
}
pub(super) fn start(config:Config)->Result<(u64,String)>{
    let mut registry=proxies().lock().map_err(|_|failure())?;
    if registry.len()>=2{return Err(err("Too many active mesh media readers."));}
    let mut random=[0;32];getrandom::fill(&mut random).map_err(|_|failure())?;
    let cap=random.iter().map(|b|format!("{b:02x}")).collect::<String>();
    let id=NEXT.fetch_add(1,Ordering::Relaxed);
    let (stop,receiver)=oneshot::channel();
    let (ready,started)=std::sync::mpsc::sync_channel(1);
    let thread=std::thread::Builder::new().name("bluefin-mesh-media".into()).spawn(move||{
        let runtime=tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_|failure())?;
        runtime.block_on(run(config,cap,receiver,ready))
    }).map_err(|_|failure())?;
    match started.recv_timeout(Duration::from_secs(10)){
        Ok(Ok(base))=>{registry.insert(id,Proxy{stop:Some(stop),thread:Some(thread)});Ok((id,base))},
        _=>{let _=stop.send(());let _=thread.join();Err(failure())},
    }
}
async fn run(config:Config,cap:String,mut stop:oneshot::Receiver<()>,ready:std::sync::mpsc::SyncSender<Result<String>>)->Result<()>{
    let endpoint=match tokio::time::timeout(Duration::from_secs(5),PqcEndpoint::client_with_policy(&config.identity,config.relay_policy.clone())).await{
        Ok(Ok(endpoint))=>endpoint,_=>{let _=ready.send(Err(failure()));return Err(failure());}
    };
    let listener=match TcpListener::bind(("127.0.0.1",0)).await{
        Ok(listener)=>listener,Err(_)=>{let _=ready.send(Err(failure()));endpoint.close().await;return Err(failure());}
    };
    let host=listener.local_addr().map_err(|_|failure())?.to_string();
    let base=format!("http://{host}/{cap}");
    if ready.send(Ok(base)).is_err(){endpoint.close().await;return Ok(());}
    let config=Arc::new(config);let limit=Arc::new(Semaphore::new(4));let mut jobs=JoinSet::new();
    loop{
        tokio::select!{
            _=&mut stop=>break,
            Some(_)=jobs.join_next(),if !jobs.is_empty()=>{},
            accepted=listener.accept()=>{
                let Ok((mut local,remote))=accepted else{break};
                if !remote.ip().is_loopback(){continue;}
                let Ok(permit)=limit.clone().try_acquire_owned() else{
                    let _=local.write_all(b"HTTP/1.1 503 Busy\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").await;continue;
                };
                let endpoint=endpoint.clone();let config=config.clone();let cap=cap.clone();let host=host.clone();
                jobs.spawn(async move{
                    let _permit=permit;
                    let _=tokio::time::timeout(Duration::from_secs(4*60*60),serve(&endpoint,&config,&mut local,&host,&cap)).await;
                });
            }
        }
    }
    jobs.abort_all();while jobs.join_next().await.is_some(){}
    tokio::time::timeout(Duration::from_secs(5),endpoint.close()).await.map_err(|_|err("Mesh endpoint cleanup timed out."))?;
    Ok(())
}
async fn serve(endpoint:&PqcEndpoint,config:&Config,local:&mut TcpStream,host:&str,cap:&str)->Result<()>{
    let request=read_request(local,host,cap).await;
    let (method,path,headers)=match request{
        Ok(request)=>request,Err(_)=>{let _=local.write_all(b"HTTP/1.1 403 Refused\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").await;return Err(failure());}
    };
    let mut connection=endpoint.connect_server(&config.server).await.map_err(|_|failure())?;
    let result=async{
        connection.send(&json!({"version":1,"body":{"type":"media","session_id":config.session_id,
            "binding":connection.binding(),"request":{"method":method,"path":path,"headers":headers,"body":[]}}}))
            .await.map_err(|_|failure())?;
        stream_response(&mut connection,local,&method).await
    }.await;
    connection.close().await;
    result
}
async fn read_request(local:&mut TcpStream,host:&str,cap:&str)->Result<(String,String,Vec<(String,String)>)>{
    let mut raw=Vec::with_capacity(2048);
    loop{
        let mut chunk=[0;1024];
        let count=tokio::time::timeout(Duration::from_secs(5),local.read(&mut chunk)).await.map_err(|_|failure())?.map_err(|_|failure())?;
        if count==0{return Err(failure());}raw.extend_from_slice(&chunk[..count]);
        if raw.len()>16*1024{return Err(failure());}
        if raw.windows(4).any(|v|v==b"\r\n\r\n"){break;}
    }
    let mut slots=[httparse::EMPTY_HEADER;32];let mut request=httparse::Request::new(&mut slots);
    if !request.parse(&raw).map_err(|_|failure())?.is_complete()||request.version!=Some(1){return Err(failure());}
    let method=request.method.filter(|m|matches!(*m,"GET"|"HEAD")).ok_or_else(failure)?.to_owned();
    let path=request.path.ok_or_else(failure)?.strip_prefix(&format!("/{cap}")).filter(|p|p.starts_with('/')).ok_or_else(failure)?.to_owned();
    if !media_path(&path){return Err(failure());}
    let mut seen=std::collections::HashSet::new();let mut host_ok=false;let mut headers=Vec::new();
    for header in request.headers{
        let key=header.name.to_ascii_lowercase();
        if !seen.insert(key.clone()){return Err(failure());}
        let value=std::str::from_utf8(header.value).map_err(|_|failure())?;
        if value.chars().any(char::is_control){return Err(failure());}
        match key.as_str(){
            "host"=>{if value!=host{return Err(failure());}host_ok=true;},
            "content-length"=>if value!="0"{return Err(failure());},
            "transfer-encoding"|"origin"|"cookie"|"upgrade"|"proxy-authorization"=>return Err(failure()),
            "range"|"if-range"|"if-none-match"|"if-modified-since"|"accept"=>headers.push((key,value.to_owned())),
            _=>{},
        }
    }
    if !host_ok{return Err(failure());}
    Ok((method,path,headers))
}
async fn stream_response(connection:&mut PqcConnection,local:&mut TcpStream,method:&str)->Result<()>{
    let head:Value=connection.receive().await.map_err(|_|failure())?;
    if head["version"]!=1||head["body"]["type"]!="response_head"{return Err(failure());}
    let status=head["body"]["status"].as_u64().filter(|v|(100..=599).contains(v)).ok_or_else(failure)?;
    let headers:Vec<(String,String)>=serde_json::from_value(head["body"]["headers"].clone()).map_err(|_|failure())?;
    if headers.len()>32{return Err(failure());}
    let mut output=format!("HTTP/1.1 {status} Response\r\nConnection: close\r\nCache-Control: no-store\r\n");
    for (key,value) in headers{
        let lower=key.to_ascii_lowercase();
        if !["content-type","content-length","content-range","accept-ranges","etag","last-modified"].contains(&lower.as_str()){continue;}
        if value.len()>2048||value.chars().any(char::is_control){return Err(failure());}
        output.push_str(&lower);output.push_str(": ");output.push_str(&value);output.push_str("\r\n");
    }
    output.push_str("\r\n");local.write_all(output.as_bytes()).await.map_err(|_|failure())?;
    loop{
        let frame:Value=connection.receive().await.map_err(|_|failure())?;
        if frame["version"]!=1{return Err(failure());}
        match frame["body"]["type"].as_str(){
            Some("response_end")=>break,
            Some("response_chunk")=>{
                let bytes:Vec<u8>=serde_json::from_value(frame["body"]["bytes"].clone()).map_err(|_|failure())?;
                if bytes.len()>8192||method=="HEAD"{return Err(failure());}
                tokio::time::timeout(Duration::from_secs(20),local.write_all(&bytes)).await.map_err(|_|failure())?.map_err(|_|failure())?;
            },
            _=>return Err(failure()),
        }
    }
    local.shutdown().await.map_err(|_|failure())
}
pub(super) fn stop(id:u64)->Result<()>{
    let mut registry=proxies().lock().map_err(|_|failure())?;
    if let Some(mut proxy)=registry.remove(&id){
        if let Some(stop)=proxy.stop.take(){let _=stop.send(());}
        if let Some(thread)=proxy.thread.take(){thread.join().map_err(|_|err("Mesh media cleanup failed."))??;}
    }
    Ok(())
}
pub(super) fn stop_all()->Result<()>{
    let ids:Vec<_>=proxies().lock().map_err(|_|failure())?.keys().copied().collect();
    for id in ids{stop(id)?;}Ok(())
}
