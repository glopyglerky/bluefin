//! Bounded public Bonjour metadata. Discovery locates candidates; it never
//! establishes trust or contains invitations, passkeys or account information.
use std::{collections::BTreeMap,net::SocketAddr};
use crate::{Error,Result,ServerAddress,capsule::validate_service};
pub const BLUEFIN_SERVICE_TYPE:&str="_bluefin-mesh._udp.local.";
pub const BLUEFIN_ALPN:&str="bluefin/connect/1";
const MAX_TXT:usize=1024;

/// This value remains untrusted until matched with an already trusted address.
/// A new server must complete the separate QR/passkey onboarding flow.
#[derive(Clone,Debug)]
pub struct Advertisement {
    name:String,
    endpoint_id:[u8;32],
    certificate_sha256:[u8;32],
    service:String,
    direct_addresses:Vec<SocketAddr>,
}
impl Advertisement {
    pub fn new(name:&str,address:&ServerAddress)->Result<Self>{
        address.validate()?;
        validate_name(name)?;
        let mut addresses:Vec<_>=address.direct_addresses.iter().copied().filter(usable_address).collect();
        addresses.sort();addresses.dedup();
        if addresses.is_empty(){return Err(Error::InvalidFields);}
        Ok(Self{name:name.into(),endpoint_id:address.endpoint_id,
            certificate_sha256:address.server_certificate_sha256,service:address.service.clone(),
            direct_addresses:addresses})
    }
    pub fn name(&self)->&str{&self.name}
    pub fn service(&self)->&str{&self.service}
    pub fn direct_addresses(&self)->&[SocketAddr]{&self.direct_addresses}
    pub fn endpoint_id(&self)->[u8;32]{self.endpoint_id}
    /// Deterministic TXT properties. SRV+A/AAAA carry bound addresses.
    pub fn to_txt(&self)->Vec<(String,Vec<u8>)>{
        [
            ("alpn",self.service.clone()),("eid",hex(&self.endpoint_id)),
            ("name",self.name.clone()),("pin",hex(&self.certificate_sha256)),("v","1".into()),
        ].into_iter().map(|(k,v)|(k.into(),v.into_bytes())).collect()
    }
    /// Resolved addresses come from Bonjour, with the actual SRV port.
    /// The native adapter must not import caller-supplied interface claims.
    pub fn from_txt(service_type:&str,expected_alpn:&str,txt:&[(String,Vec<u8>)],resolved:&[SocketAddr])->Result<Self>{
        if !matches!(service_type,BLUEFIN_SERVICE_TYPE|"_bluefin-mesh._udp")
            || txt.len()!=5 || resolved.is_empty() || resolved.len()>4{return Err(Error::InvalidFields);}
        validate_service(expected_alpn)?;
        let mut count=0usize;let mut props=BTreeMap::new();
        for(k,v)in txt{
            count=count.checked_add(k.len()+v.len()+2).ok_or(Error::TooLarge)?;
            if count>MAX_TXT || k.len()+v.len()+1>255 || !matches!(k.as_str(),"v"|"alpn"|"eid"|"pin"|"name")
                || props.insert(k.as_str(),v.as_slice()).is_some(){return Err(Error::InvalidFields);}
        }
        let text=|key:&str|->Result<&str>{
            std::str::from_utf8(props.get(key).copied().ok_or(Error::InvalidFields)?).map_err(|_|Error::InvalidFields)
        };
        if text("v")?!="1"{return Err(Error::UnsupportedVersion);}
        if text("alpn")?!=expected_alpn{return Err(Error::ServiceMismatch);}
        let name=text("name")?;validate_name(name)?;
        if resolved.iter().any(|a|!usable_address(a)){return Err(Error::InvalidFields);}
        let mut direct_addresses=resolved.to_vec();direct_addresses.sort();direct_addresses.dedup();
        if direct_addresses.len()!=resolved.len(){return Err(Error::InvalidFields);}
        let endpoint_id=unhex(text("eid")?)?;
        let certificate_sha256=unhex(text("pin")?)?;
        if endpoint_id==[0;32]||certificate_sha256==[0;32]{return Err(Error::InvalidFields);}
        Ok(Self{name:name.into(),endpoint_id,certificate_sha256,
            service:expected_alpn.into(),direct_addresses})
    }
    /// Only exact stored identity+pin matches refresh direct connection hints.
    /// Trusted relays remain from the authenticated record. TLS still verifies
    /// the pin and key possession on the subsequent connection.
    pub fn matched_address(&self,trusted:&ServerAddress)->Result<ServerAddress>{
        trusted.validate()?;
        if trusted.endpoint_id!=self.endpoint_id
            || trusted.server_certificate_sha256!=self.certificate_sha256
            || trusted.service!=self.service{return Err(Error::Authentication);}
        let mut refreshed=trusted.clone();refreshed.direct_addresses=self.direct_addresses.clone();
        refreshed.validate()?;Ok(refreshed)
    }
}
fn validate_name(value:&str)->Result<()>{
    if value.is_empty()||value.len()>96||value.trim()!=value
        ||value.chars().any(|c|c.is_control()||matches!(c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}')){
        return Err(Error::InvalidFields);
    }Ok(())
}
pub fn usable_address(addr:&SocketAddr)->bool{
    if addr.port()==0||addr.ip().is_unspecified()||addr.ip().is_loopback()||addr.ip().is_multicast(){return false;}
    match addr{
        SocketAddr::V4(a)=>!a.ip().is_broadcast(),
        SocketAddr::V6(a)=>a.scope_id()==0&&a.flowinfo()==0&&!a.ip().is_unicast_link_local(),
    }
}
fn hex(bytes:&[u8;32])->String{bytes.iter().map(|b|format!("{b:02x}")).collect()}
fn unhex(text:&str)->Result<[u8;32]>{
    if text.len()!=64||!text.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)){return Err(Error::InvalidFields);}
    let mut out=[0;32];
    for(i,pair)in text.as_bytes().chunks_exact(2).enumerate(){
        let n=|b:u8|if b.is_ascii_digit(){b-b'0'}else{b-b'a'+10};
        out[i]=(n(pair[0])<<4)|n(pair[1]);
    }Ok(out)
}

/// Merge bounded sightings without silently picking a conflicting identity.
/// No multicast event can create or replace a stored pin.
pub fn merge_sighting(current:&mut Vec<Advertisement>,incoming:Advertisement)->Result<()>{
    if let Some(existing)=current.iter_mut().find(|a|a.endpoint_id==incoming.endpoint_id){
        if existing.certificate_sha256!=incoming.certificate_sha256||existing.service!=incoming.service{
            return Err(Error::Authentication);
        }
        *existing=incoming;return Ok(());
    }
    if current.len()>=64{return Err(Error::TooLarge);}
    current.push(incoming);Ok(())
}
#[cfg(test)]
mod tests{
    use super::*;
    fn address()->ServerAddress{ServerAddress{endpoint_id:[1;32],server_certificate_sha256:[2;32],
        service:BLUEFIN_ALPN.into(),direct_addresses:vec!["192.0.2.4:4433".parse().unwrap()],relay_urls:vec![]}}
    #[test]
    fn bounded_advertisement_updates_only_an_existing_pin(){
        let a=address();let ad=Advertisement::new("Living room server",&a).unwrap();
        let decoded=Advertisement::from_txt(BLUEFIN_SERVICE_TYPE,BLUEFIN_ALPN,&ad.to_txt(),&a.direct_addresses).unwrap();
        assert_eq!(decoded.matched_address(&a).unwrap(),a);
        let mut changed=a;changed.server_certificate_sha256=[3;32];
        assert!(decoded.matched_address(&changed).is_err());
        assert!(!format!("{:?}",ad.to_txt()).contains("secret"));
    }
    #[test]
    fn foreign_ambiguous_oversized_and_unusable_sightings_are_refused(){
        let a=address();let ad=Advertisement::new("Server",&a).unwrap();let txt=ad.to_txt();
        assert!(Advertisement::from_txt("_http._tcp.local.",BLUEFIN_ALPN,&txt,&a.direct_addresses).is_err());
        assert!(Advertisement::from_txt(BLUEFIN_SERVICE_TYPE,"other/1",&txt,&a.direct_addresses).is_err());
        let mut bad=txt.clone();bad[2].1=vec![b'x';1025];
        assert!(Advertisement::from_txt(BLUEFIN_SERVICE_TYPE,BLUEFIN_ALPN,&bad,&a.direct_addresses).is_err());
        let mut bad=txt.clone();bad[4]=bad[0].clone();
        assert!(Advertisement::from_txt(BLUEFIN_SERVICE_TYPE,BLUEFIN_ALPN,&bad,&a.direct_addresses).is_err());
        for ip in ["0.0.0.0:4433","127.0.0.1:4433","224.0.0.251:4433","255.255.255.255:4433","[fe80::1]:4433"]{
            assert!(Advertisement::from_txt(BLUEFIN_SERVICE_TYPE,BLUEFIN_ALPN,&txt,&[ip.parse().unwrap()]).is_err());
        }
        let mut current=vec![ad];let mut other=a;other.server_certificate_sha256=[3;32];
        assert!(merge_sighting(&mut current,Advertisement::new("Same endpoint",&other).unwrap()).is_err());
    }
}
