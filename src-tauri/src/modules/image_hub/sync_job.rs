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
    let mut statement = db.prepare("SELECT status_json FROM image_hub_sync_state")?;
    let rows = statement
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let active = ACTIVE_JOB
        .lock()
        .map_err(|_| rusqlite::Error::InvalidQuery)?
        .clone();
    let mut result = Vec::new();
    for row in rows {
        let mut status: SyncStatus =
            serde_json::from_str(&row).map_err(|_| rusqlite::Error::InvalidQuery)?;
        if status.running && active.as_ref() != Some(&status.run_id) {
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
