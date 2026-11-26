//! Optional transport advertisement; no account grant or reachability assertion.
use serde::{Deserialize, Serialize};
/// Published only while the opted-in endpoint is running and the viewer is permitted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IrohMediaTransport {
    /// Protocol revision; one HTTP/1.1 GET/HEAD request per pinned connection.
    pub version: u32,
    /// Public Iroh endpoint identity, never a secret seed.
    pub endpoint_id: String,
    /// Operator-bound direct UDP hints, not verified public accessibility.
    pub direct_addresses: Vec<String>,
    /// Exact application protocol.
    pub alpn: String,
    /// True for this initial direct-only implementation.
    pub direct_only: bool,
}
