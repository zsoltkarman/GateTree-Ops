use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentConfigResource {
    pub id: String,
    pub name: String,
    pub definition: String,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentConfigSnapshot {
    pub drivers: Vec<DeploymentConfigResource>,
    pub processes: Vec<DeploymentConfigResource>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentConfigValidation {
    pub formatted: String,
    pub id: String,
    pub name: String,
}
