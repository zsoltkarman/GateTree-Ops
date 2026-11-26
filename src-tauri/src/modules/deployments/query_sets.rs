//! Saved ALL-of health gates. Plans freeze membership/policy; query text stays live.
use crate::modules::workspace::WorkspaceState;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuerySetItem {
    pub query_id: String,
    pub scope: String,
    #[serde(default)]
    pub expected_version: String,
    pub min_results: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuerySet {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub revision: i64,
    pub items: Vec<QuerySetItem>,
    pub stable_samples: u32,
    pub interval_seconds: u64,
    pub timeout_seconds: u64,
    pub max_age_seconds: u64,
}
impl QuerySet {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty()
            || self.name.len() > 160
            || self.items.is_empty()
            || self.items.len() > 32
        {
            return Err(
                "Query Set needs a name (maximum 160 bytes) and 1..32 required queries.".into(),
            );
        }
        if !(1..=100).contains(&self.stable_samples)
            || !(1..=86400).contains(&self.timeout_seconds)
            || self.interval_seconds == 0
            || self.interval_seconds > self.timeout_seconds
            || u64::from(self.stable_samples - 1) * self.interval_seconds >= self.timeout_seconds
            || !(1..=3600).contains(&self.max_age_seconds)
        {
            return Err("Invalid Query Set polling policy: use 1..100 consecutive rounds, a 1..86400s timeout and 1..3600s maximum result age; all rounds must fit within the timeout.".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for item in &self.items {
            if item.query_id.trim().is_empty()
                || !ids.insert(&item.query_id)
                || !matches!(item.scope.as_str(), "node" | "cluster")
                || !(1..=10000).contains(&item.min_results)
                || item.expected_version.len() > 256
            {
                return Err("Each Query Set item needs a unique saved query, node/cluster scope and 1..10000 minimum results.".into());
            }
        }
        Ok(())
    }
}

pub(super) fn load(db: &Connection) -> rusqlite::Result<Vec<QuerySet>> {
    db.prepare(
        "SELECT definition_json FROM deployment_validation_query_sets ORDER BY name COLLATE NOCASE",
    )?
    .query_map([], |row| {
        let text: String = row.get(0)?;
        serde_json::from_str(&text).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })
    })?
    .collect()
}

fn save(state: &WorkspaceState, mut set: QuerySet) -> Result<QuerySet, String> {
    set.name = set.name.trim().into();
    set.validate()?;
    state.with_connection(|db| {
        let queries = super::validation_queries::load(db)?;
        for item in &set.items {
            let query = queries.iter().find(|q| q.id == item.query_id)
                .ok_or_else(|| rusqlite::Error::InvalidParameterName("A selected query was deleted. Reload Config.".into()))?;
            validate_item(item, &query.query).map_err(rusqlite::Error::InvalidParameterName)?;
        }
        if set.id.is_empty() {
            set.id = Uuid::new_v4().to_string(); set.revision = 1;
            db.execute("INSERT INTO deployment_validation_query_sets(id,name,definition_json,revision) VALUES(?1,?2,?3,?4)",
                params![set.id,set.name,serde_json::to_string(&set).unwrap(),set.revision])?;
        } else {
            let revision = set.revision; set.revision += 1;
            let updated = db.execute("UPDATE deployment_validation_query_sets SET name=?2,definition_json=?3,revision=?4 WHERE id=?1 AND revision=?5",
                params![set.id,set.name,serde_json::to_string(&set).unwrap(),set.revision,revision])?;
            if updated != 1 { return Err(rusqlite::Error::InvalidParameterName("Query Set changed or was deleted. Reload Config.".into())); }
        }
        Ok(())
    }).map_err(|e| e.to_string())?;
    Ok(set)
}

pub(super) fn validate_item(item: &QuerySetItem, query: &str) -> Result<(), String> {
    super::queue_service::validate_thanos_query_template(query)?;
    if item.scope == "node" && !query.contains("${DEPLOY_HOST_REGEX}") {
        return Err("Node queries require ${DEPLOY_HOST_REGEX}.".into());
    }
    if item.scope == "cluster"
        && (query.contains("${DEPLOY_HOST_REGEX}") || query.contains("${DEPLOY_IMAGE_VERSION}"))
    {
        return Err("Cluster queries cannot use host or deployment image placeholders.".into());
    }
    if query.contains("${EXPECTED_VERSION}") && item.expected_version.trim().is_empty() {
        return Err("Enter the expected version for this Query Set item.".into());
    }
    Ok(())
}

#[tauri::command]
pub fn save_deployment_query_set(
    state: State<'_, WorkspaceState>,
    set: QuerySet,
) -> Result<QuerySet, String> {
    save(&state, set)
}
#[tauri::command]
pub fn delete_deployment_query_set(
    state: State<'_, WorkspaceState>,
    id: String,
) -> Result<(), String> {
    state
        .with_connection(|db| {
            if db.execute(
                "DELETE FROM deployment_validation_query_sets WHERE id=?1",
                [id],
            )? != 1
            {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            Ok(())
        })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::workspace::WorkspaceService;
    use serde_json::json;
    #[test]
    fn kafka_preset_references_valid_saved_queries_with_complete_gate_policy() {
        let preset: serde_json::Value = serde_json::from_str(include_str!("presets/kafka-upgrade-health.json")).unwrap();
        let queries: Vec<super::super::validation_queries::ValidationQuery> = serde_json::from_value(preset["queries"].clone()).unwrap();
        let set: QuerySet = serde_json::from_value(preset["querySets"][0].clone()).unwrap();
        set.validate().unwrap();
        assert_eq!(queries.len(),18); assert_eq!(set.items.len(),18);
        for item in &set.items {
            let query = queries.iter().find(|q| q.id == item.query_id).unwrap();
            validate_item(item,&query.query).unwrap();
            assert!(query.query.contains("timestamp("));
        }
        assert_eq!(set.items.iter().filter(|item|item.scope=="node").count(),1);
        assert_eq!(set.stable_samples,3); assert_eq!(set.interval_seconds,30);
        assert_eq!(set.timeout_seconds,1200); assert_eq!(set.max_age_seconds,120);
    }

    #[test]
    fn saves_reloads_and_rejects_stale_edits_missing_queries_and_bad_scopes() {
        let directory = std::env::temp_dir().join(format!("gt-query-set-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let state = WorkspaceState::default();
        WorkspaceService::create(
            &state,
            directory.to_string_lossy().into_owned(),
            "test workspace password",
        )
        .unwrap();
        state.with_connection(|db| db.execute("INSERT INTO deployment_validation_queries(id,name,role,query) VALUES('q','Brokers','','vector(1)')",[])).unwrap();
        let set: QuerySet = serde_json::from_value(
            json!({"name":"Kafka health","items":[{"queryId":"q","scope":"cluster","minResults":1}],
            "stableSamples":3,"intervalSeconds":30,"timeoutSeconds":1200,"maxAgeSeconds":120}),
        )
        .unwrap();
        let first = save(&state, set).unwrap();
        assert_eq!(first.revision, 1);
        assert_eq!(save(&state, first.clone()).unwrap().revision, 2);
        assert!(save(&state, first.clone()).is_err());
        state.lock().unwrap();
        WorkspaceService::unlock(
            &state,
            directory.to_string_lossy().into_owned(),
            "test workspace password",
        )
        .unwrap();
        let loaded = state.with_connection(load).unwrap();
        assert_eq!(loaded[0].items[0].query_id, "q");
        let mut invalid = loaded[0].clone();
        invalid.items[0].scope = "node".into();
        assert!(
            save(&state, invalid)
                .unwrap_err()
                .contains("DEPLOY_HOST_REGEX")
        );
        let mut invalid = loaded[0].clone();
        invalid.items[0].query_id = "deleted".into();
        assert!(save(&state, invalid).is_err());
        let mut invalid = loaded[0].clone();
        invalid.timeout_seconds = 60;
        assert!(invalid.validate().is_err());
        state.lock().unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
