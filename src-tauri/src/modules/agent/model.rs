use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub running: bool,
    pub label: String,
    pub can_focus_browser: bool,
    pub pid: Option<u32>,
    pub version: Option<String>,
    pub protocol_version: Option<u16>,
    pub expected_protocol_version: u16,
    pub expected_version: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentActivity {
    pub id: String,
    pub connection_id: Option<String>,
    pub resource_type: String,
    pub name: String,
    pub target: String,
    pub status: String,
    pub opened_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshHostKeyInspection {
    pub host: String,
    pub port: u16,
    pub state: String,
    pub fingerprint: String,
    pub public_key: String,
    pub trusted_fingerprint: Option<String>,
}
