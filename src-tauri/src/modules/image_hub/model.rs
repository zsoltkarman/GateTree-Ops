use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageHubFolder {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderInput {
    pub id: Option<String>,
    pub name: String,
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Repository {
    pub id: String,
    pub folder_id: Option<String>,
    pub name: String,
    pub local_path: String,
    pub ocid: String,
    pub url: String,
    pub repo_type: String,
    pub browser_url: Option<String>,
    pub project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryInput {
    pub id: Option<String>,
    pub folder_id: Option<String>,
    pub name: String,
    pub local_path: String,
    pub ocid: String,
    pub url: String,
    pub repo_type: String,
    #[serde(default)]
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInput {
    pub id: Option<String>,
    pub folder_id: Option<String>,
    pub name: String,
    pub local_path: String,
    pub project_type: String,
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub folder_id: Option<String>,
    pub name: String,
    pub local_path: String,
    pub project_type: String,
    pub source: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageHubSnapshot {
    pub folders: Vec<ImageHubFolder>,
    pub repositories: Vec<Repository>,
    pub projects: Vec<Project>,
    pub sync_statuses: Vec<super::sync_job::SyncStatus>,
    pub security_reports: Vec<super::security::SecurityReport>,
    pub latest_images: Vec<super::artifacts::LatestImage>,
    pub config: super::config::ImageHubConfig,
    pub runtime_config: super::config::ImageHubConfig,
}
