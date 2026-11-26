use std::{fmt, net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr}};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use crate::{Error, Result};

const MAGIC: &[u8; 4] = b"MC\x01\0";
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const CHECK_DOMAIN: &[u8] = b"mesh-connect/invite-checksum/v1\0";
const MAX_BINARY: usize = 1024;
/// Maximum original text, including human grouping and known URI wrappers.
pub const MAX_CODE_BYTES: usize = 2048;

/// Advisory intent; the owner's stored invitation record remains authoritative.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InviteIntent { Login, Signup }

/// Public address retained after an invitation has been consumed.
/// It deliberately contains neither invitation secret nor invitation expiry.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerAddress {
    pub endpoint_id: [u8; 32],
    pub server_certificate_sha256: [u8; 32],
    /// Exact ASCII ALPN for both outer iroh and inner TLS.
    pub service: String,
    pub relay_urls: Vec<String>,
    pub direct_addresses: Vec<SocketAddr>,
}

impl fmt::Debug for ServerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerAddress").field("service", &self.service)
            .field("relay_count", &self.relay_urls.len())
            .field("direct_address_count", &self.direct_addresses.len()).finish_non_exhaustive()
    }
}

/// The complete v1 capsule. All import/export representations carry these same
/// fields; creating a PNG or copying text never creates another invitation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectInviteV1 {
    pub authority_id: [u8; 32],
    pub authority_generation: u64,
    pub authority_epoch: [u8; 32],
    pub endpoint_id: [u8; 32],
    pub server_certificate_sha256: [u8; 32],
    pub invitation_id: [u8; 32],
    pub secret: [u8; 32],
    pub expires_at_unix_ms: u64,
    pub service: String,
    pub intent: InviteIntent,
    pub account_hint: Option<[u8; 32]>,
    pub relay_urls: Vec<String>,
    pub direct_addresses: Vec<SocketAddr>,
}
pub type ConnectInvite = ConnectInviteV1;

impl fmt::Debug for ConnectInviteV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Do not expose even partial secrets, account hints or bearer codes.
        f.debug_struct("ConnectInviteV1")
            .field("authority_generation", &self.authority_generation)
            .field("expires_at_unix_ms", &self.expires_at_unix_ms)
            .field("service", &self.service).field("intent", &self.intent)
            .field("secret", &"[REDACTED]").finish_non_exhaustive()
    }
}

pub(crate) fn validate_service(service: &str) -> Result<()> {
    if service.is_empty() || service.len() > 64
        || !service.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b)) {
        return Err(Error::InvalidFields);
    }
    Ok(())
}

/// Validate and return the exact normalized HTTPS relay URL. Imported relay
/// hints are not permission to contact that relay: native RelayPolicy checks
/// the owner's allowlist separately.
pub(crate) fn canonical_relay(text: &str) -> Result<String> {
    if text.len() > 256 || text.is_empty() || !text.is_ascii() {
        return Err(Error::InvalidFields);
    }
    let url = url::Url::parse(text).map_err(|_| Error::InvalidFields)?;
    if url.scheme() != "https" || url.host_str().is_none()
        || !url.username().is_empty() || url.password().is_some()
        || url.query().is_some() || url.fragment().is_some()
        || url.path() != "/" {
        return Err(Error::InvalidFields);
    }
    let normalized = url.to_string();
    if normalized != text { return Err(Error::InvalidFields); }
    Ok(normalized)
}

impl ServerAddress {
    pub fn validate(&self) -> Result<()> {
        validate_service(&self.service)?;
        if self.endpoint_id == [0; 32] || self.server_certificate_sha256 == [0; 32]
            || self.relay_urls.len() > 2 || self.direct_addresses.len() > 4
            || (self.relay_urls.is_empty() && self.direct_addresses.is_empty()) {
            return Err(Error::InvalidFields);
        }
        // Sorted unique lists provide one binary representation per capsule.
        for (i, relay) in self.relay_urls.iter().enumerate() {
            canonical_relay(relay)?;
            if i > 0 && self.relay_urls[i-1] >= *relay { return Err(Error::InvalidFields); }
        }
        for (i, addr) in self.direct_addresses.iter().enumerate() {
            if addr.port() == 0 || addr.ip().is_unspecified() || addr.ip().is_multicast() {
                return Err(Error::InvalidFields);
            }
            if let SocketAddr::V6(v6) = addr {
                if v6.flowinfo() != 0 || v6.scope_id() != 0 { return Err(Error::InvalidFields); }
            }
            if i > 0 && self.direct_addresses[i-1] >= *addr { return Err(Error::InvalidFields); }
        }
        Ok(())
    }
}

impl ConnectInviteV1 {
    pub fn server_address(&self) -> ServerAddress {
        ServerAddress {
            endpoint_id: self.endpoint_id, server_certificate_sha256: self.server_certificate_sha256,
            service: self.service.clone(), relay_urls: self.relay_urls.clone(),
            direct_addresses: self.direct_addresses.clone(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        self.server_address().validate()?;
        if self.authority_id == [0;32] || self.authority_epoch == [0;32]
            || self.authority_generation == 0 || self.invitation_id == [0;32]
            || self.secret == [0;32] || self.expires_at_unix_ms == 0
            || self.account_hint == Some([0;32]) {
            return Err(Error::InvalidFields);
        }
        Ok(())
    }
    /// Local expiry is a convenience check only. The live owner must repeat it.
    pub fn validate_at(&self, now_unix_ms: u64) -> Result<()> {
        self.validate()?;
        if now_unix_ms >= self.expires_at_unix_ms { return Err(Error::Expired); }
        Ok(())
    }
    pub fn require_service(&self, expected: &str) -> Result<()> {
        if self.service != expected { return Err(Error::ServiceMismatch); }
        Ok(())
    }
    /// Canonical binary form, including the eight-byte typo checksum.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut out = Vec::with_capacity(384);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.authority_id);
        out.extend_from_slice(&self.authority_generation.to_be_bytes());
        out.extend_from_slice(&self.authority_epoch);
        out.extend_from_slice(&self.endpoint_id);
        out.extend_from_slice(&self.server_certificate_sha256);
        out.extend_from_slice(&self.invitation_id);
        out.extend_from_slice(&self.secret);
        out.extend_from_slice(&self.expires_at_unix_ms.to_be_bytes());
        out.push(self.service.len() as u8);
        out.extend_from_slice(self.service.as_bytes());
        out.push(match self.intent { InviteIntent::Login => 0, InviteIntent::Signup => 1 });
        match self.account_hint {
            Some(hint) => { out.push(1); out.extend_from_slice(&hint); },
            None => out.push(0),
        }
        out.push(self.relay_urls.len() as u8);
        for relay in &self.relay_urls {
            out.extend_from_slice(&(relay.len() as u16).to_be_bytes());
            out.extend_from_slice(relay.as_bytes());
        }
        out.push(self.direct_addresses.len() as u8);
        for addr in &self.direct_addresses {
            match addr.ip() {
                IpAddr::V4(ip) => { out.push(4); out.extend_from_slice(&ip.octets()); },
                IpAddr::V6(ip) => { out.push(6); out.extend_from_slice(&ip.octets()); },
            }
            out.extend_from_slice(&addr.port().to_be_bytes());
        }
        if out.len() + 8 > MAX_BINARY { return Err(Error::TooLarge); }
        let checksum = checksum(&out);
        out.extend_from_slice(&checksum);
        Ok(out)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_BINARY { return Err(Error::TooLarge); }
        if bytes.len() < 12 { return Err(Error::InvalidCode); }
        if &bytes[..2] != b"MC" { return Err(Error::InvalidCode); }
        if bytes[2] != 1 || bytes[3] != 0 { return Err(Error::UnsupportedVersion); }
        let payload_len = bytes.len()-8;
        if checksum(&bytes[..payload_len]) != bytes[payload_len..] {
            return Err(Error::Checksum);
        }
        let mut r = Reader { bytes: &bytes[4..payload_len], at: 0 };
        let authority_id = r.array()?;
        let authority_generation = u64::from_be_bytes(r.array()?);
        let authority_epoch = r.array()?;
        let endpoint_id = r.array()?;
        let server_certificate_sha256 = r.array()?;
        let invitation_id = r.array()?;
        let secret = r.array()?;
        let expires_at_unix_ms = u64::from_be_bytes(r.array()?);
        let service_len = r.byte()? as usize;
        if service_len > 64 { return Err(Error::InvalidFields); }
        let service = r.string(service_len)?;
        let intent = match r.byte()? { 0 => InviteIntent::Login, 1 => InviteIntent::Signup, _ => return Err(Error::InvalidFields) };
        let account_hint = match r.byte()? { 0 => None, 1 => Some(r.array()?), _ => return Err(Error::InvalidFields) };
        let count = r.byte()?;
        if count > 2 { return Err(Error::InvalidFields); }
        let mut relay_urls = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let len = u16::from_be_bytes(r.array()?) as usize;
            if len > 256 { return Err(Error::InvalidFields); }
            relay_urls.push(r.string(len)?);
        }
        let count = r.byte()?;
        if count > 4 { return Err(Error::InvalidFields); }
        let mut direct_addresses = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let ip = match r.byte()? {
                4 => IpAddr::V4(Ipv4Addr::from(r.array::<4>()?)),
                6 => IpAddr::V6(Ipv6Addr::from(r.array::<16>()?)),
                _ => return Err(Error::InvalidFields),
            };
            direct_addresses.push(SocketAddr::new(ip, u16::from_be_bytes(r.array()?)));
        }
        if r.at != r.bytes.len() { return Err(Error::InvalidCode); }
        let invite = Self { authority_id, authority_generation, authority_epoch, endpoint_id,
            server_certificate_sha256, invitation_id, secret, expires_at_unix_ms, service,
            intent, account_hint, relay_urls, direct_addresses };
        invite.validate()?;
        Ok(invite)
    }
    /// Grouped, checksummed Crockford base32. Validation failures never emit a code.
    pub fn to_code(&self) -> Result<String> {
        let raw = base32_encode(&self.to_bytes()?);
        let mut code = String::with_capacity(raw.len()+raw.len()/5+4);
        code.push_str("MC1-");
        for (i, ch) in raw.chars().enumerate() {
            if i > 0 && i % 5 == 0 { code.push('-'); }
            code.push(ch);
        }
        Ok(code)
    }
    pub fn to_uri(&self) -> Result<String> {
        Ok(format!("mesh-connect://invite/{}", self.to_code()?))
    }
    /// Accept only documented wrappers, ASCII case, hyphens and whitespace.
    /// Ambiguous Crockford O/I/L aliases are deliberately not accepted.
    pub fn parse(input: &str) -> Result<Self> {
        if input.len() > crate::launch::MAX_WEB_LAUNCH_BYTES { return Err(Error::TooLarge); }
        let input = input.trim_matches(|c: char| c.is_ascii_whitespace());
        if input.starts_with("https://") { return crate::launch::parse_web_launch(input); }
        if input.len() > MAX_CODE_BYTES { return Err(Error::TooLarge); }
        let payload = input.strip_prefix("mesh-connect://invite/")
            .or_else(|| input.strip_prefix("bluefin://connect/")).unwrap_or(input);
        if !payload.is_ascii() { return Err(Error::InvalidCode); }
        let compact: String = payload.chars().filter(|c| *c != '-' && !c.is_ascii_whitespace())
            .map(|c| c.to_ascii_uppercase()).collect();
        let body = compact.strip_prefix("MC1").ok_or(Error::UnsupportedVersion)?;
        Self::from_bytes(&base32_decode(body)?)
    }
    pub fn parse_at(input: &str, now_unix_ms: u64) -> Result<Self> {
        let invite = Self::parse(input)?;
        invite.validate_at(now_unix_ms)?;
        Ok(invite)
    }
}
fn checksum(bytes: &[u8]) -> [u8;8] {
    let mut h = Sha256::new(); h.update(CHECK_DOMAIN); h.update(bytes);
    let d = h.finalize(); let mut sum = [0;8]; sum.copy_from_slice(&d[..8]); sum
}
fn base32_encode(bytes: &[u8]) -> String {
    let mut out = String::new(); let mut acc = 0u32; let mut bits = 0;
    for &byte in bytes {
        acc = (acc << 8) | u32::from(byte); bits += 8;
        while bits >= 5 { bits -= 5; out.push(ALPHABET[((acc >> bits)&31) as usize] as char); }
    }
    if bits > 0 { out.push(ALPHABET[((acc << (5-bits))&31) as usize] as char); }
    out
}
fn base32_decode(text: &str) -> Result<Vec<u8>> {
    if text.len() > MAX_BINARY.div_ceil(5)*8 { return Err(Error::TooLarge); }
    let mut out=Vec::with_capacity(text.len()*5/8); let mut acc=0u32; let mut bits=0;
    for byte in text.bytes() {
        let v=ALPHABET.iter().position(|&b| b == byte).ok_or(Error::InvalidCode)?;
        acc=(acc<<5)|(v as u32); bits+=5;
        if bits>=8 { bits-=8; out.push(((acc>>bits)&255) as u8); }
    }
    if bits >= 5 || (bits > 0 && (acc & ((1<<bits)-1)) != 0)
        || base32_encode(&out) != text { return Err(Error::InvalidCode); }
    Ok(out)
}
struct Reader<'a> { bytes: &'a [u8], at: usize }
impl Reader<'_> {
    fn take(&mut self, len:usize)->Result<&[u8]> {
        let end=self.at.checked_add(len).ok_or(Error::InvalidCode)?;
        let out=self.bytes.get(self.at..end).ok_or(Error::InvalidCode)?;self.at=end;Ok(out)
    }
    fn byte(&mut self)->Result<u8>{Ok(self.take(1)?[0])}
    fn array<const N:usize>(&mut self)->Result<[u8;N]>{
        self.take(N)?.try_into().map_err(|_|Error::InvalidCode)
    }
    fn string(&mut self,len:usize)->Result<String>{
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_|Error::InvalidCode)
    }
}
