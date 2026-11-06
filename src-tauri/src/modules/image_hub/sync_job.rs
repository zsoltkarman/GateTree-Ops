use crate::modules::workspace::WorkspaceState;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::Manager;

static ACTIVE_JOB: Mutex<Option<String>> = Mutex::new(None);
struct Reservation(String);
impl Drop for Reservation {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE_JOB.lock() {
            if active.as_ref() == Some(&self.0) {
                *active = None;
            }
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceProgress {
    pub state: String,
    pub completed: usize,
    pub total: usize,
    pub errors: usize,
    pub detail: String,
}
impl Default for SourceProgress {
    fn default() -> Self {
        Self {
            state: "pending".into(),
            completed: 0,
            total: 0,
            errors: 0,
            detail: String::new(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub project_id: String,
    pub run_id: String,
    pub running: bool,
    pub percent: u8,
    pub scm: SourceProgress,
    pub af: SourceProgress,
    pub slaps: SourceProgress,
    pub error: Option<String>,
    pub updated_at: i64,
}
impl SyncStatus {
    fn new(project_id: &str) -> Self {
        Self {
            project_id: project_id.into(),
            run_id: uuid::Uuid::new_v4().to_string(),
            running: true,
            percent: 0,
            scm: Default::default(),
            af: Default::default(),
            slaps: Default::default(),
            error: None,
            updated_at: super::artifacts::now(),
        }
    }
    fn step_mut(&mut self, source: &str) -> &mut SourceProgress {
        match source {
            "scm" => &mut self.scm,
            "af" => &mut self.af,
            _ => &mut self.slaps,
        }
    }
    fn update(
        &mut self,
        source: &str,
        completed: usize,
        total: usize,
        errors: usize,
        state: &str,
        detail: &str,
    ) {
        *self.step_mut(source) = SourceProgress {
            state: state.into(),
            completed,
            total,
            errors,
            detail: detail.into(),
        };
        let (base, weight) = match source {
            "scm" => (0, 30),
            "af" => (30, 30),
            _ => (60, 40),
        };
        let progress = if matches!(state, "synced" | "error" | "not_configured") {
            weight
        } else if total > 0 {
            weight * completed.min(total) / total
        } else {
            0
        };
        self.percent = self.percent.max((base + progress).min(99) as u8);
        self.updated_at = super::artifacts::now();
    }
}
fn persist(db: &Connection, status: &SyncStatus) -> rusqlite::Result<()> {
    let json = serde_json::to_string(status).map_err(|_| rusqlite::Error::InvalidQuery)?;
    db.execute("INSERT INTO image_hub_sync_state(project_id,status_json) VALUES(?1,?2) ON CONFLICT(project_id) DO UPDATE SET status_json=excluded.status_json",params![status.project_id,json])?;
    Ok(())
}
pub fn load(db: &Connection) -> rusqlite::Result<Vec<SyncStatus>> {
    let active = ACTIVE_JOB
        .lock()
        .map_err(|_| rusqlite::Error::InvalidQuery)?
        .clone();
    load_with_active(db, active.as_deref())
}
fn load_with_active(db: &Connection, active: Option<&str>) -> rusqlite::Result<Vec<SyncStatus>> {
    let mut statement = db.prepare("SELECT status_json FROM image_hub_sync_state")?;
    let rows = statement
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut result = Vec::new();
    for row in rows {
        let mut status: SyncStatus =
            serde_json::from_str(&row).map_err(|_| rusqlite::Error::InvalidQuery)?;
        if status.running && active != Some(status.run_id.as_str()) {
            status.running = false;
            status.error =
                Some("Sync interrupted. Run Sync Project to continue saved work.".into());
            persist(db, &status)?;
        }
        result.push(status);
    }
    Ok(result)
}
pub struct Tracker<'a> {
    pub state: &'a WorkspaceState,
    pub session: &'a str,
    pub status: SyncStatus,
}
impl Tracker<'_> {
    pub fn update(
        &mut self,
        source: &str,
        completed: usize,
        total: usize,
        errors: usize,
        state: &str,
        detail: &str,
    ) -> Result<(), String> {
        self.status
            .update(source, completed, total, errors, state, detail);
        self.save()
    }
    fn save(&self) -> Result<(), String> {
        self.state
            .with_session_connection(self.session, |db| persist(db, &self.status))
            .map_err(|e| e.to_string())
    }
    fn finish(&mut self, error: Option<String>) -> Result<(), String> {
        self.status.running = false;
        if error.is_none() {
            self.status.percent = 100;
        }
        if let Some(message) = &error {
            for source in [
                &mut self.status.scm,
                &mut self.status.af,
                &mut self.status.slaps,
            ] {
                if source.state == "running" {
                    source.state = "error".into();
                    source.detail = message.clone();
                }
            }
        }
        self.status.error = error;
        self.status.updated_at = super::artifacts::now();
        self.save()
    }
}
#[tauri::command]
pub fn sync_image_hub_project(app: tauri::AppHandle, id: String) -> Result<SyncStatus, String> {
    let state = app.state::<WorkspaceState>();
    let session = state.session_id().map_err(|e| e.to_string())?;
    let initial = SyncStatus::new(&id);
    let reservation = {
        let mut active = ACTIVE_JOB
            .lock()
            .map_err(|_| "Sync state is unavailable.")?;
        if active.is_some() {
            return Err("Another project sync is running in the background.".into());
        }
        *active = Some(initial.run_id.clone());
        Reservation(initial.run_id.clone())
    };
    state
        .with_session_connection(&session, |db| {
            super::service::project(db, &id)?;
            persist(db, &initial)
        })
        .map_err(|e| e.to_string())?;
    let result = initial.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _reservation = reservation;
        let state = app.state::<WorkspaceState>();
        let mut tracker = Tracker {
            state: &state,
            session: &session,
            status: initial,
        };
        let operation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::commands::run_image_hub_sync(&app, &id, &session, &mut tracker)
        }));
        let error = match operation {
            Ok(Ok(_)) => None,
            Ok(Err(error)) => Some(error),
            Err(_) => {
                super::project_sync::SYNC_LOCK.clear_poison();
                Some("The background sync worker stopped unexpectedly. Run Sync Project to continue saved work.".into())
            }
        };
        let _ = tracker.finish(error);
    });
    Ok(result)
}

/// Reserve the existing single sync worker for the whole inventory. Each
/// project keeps its own persisted counters while sharing the batch run ID.
#[tauri::command]
pub fn sync_image_hub_inventory(app: tauri::AppHandle) -> Result<Vec<SyncStatus>, String> {
    let state = app.state::<WorkspaceState>();
    let session = state.session_id().map_err(|error| error.to_string())?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let reservation = {
        let mut active = ACTIVE_JOB
            .lock()
            .map_err(|_| "Sync state is unavailable.")?;
        if active.is_some() {
            return Err("Another project sync is running in the background.".into());
        }
        *active = Some(run_id.clone());
        Reservation(run_id.clone())
    };
    let initial = state
        .with_session_connection(&session, |db| prepare_inventory_sync(db, &run_id))
        .map_err(|error| error.to_string())?;
    if initial.is_empty() {
        return Err(
            "Add a project under Image Hub → Config → Projects before syncing inventory.".into(),
        );
    }
    let result = initial.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _reservation = reservation;
        let state = app.state::<WorkspaceState>();
        for status in initial {
            let id = status.project_id.clone();
            let mut tracker = Tracker {
                state: &state,
                session: &session,
                status,
            };
            let operation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                super::commands::run_image_hub_sync(&app, &id, &session, &mut tracker)
            }));
            let error = match operation {
                Ok(Ok(_)) => None,
                Ok(Err(error)) => Some(error),
                Err(_) => {
                    super::project_sync::SYNC_LOCK.clear_poison();
                    Some("The project sync worker stopped unexpectedly. Retry Sync Project to continue saved work.".into())
                }
            };
            if tracker.finish(error).is_err() {
                break;
            } // Workspace closed/changed.
        }
    });
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_is_monotonic_and_does_not_report_completion_before_the_worker_finishes() {
        let mut status = SyncStatus::new("project");
        status.update("scm", 3, 4, 1, "running", "");
        assert_eq!(status.percent, 22);
        status.update("scm", 3, 10, 1, "running", "");
        assert_eq!(status.percent, 22);
        status.update("scm", 4, 4, 1, "error", "one failed");
        assert_eq!(status.percent, 30);
        status.update("af", 0, 0, 0, "not_configured", "");
        assert_eq!(status.percent, 60);
        status.update("slaps", 9, 10, 0, "running", "");
        assert_eq!(status.percent, 96);
        status.update("slaps", 10, 10, 0, "synced", "");
        assert_eq!(status.percent, 99);
        assert!(status.running);
    }
    #[test]
    fn interrupted_sync_state_is_recovered_from_sql() {
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage::migrations::run_migrations(&mut db).unwrap();
        db.execute("INSERT INTO image_hub_projects(id,name,local_path,project_type,source) VALUES('project','Test','/tmp/test','scm','')",[]).unwrap();
        let mut status = SyncStatus::new("project");
        status.percent = 45;
        persist(&db, &status).unwrap();
        let loaded = load(&db).unwrap();
        assert!(!loaded[0].running);
        assert_eq!(loaded[0].percent, 45);
        assert!(loaded[0].error.as_ref().unwrap().contains("interrupted"));
    }
}

fn prepare_inventory_sync(db: &Connection, run_id: &str) -> rusqlite::Result<Vec<SyncStatus>> {
    let mut statement =
        db.prepare("SELECT id FROM image_hub_projects ORDER BY name COLLATE NOCASE,id")?;
    let ids = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let statuses: Vec<_> = ids
        .iter()
        .map(|id| {
            let mut status = SyncStatus::new(id);
            status.run_id = run_id.to_string();
            status
        })
        .collect();
    let transaction = db.unchecked_transaction()?;
    for status in &statuses {
        persist(&transaction, status)?;
    }
    transaction.commit()?;
    Ok(statuses)
}

#[cfg(test)]
mod inventory_tests {
    use super::*;
    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE image_hub_projects(id TEXT PRIMARY KEY,name TEXT); CREATE TABLE image_hub_sync_state(project_id TEXT PRIMARY KEY,status_json TEXT NOT NULL); INSERT INTO image_hub_projects VALUES('b','Bravo'),('a','Alpha');").unwrap();
        db
    }
    #[test]
    fn inventory_batch_is_ordered_and_queued_projects_share_the_active_reservation() {
        let db = database();
        let initial = prepare_inventory_sync(&db, "batch").unwrap();
        assert_eq!(
            initial
                .iter()
                .map(|status| status.project_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert!(
            initial
                .iter()
                .all(|status| status.running && status.run_id == "batch")
        );
        let running = load_with_active(&db, Some("batch")).unwrap();
        assert_eq!(running.len(), 2);
        assert!(
            running
                .iter()
                .all(|status| status.running && status.error.is_none()),
            "queued batch members must not be marked interrupted"
        );
        let stopped = load_with_active(&db, None).unwrap();
        assert!(stopped.iter().all(|status| {
            !status.running
                && status
                    .error
                    .as_deref()
                    .is_some_and(|message| message.contains("interrupted"))
        }));
        assert!(
            load_with_active(&db, Some("another"))
                .unwrap()
                .iter()
                .all(|status| !status.running)
        );
    }
    #[test]
    fn preparing_a_batch_is_atomic_if_any_project_status_cannot_be_saved() {
        let db = database();
        prepare_inventory_sync(&db, "original").unwrap();
        db.execute_batch("CREATE TRIGGER reject_update BEFORE UPDATE ON image_hub_sync_state WHEN NEW.project_id='b' BEGIN SELECT RAISE(ABORT,'failed'); END;").unwrap();
        assert!(prepare_inventory_sync(&db, "replacement").is_err());
        assert!(
            load_with_active(&db, Some("original"))
                .unwrap()
                .iter()
                .all(|status| status.run_id == "original" && status.running)
        );
    }
    #[test]
    fn an_empty_inventory_does_not_create_placeholder_project_statuses() {
        let db = database();
        db.execute("DELETE FROM image_hub_projects", []).unwrap();
        assert!(prepare_inventory_sync(&db, "empty").unwrap().is_empty());
        assert!(load_with_active(&db, Some("empty")).unwrap().is_empty());
    }
}
