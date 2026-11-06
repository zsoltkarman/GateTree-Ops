use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::modules::workspace::WorkspaceState;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationQuery {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub query: String,
    #[serde(default)]
    pub revision: i64,
}

pub(super) fn load(connection: &Connection) -> rusqlite::Result<Vec<ValidationQuery>> {
    connection.prepare("SELECT id,name,query,revision FROM deployment_validation_queries ORDER BY name COLLATE NOCASE")?
        .query_map([], |row| Ok(ValidationQuery {
            id: row.get(0)?, name: row.get(1)?, query: row.get(2)?, revision: row.get(3)?,
        }))?.collect()
}

fn save(state: &WorkspaceState, mut query: ValidationQuery) -> Result<ValidationQuery, String> {
    query.name = query.name.trim().into();
    query.query = query.query.trim().into();
    if query.name.is_empty() || query.name.len() > 160 {
        return Err("Enter a query name (maximum 160 bytes).".into());
    }
    super::queue_service::validate_thanos_query_template(&query.query)?;
    let creating = query.id.is_empty();
    if creating {
        query.id = Uuid::new_v4().to_string();
    }
    state.with_connection(|connection| {
        let duplicate: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM deployment_validation_queries WHERE name=?1 COLLATE NOCASE AND id<>?2)",params![query.name,query.id],|row|row.get(0))?;
        if duplicate { return Err(rusqlite::Error::InvalidParameterName("A validation query with this name already exists.".into())); }
        if creating {
            connection.execute("INSERT INTO deployment_validation_queries(id,name,role,query) VALUES(?1,?2,'',?3)",params![query.id,query.name,query.query])?;
            query.revision = 1;
        } else {
            let updated = connection.execute("UPDATE deployment_validation_queries SET name=?2,query=?3,revision=revision+1,updated_at=CURRENT_TIMESTAMP WHERE id=?1 AND revision=?4",params![query.id,query.name,query.query,query.revision])?;
            if updated != 1 { return Err(rusqlite::Error::InvalidParameterName("This query changed or was deleted. Reopen Config before editing it again.".into())); }
            query.revision += 1;
        }
        Ok(())
    }).map_err(|error| error.to_string())?;
    Ok(query)
}

fn delete(state: &WorkspaceState, id: &str) -> Result<(), String> {
    state
        .with_connection(|connection| {
            let deleted = connection.execute(
                "DELETE FROM deployment_validation_queries WHERE id=?1",
                [id],
            )?;
            if deleted != 1 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            Ok(())
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn save_deployment_validation_query(
    state: State<'_, WorkspaceState>,
    query: ValidationQuery,
) -> Result<ValidationQuery, String> {
    save(&state, query)
}

#[tauri::command]
pub fn delete_deployment_validation_query(
    state: State<'_, WorkspaceState>,
    id: String,
) -> Result<(), String> {
    delete(&state, &id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::workspace::WorkspaceService;

    #[test]
    fn queries_persist_across_unlock_and_edits_reject_stale_revisions() {
        let directory =
            std::env::temp_dir().join(format!("gt-validation-queries-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let state = WorkspaceState::default();
        let password = "test workspace password";
        WorkspaceService::create(&state, directory.to_string_lossy().into_owned(), password)
            .unwrap();
        let draft = ValidationQuery { id:String::new(),name:"Exporter version".into(),
            query:r#"metric{instance=~"${DEPLOY_HOST_REGEX}",region="${DEPLOY_REGION}",version="${EXPECTED_VERSION}"}"#.into(),revision:0 };
        let saved = save(&state, draft.clone()).unwrap();
        assert_eq!(saved.revision, 1);
        // Existing workspace rows may still have legacy role metadata. It is
        // neither required for new queries nor exposed by the current API.
        state.with_connection(|db| db.execute("UPDATE deployment_validation_queries SET role='legacy_role' WHERE id=?1",[&saved.id])).unwrap();
        let snapshot = super::super::service::DeploymentsService::snapshot(&state).unwrap();
        let snapshot_json = serde_json::to_value(snapshot).unwrap();
        assert_eq!(snapshot_json["validationQueries"][0]["query"], saved.query);
        let bundle = super::super::service::DeploymentsService::bundle(&state).unwrap();
        let bundle: serde_json::Value = serde_json::from_str(&bundle).unwrap();
        assert!(bundle["validation_queries"][0].get("role").is_none());
        let mut duplicate = draft.clone();
        duplicate.name = "EXPORTER VERSION".into();
        assert!(
            save(&state, duplicate)
                .unwrap_err()
                .contains("already exists")
        );
        let mut changed = saved.clone();
        changed.query.push_str(" > bool 0");
        let updated = save(&state, changed).unwrap();
        assert_eq!(updated.revision, 2);
        assert!(
            save(&state, saved)
                .unwrap_err()
                .contains("changed or was deleted")
        );
        let mut invalid = draft.clone();
        invalid.name = "Invalid".into();
        invalid.query = r#"metric{version="${UNKNOWN}"}"#.into();
        assert!(save(&state, invalid).is_err());
        state.lock().unwrap();
        WorkspaceService::unlock(&state, directory.to_string_lossy().into_owned(), password)
            .unwrap();
        let loaded = state.with_connection(load).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].query, updated.query);
        assert_eq!(loaded[0].revision, 2);
        delete(&state, &updated.id).unwrap();
        assert!(state.with_connection(load).unwrap().is_empty());
        state.lock().unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
