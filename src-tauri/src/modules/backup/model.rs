use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const BACKUP_FORMAT: &str = "gatetree-ops-workspace-backup";
pub const LEGACY_BACKUP_FORMAT: &str = "pharos-workspace-backup";
pub const BACKUP_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupDocument {
    pub format: String,
    pub version: u32,
    pub schema_version: String,
    pub tables: BTreeMap<String, TableBackup>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableBackup {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<BackupValue>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum BackupValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(String),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSummary {
    pub tables: usize,
    pub records: usize,
}
