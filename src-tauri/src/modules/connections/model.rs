use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionFolder {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub credential_reference: Option<String>,
    pub credential_mode: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionItem {
    pub id: String,
    pub folder_id: String,
    pub connection_type: String,
    pub name: String,
    pub target: String,
    pub port: Option<u16>,
    pub tag: String,
    pub username: Option<String>,
    pub authentication: Option<String>,
    pub credential_reference: Option<String>,
    pub credential_mode: String,
    pub ssh_transport: String,
    pub icon: String,
    pub created_at: String,
    pub updated_at: String,
}
