//! Provider-independent discovery data. No provider configuration or authority.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryCapabilities {
    pub search: bool,
    pub requests: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryPage {
    pub results: Vec<DiscoveryItem>,
    pub page: u32,
    pub total_pages: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryItem {
    pub id: Uuid,
    pub title: String,
    pub overview: String,
    pub year: Option<String>,
    pub kind: String,
    /// A catalog status, never permission to play a stream.
    pub availability: String,
    pub actions: Vec<DiscoveryAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryAction {
    pub id: Uuid,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionForm {
    pub id: Uuid,
    pub title: String,
    pub notice: String,
    pub choices: Vec<ActionChoice>,
    pub requires_selection: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionChoice {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubmitAction {
    pub action_id: Uuid,
    pub selections: Vec<String>,
    /// Deliberate acknowledgement; it does not establish ownership or rights.
    pub authorized_use_confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionReceipt {
    pub status: String,
    pub message: String,
}
