use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInspection {
    pub path: String,
    pub name: String,
    pub kind: WorkspaceKind,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceKind {
    New,
    Existing,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSessionView {
    pub path: String,
    pub name: String,
    pub cipher_version: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BiometricStatusView {
    pub available: bool,
    pub configured: bool,
    pub label: String,
}
