//! Versioned onboarding messages. Account policy and consumption belong to
//! the live application owner; these types cannot authorize an invitation.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use crate::{ConnectInviteV1, Error, Result};
pub const VERSION: u16 = 1;
pub const MAX_FRAME_BYTES: usize = 65_536;
pub const ML_DSA_65_PUBLIC_KEY_BYTES: usize = 1_952;
pub const ML_DSA_65_SIGNATURE_BYTES: usize = 3_309;

/// A fixed version envelope keeps future formats from being silently accepted.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope<T> { pub version: u16, pub body: T }
impl<T> Envelope<T> {
    pub fn new(body: T) -> Self { Self { version: VERSION, body } }
    pub fn require_version(&self) -> Result<()> {
        if self.version != VERSION { return Err(Error::UnsupportedVersion); }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Begin {
        request_id: [u8;32],
        invitation: ConnectInviteV1,
        device_public_key: Vec<u8>,
        device_label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_name: Option<String>,
    },
    Finish {
        request_id: [u8;32],
        challenge_id: String,
        passkey_response: serde_json::Value,
        device_signature: Vec<u8>,
    },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    BeginResult {
        request_id: [u8;32],
        challenge_id: String,
        expires_at_unix_ms: u64,
        passkey_options: serde_json::Value,
        device_proof_message: String,
    },
    FinishResult {
        request_id: [u8;32],
        /// An owner-defined public descriptor, never an unbound bearer token.
        session: serde_json::Value,
    },
    Error { request_id: [u8;32], code: FailureCode },
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode { Invalid, Expired, Used, Denied, RetryLater }
fn valid_id(id: &[u8;32]) -> bool { *id != [0;32] }
fn valid_challenge(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl Request {
    pub fn request_id(&self) -> [u8;32] {
        match self { Self::Begin{request_id,..} | Self::Finish{request_id,..} => *request_id }
    }
    /// Call after decoding, before invoking the account owner.
    pub fn validate(&self) -> Result<()> {
        if !valid_id(&self.request_id()) { return Err(Error::InvalidMessage); }
        match self {
            Self::Begin{invitation,device_public_key,device_label,account_name,..} => {
                invitation.validate()?;
                if let Some(name)=account_name {
                    if invitation.intent==crate::InviteIntent::Login || name.is_empty() || name.len()>64 || name.chars().any(char::is_control) { return Err(Error::InvalidMessage); }
                }
                if device_public_key.len()!=ML_DSA_65_PUBLIC_KEY_BYTES
                    || device_label.is_empty() || device_label.len()>128
                    || device_label.chars().any(char::is_control) {
                    return Err(Error::InvalidMessage);
                }
            },
            Self::Finish{challenge_id,device_signature,passkey_response,..} => {
                if !valid_challenge(challenge_id)
                    || device_signature.len()!=ML_DSA_65_SIGNATURE_BYTES
                    || !passkey_response.is_object() {
                    return Err(Error::InvalidMessage);
                }
            }
        }
        Ok(())
    }
}
/// Serialization intentionally has no logging and never includes payloads in
/// an error. Framing bounds apply even to application-specific messages.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let bytes=serde_json::to_vec(value).map_err(|_|Error::InvalidMessage)?;
    if bytes.is_empty() || bytes.len()>MAX_FRAME_BYTES { return Err(Error::TooLarge); }
    Ok(bytes)
}
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    if bytes.is_empty() || bytes.len()>MAX_FRAME_BYTES { return Err(Error::TooLarge); }
    serde_json::from_slice(bytes).map_err(|_|Error::InvalidMessage)
}
