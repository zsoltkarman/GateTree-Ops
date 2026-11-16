use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;
use uuid::Uuid;

use crate::modules::workspace::WorkspaceState;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentTask {
    pub id: String,
    pub jira_key: String,
    pub name: String,
    pub revision: i64,
    pub items: Vec<TaskItem>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskItem {
    pub id: String,
    pub role: String,
    pub plan_id: Option<String>,
}

fn invalid(message: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(message.into())
}

fn load(connection: &Connection) -> rusqlite::Result<Vec<DeploymentTask>> {
    let mut statement = connection.prepare(
        "SELECT id,jira_key,name,revision FROM deployment_tasks ORDER BY created_at DESC,id",
    )?;
    let mut tasks = statement
        .query_map([], |row| {
            Ok(DeploymentTask {
                id: row.get(0)?,
                jira_key: row.get(1)?,
                name: row.get(2)?,
                revision: row.get(3)?,
                items: vec![],
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut items = connection.prepare("SELECT id,role,plan_id FROM deployment_task_items WHERE task_id=?1 ORDER BY position,id")?;
    for task in &mut tasks {
        task.items = items
            .query_map([&task.id], |row| {
                Ok(TaskItem {
                    id: row.get(0)?,
                    role: row.get(1)?,
                    plan_id: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
    }
    Ok(tasks)
}

fn save(connection: &Connection, mut task: DeploymentTask) -> rusqlite::Result<String> {
    task.jira_key = task.jira_key.trim().to_uppercase();
    task.name = task.name.trim().into();
    if task.name.is_empty() { task.name = task.jira_key.clone(); }
    let valid_key = task
        .jira_key
        .rsplit_once('-')
        .is_some_and(|(project, number)| {
            !project.is_empty()
                && project.starts_with(|c: char| c.is_ascii_alphabetic())
                && project
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !number.is_empty()
                && number.chars().all(|c| c.is_ascii_digit())
        });
    if !valid_key || task.jira_key.len() > 100 || task.name.is_empty() || task.name.len() > 200 {
        return Err(invalid(
                "Enter a valid Jira key. Task names must not exceed 200 characters.",
        ));
    }
    if task.items.is_empty() || task.items.len() > 100 {
        return Err(invalid("A Task needs between 1 and 100 services."));
    }
    let mut roles = std::collections::HashSet::new();
    let mut ids = std::collections::HashSet::new();
    for item in &mut task.items {
        item.role = item.role.trim().into();
        if item.role.is_empty()
            || item.role.len() > 200
            || !roles.insert(item.role.clone())
        {
            return Err(invalid(
                "Every service needs a unique role.",
            ));
        }
        if item.id.is_empty() {
            item.id = Uuid::new_v4().to_string();
        }
        if !ids.insert(item.id.clone()) {
            return Err(invalid("Duplicate service ID."));
        }
    }
    let tx = connection.unchecked_transaction()?;
    if task.id.is_empty() {
        task.id = Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO deployment_tasks(id,jira_key,name) VALUES(?1,?2,?3)",
            params![task.id, task.jira_key, task.name],
        )?;
    } else {
        let existing = load(&tx)?
            .into_iter()
            .find(|entry| entry.id == task.id)
            .ok_or_else(|| invalid("Task no longer exists."))?;
        if existing.items.iter().any(|item| item.plan_id.is_some())
            && existing.jira_key != task.jira_key
        {
            return Err(invalid("Unlink Plans before changing the Jira key."));
        }
        for item in existing.items.iter().filter(|item| item.plan_id.is_some()) {
            if !task.items.iter().any(|next| {
                next.id == item.id
                    && next.role == item.role
            }) {
                return Err(invalid(
                    "Unlink the Plan before changing or removing its role.",
                ));
            }
        }
        if tx.execute("UPDATE deployment_tasks SET jira_key=?2,name=?3,revision=revision+1 WHERE id=?1 AND revision=?4", params![task.id,task.jira_key,task.name,task.revision])? != 1 {
            return Err(invalid("Task changed. Refresh before editing again."));
        }
    }
    // Preserve links server-side; submitted plan IDs never create or change links.
    for (position, item) in task.items.iter().enumerate() {
        let owner: Option<String> = tx
            .query_row(
                "SELECT task_id FROM deployment_task_items WHERE id=?1",
                [&item.id],
                |r| r.get(0),
            )
            .optional()?;
        if owner.as_ref().is_some_and(|id| id != &task.id) {
            return Err(invalid("Service belongs to another Task."));
        }
        // Keep the legacy version column for existing workspaces/backups; Tasks no longer use it.
        tx.execute("INSERT INTO deployment_task_items(id,task_id,role,target_version,position) VALUES(?1,?2,?3,'',?4)
            ON CONFLICT(id) DO UPDATE SET role=excluded.role,position=excluded.position",
            params![item.id,task.id,item.role,position as i64])?;
    }
    let existing_ids: Vec<String> = {
        let mut query = tx.prepare("SELECT id FROM deployment_task_items WHERE task_id=?1")?;
        query
            .query_map([&task.id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    for id in existing_ids.into_iter().filter(|id| !ids.contains(id)) {
        tx.execute("DELETE FROM deployment_task_items WHERE id=?1", [id])?;
    }
    tx.commit()?;
    Ok(task.id)
}

fn validate_definition(
    connection: &Connection,
    item_id: &str,
    definition: &str,
) -> rusqlite::Result<()> {
    let (jira, role): (String, String) = connection.query_row(
        "SELECT t.jira_key,i.role FROM deployment_task_items i JOIN deployment_tasks t ON t.id=i.task_id WHERE i.id=?1",
        [item_id], |row| Ok((row.get(0)?,row.get(1)?)),
    )?;
    let plan: Value =
        serde_yaml::from_str(definition).map_err(|_| invalid("Invalid Plan YAML."))?;
    let inputs = &plan["inputs"];
    let deployment = inputs
        .get("ansible")
        .or_else(|| inputs.get("olam"))
        .or_else(|| inputs.get("container"))
        .ok_or_else(|| invalid("Plan has no deployment inputs."))?;
    if !inputs["jira"]["issue"]
        .as_str()
        .is_some_and(|key| key.eq_ignore_ascii_case(&jira))
    {
        return Err(invalid("Plan Jira must match the Task Jira."));
    }
    if deployment["targets"]["roles"]
        .as_array()
        .is_none_or(|roles| roles.len() != 1 || roles[0].as_str() != Some(&role))
    {
        return Err(invalid("Plan role must match the Task service."));
    }
    let has_deploy = deployment["steps"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|step| step["type"].as_str().unwrap_or("deploy") == "deploy");
    if !has_deploy {
        return Err(invalid("Plan must contain a Deploy step."));
    }
    Ok(())
}

pub(super) fn link(connection: &Connection, item_id: &str, plan_id: &str) -> rusqlite::Result<()> {
    let current: Option<String> = connection.query_row(
        "SELECT plan_id FROM deployment_task_items WHERE id=?1",
        [item_id],
        |r| r.get(0),
    )?;
    if current.as_deref().is_some_and(|id| id != plan_id) {
        return Err(invalid("Service already has a Plan. Unlink it first."));
    }
    let definition: String = connection.query_row(
        "SELECT definition_yaml FROM deployment_plans WHERE id=?1",
        [plan_id],
        |r| r.get(0),
    )?;
    validate_definition(connection, item_id, &definition)?;
    connection.execute(
        "UPDATE deployment_task_items SET plan_id=?2 WHERE id=?1",
        params![item_id, plan_id],
    )?;
    connection.execute("UPDATE deployment_tasks SET revision=revision+1 WHERE id=(SELECT task_id FROM deployment_task_items WHERE id=?1)", [item_id])?;
    Ok(())
}

pub(super) fn validate_plan_update(
    connection: &Connection,
    plan_id: &str,
    definition: &str,
) -> rusqlite::Result<()> {
    let item: Option<String> = connection
        .query_row(
            "SELECT id FROM deployment_task_items WHERE plan_id=?1",
            [plan_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(item) = item {
        validate_definition(connection, &item, definition)?;
    }
    Ok(())
}

#[tauri::command]
pub fn get_deployment_tasks(
    state: State<'_, WorkspaceState>,
) -> Result<Vec<DeploymentTask>, String> {
    state
        .with_connection(load)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn save_deployment_task(
    state: State<'_, WorkspaceState>,
    task: DeploymentTask,
) -> Result<String, String> {
    state
        .with_connection(|connection| save(connection, task))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn link_deployment_task_plan(
    state: State<'_, WorkspaceState>,
    item_id: String,
    plan_id: Option<String>,
) -> Result<(), String> {
    state.with_connection(|connection| {
        let tx = connection.unchecked_transaction()?;
        if let Some(plan_id) = plan_id { link(&tx, &item_id, &plan_id)?; }
        else {
            if tx.execute("UPDATE deployment_task_items SET plan_id=NULL WHERE id=?1", [&item_id])? != 1 { return Err(rusqlite::Error::QueryReturnedNoRows); }
            tx.execute("UPDATE deployment_tasks SET revision=revision+1 WHERE id=(SELECT task_id FROM deployment_task_items WHERE id=?1)", [&item_id])?;
        }
        tx.commit()
    }).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_deployment_task(
    state: State<'_, WorkspaceState>,
    id: String,
    revision: i64,
) -> Result<(), String> {
    state
        .with_connection(|connection| {
            if connection.execute(
                "DELETE FROM deployment_tasks WHERE id=?1 AND revision=?2",
                params![id, revision],
            )? != 1
            {
                return Err(invalid("Task changed. Refresh before deleting it."));
            }
            Ok(())
        })
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        modules::{deployments::queue_service::QueueService, workspace::WorkspaceService},
        storage::migrations::run_migrations,
    };

    fn database() -> Connection {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_migrations(&mut db).unwrap();
        db
    }

    fn new_task(key: &str) -> DeploymentTask {
        DeploymentTask {
            id: String::new(),
            jira_key: key.into(),
            name: "STG rollout".into(),
            revision: 0,
            items: vec![TaskItem {
                id: String::new(),
                role: "ping_tool".into(),
                plan_id: None,
            }],
        }
    }

    fn definition() -> String {
        "plan_name: Ping rollout\nprocess: Deploy\ninputs:\n  jira: {issue: GPDM-25741}\n  ansible:\n    strategy: linear\n    account: deployment\n    targets: {roles: [ping_tool], hosts: [consumer-1]}\n    steps: [{name: Deploy, extra_vars: {IMAGE_VERSION: '2026.11.2'}}]\n".into()
    }

    fn insert_plan(db: &Connection, id: &str, number: i64, definition: &str) {
        db.execute("INSERT INTO deployment_plans(id,plan_number,name,process_name,definition_yaml) VALUES(?1,?2,'Deploy','Deploy',?3)", params![id,number,definition]).unwrap();
    }

    #[test]
    fn task_edits_are_validated_versioned_and_persisted() {
        let db = database();
        save(&db, new_task(" gpdm-25741 ")).unwrap();
        let mut task = load(&db).unwrap().remove(0);
        assert_eq!(task.jira_key, "GPDM-25741");
        assert_eq!(task.revision, 1);
        assert!(save(&db, new_task("GPDM-25741")).is_err());
        assert!(save(&db, new_task("not a jira")).is_err());
        let stale: DeploymentTask =
            serde_json::from_value(serde_json::to_value(&task).unwrap()).unwrap();
        task.name = "Updated rollout".into();
        save(&db, task).unwrap();
        assert!(save(&db, stale).is_err());
        let mut task = load(&db).unwrap().remove(0);
        assert_eq!(task.name, "Updated rollout");
        task.items.push(TaskItem {
            id: String::new(),
            role: "ping_tool".into(),
            plan_id: None,
        });
        assert!(save(&db, task).is_err());
        assert_eq!(load(&db).unwrap()[0].items.len(), 1);
    }

    #[test]
    fn tasks_need_only_jira_and_roles_and_ignore_legacy_versions() {
        let db = database();
        let task: DeploymentTask = serde_json::from_value(serde_json::json!({
            "id":"", "jiraKey":"GPDM-25741", "name":"", "revision":0,
            "items":[{"id":"", "role":"ping_tool", "planId":null}]
        })).unwrap();
        save(&db, task).unwrap();
        let task = load(&db).unwrap().remove(0);
        assert_eq!(task.name, "GPDM-25741");
        db.execute("UPDATE deployment_task_items SET target_version='legacy-version' WHERE id=?1", [&task.items[0].id]).unwrap();
        let inherited = definition().replace(", extra_vars: {IMAGE_VERSION: '2026.11.2'}", "");
        insert_plan(&db, "inherited", 1, &inherited);
        link(&db, &task.items[0].id, "inherited").unwrap();
        let loaded = load(&db).unwrap().remove(0);
        assert_eq!(loaded.items[0].plan_id.as_deref(), Some("inherited"));
        assert!(serde_json::to_value(&loaded).unwrap()["items"][0].get("targetVersion").is_none());
        save(&db, loaded).unwrap();
    }

    #[test]
    fn links_require_matching_jira_and_role_without_overriding_ansible_versions() {
        let db = database();
        save(&db, new_task("GPDM-25741")).unwrap();
        let task = load(&db).unwrap().remove(0);
        let item_id = &task.items[0].id;
        insert_plan(&db, "plan", 1, &definition());
        for yaml in [
            definition().replace("GPDM-25741", "GPDM-1"),
            definition().replace("ping_tool", "acg"),
        ] {
            assert!(validate_definition(&db, item_id, &yaml).is_err(), "{yaml}");
        }
        link(&db, item_id, "plan").unwrap();
        validate_plan_update(&db, "plan", &definition().replace("ansible:", "olam:")).unwrap();
        for yaml in [
            definition().replace(", extra_vars: {IMAGE_VERSION: '2026.11.2'}", ""),
            definition().replace("2026.11.2", "other"),
        ] {
            validate_plan_update(&db, "plan", &yaml).unwrap();
        }
        let mut edited = load(&db).unwrap().remove(0);
        edited.items[0].role = "new_role".into();
        assert!(save(&db, edited).is_err());
        assert_eq!(load(&db).unwrap()[0].items[0].role, "ping_tool");
        // A Plan belongs to only one service; neither invalid links nor deletion run it.
        save(&db, new_task("GPDM-2")).unwrap();
        let other = load(&db)
            .unwrap()
            .into_iter()
            .find(|task| task.jira_key == "GPDM-2")
            .unwrap();
        assert!(link(&db, &other.items[0].id, "plan").is_err());
        db.execute("DELETE FROM deployment_tasks WHERE id=?1", [&task.id])
            .unwrap();
        assert_eq!(
            db.query_row(
                "SELECT status FROM deployment_plans WHERE id='plan'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "queued"
        );
    }

    #[test]
    fn migration_and_plan_deletion_preserve_tasks_and_clear_stale_links() {
        let mut db = database();
        save(&db, new_task("GPDM-25741")).unwrap();
        insert_plan(&db, "plan", 1, &definition());
        let task = load(&db).unwrap().remove(0);
        link(&db, &task.items[0].id, "plan").unwrap();
        run_migrations(&mut db).unwrap();
        assert_eq!(
            load(&db).unwrap()[0].items[0].plan_id.as_deref(),
            Some("plan")
        );
        db.execute("DELETE FROM deployment_plans WHERE id='plan'", [])
            .unwrap();
        let task = load(&db).unwrap().remove(0);
        assert_eq!(task.items.len(), 1);
        assert_eq!(task.items[0].plan_id, None);
    }

    #[test]
    fn creating_a_linked_plan_is_atomic_and_keeps_standard_plans_compatible() {
        let directory = std::env::temp_dir().join(format!("gt-tasks-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let state = WorkspaceState::default();
        WorkspaceService::create(
            &state,
            directory.to_string_lossy().into_owned(),
            "test workspace password",
        )
        .unwrap();
        let item_id = state.with_connection(|db| {
            save(db, new_task("GPDM-25741"))?;
            db.execute("INSERT INTO deployment_processes(id,name,definition_json) VALUES('process','Deploy',?1)", [serde_json::json!({"steps":[{"name":"Deploy","driver":"olam","action":"deploy","workflow":["approve","deploy"]}],"details":{"olam":{"account":"deployment","payload_defaults":{"ENVIRONMENT":"test"}}}}).to_string()])?;
            db.execute("INSERT INTO vault_entries(id,name,entry_path) VALUES('account','deployment','Accounts/deployment')", [])?;
            Ok(load(db)?.remove(0).items.remove(0).id)
        }).unwrap();
        assert!(
            QueueService::create_for_task(
                &state,
                definition().replace("GPDM-25741", "GPDM-1"),
                Some(&item_id)
            )
            .is_err()
        );
        assert!(
            QueueService::summary_snapshot(&state)
                .unwrap()
                .plans
                .is_empty()
        );
        let inherited = definition().replace(", extra_vars: {IMAGE_VERSION: '2026.11.2'}", "");
        let linked = QueueService::create_for_task(&state, inherited.clone(), Some(&item_id)).unwrap();
        let snapshot = QueueService::summary_snapshot(&state).unwrap();
        assert_eq!(snapshot.plans[0].status, "queued");
        assert_eq!(snapshot.plans[0].plan_number, 1);
        assert!(
            QueueService::update(&state, &linked, definition().replace("GPDM-25741", "GPDM-1"))
                .is_err()
        );
        QueueService::update(&state, &linked, inherited).unwrap();
        assert!(QueueService::create_for_task(&state, definition(), Some(&item_id)).is_err());
        QueueService::create(&state, definition().replace("2026.11.2", "other")).unwrap();
        assert_eq!(
            QueueService::summary_snapshot(&state).unwrap().plans.len(),
            2
        );
        drop(state);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
