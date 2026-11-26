//! Native Bonjour sightings. Discovery may refresh a stored pin's route hints;
//! it never creates trust, consumes an invitation or signs a user in.
use super::{err,Result};
use bluefin_mesh_connect::discovery::{Advertisement,BLUEFIN_ALPN,BLUEFIN_SERVICE_TYPE,usable_address};
use serde_json::{json,Value};
use std::net::{IpAddr,Ipv4Addr,Ipv6Addr,SocketAddr};
fn failed()->String{err("The nearby server advertisement is invalid.")}
fn darwin_address(bytes:&[u8])->Option<SocketAddr>{
    // Apple's sockaddr_in / sockaddr_in6 layouts. Parse bounded bytes without
    // dereferencing foreign pointers or relying on host alignment.
    if bytes.first().copied()? as usize!=bytes.len(){return None;}
    let port=u16::from_be_bytes([*bytes.get(2)?,*bytes.get(3)?]);
    match (bytes.len(),bytes.get(1).copied()?){
        (16,2)=>Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(bytes[4],bytes[5],bytes[6],bytes[7])),port)),
        (28,30) if bytes[24..28]==[0,0,0,0]=>{
            let ip=<[u8;16]>::try_from(&bytes[8..24]).ok()?;
            Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(ip)),port))
        },
        _=>None,
    }
}
pub(super) fn call(v:&Value,operation:&str)->Result<Value>{
    let txt:Vec<(String,Vec<u8>)>=serde_json::from_value(v["txt"].clone()).map_err(|_|failed())?;
    let mut addresses=if operation=="meshLanDecode"{
        let raw:Vec<Vec<u8>>=serde_json::from_value(v["addresses"].clone()).map_err(|_|failed())?;
        if raw.len()>32||raw.iter().any(|a|a.len()>128){return Err(failed());}
        raw.iter().filter_map(|bytes|darwin_address(bytes)).filter(usable_address).collect::<Vec<_>>()
    }else{
        serde_json::from_value::<Vec<SocketAddr>>(v["addresses"].clone()).map_err(|_|failed())?
    };
    addresses.sort();addresses.dedup();addresses.truncate(4);
    let ad=Advertisement::from_txt(BLUEFIN_SERVICE_TYPE,BLUEFIN_ALPN,&txt,&addresses).map_err(|_|failed())?;
    if operation=="meshLanRefresh"{
        let trusted=super::mesh_connection::address(&v["serverAddress"])?;
        return serde_json::to_value(ad.matched_address(&trusted).map_err(|_|err("This nearby server does not match the saved identity."))?)
            .map_err(|_|failed());
    }
    Ok(json!({"name":ad.name(),"endpointId":ad.endpoint_id().iter().map(|b|format!("{b:02x}")).collect::<String>(),
        "txt":ad.to_txt(),"addresses":ad.direct_addresses(),"trusted":false}))
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn bounded_darwin_sockaddrs_do_not_accept_foreign_layouts(){
        let bytes=[16,2,0x10,0x01,192,0,2,5,0,0,0,0,0,0,0,0];
        assert_eq!(darwin_address(&bytes),Some("192.0.2.5:4097".parse().unwrap()));
        assert!(darwin_address(&bytes[..8]).is_none());
        let mut foreign=bytes;foreign[1]=30;assert!(darwin_address(&foreign).is_none());
    }
}
