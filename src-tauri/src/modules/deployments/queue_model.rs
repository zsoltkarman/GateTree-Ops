use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueLogChunk {
    pub reset: bool,
    pub text: String,
    pub total_bytes: usize,
    pub hash: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot {
    pub plans: Vec<QueuePlan>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuePlan {
    pub group: Option<QueuePlanGroup>,
    pub archived: bool,
    pub parallel_regions: u8,
    pub approve_all: bool,
    pub id: String,
    pub plan_number: i64,
    pub name: String,
    pub process_name: String,
    pub definition: String,
    pub status: String,
    pub created_at: String,
    pub jobs: Vec<QueueJob>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueJob {
    pub report_unconfirmed: bool,
    pub region: Option<String>,
    pub id: String,
    pub sequence: i64,
    pub name: String,
    pub target: Option<String>,
    pub details: String,
    pub payload: Option<String>,
    pub log: String,
    pub driver: String,
    pub action: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedQueueSnapshot {
    #[serde(flatten)]
    pub queue: QueueSnapshot,
    pub created_plan_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QueuePlanGroup { pub id: String, pub name: String }

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedBatchSnapshot {
    #[serde(flatten)]
    pub queue: QueueSnapshot,
    pub created_plan_ids: Vec<String>,
}
