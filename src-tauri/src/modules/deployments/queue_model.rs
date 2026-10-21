use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot {
    pub plans: Vec<QueuePlan>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuePlan {
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
