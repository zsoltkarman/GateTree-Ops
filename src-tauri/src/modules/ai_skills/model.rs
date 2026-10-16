use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSkillLibrary {
    pub id: String,
    pub name: String,
    pub path: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSkill {
    pub id: String,
    pub library_id: String,
    pub name: String,
    pub description: String,
    pub path: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSkillsSnapshot {
    pub libraries: Vec<AiSkillLibrary>,
    pub skills: Vec<AiSkill>,
    pub groups: Vec<AiSkillGroup>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSkillGroup {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub skill_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSkillDocument {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: String,
    pub content: String,
    pub input_prompt: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSkillRunResult {
    pub output: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSkillRefreshResult {
    pub discovered: usize,
    pub skipped_libraries: Vec<String>,
}
