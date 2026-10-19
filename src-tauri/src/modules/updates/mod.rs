use crate::{agent_ipc, modules::workspace::WorkspaceState};
use serde::Serialize;
use std::{
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tauri::{Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Default)]
pub struct UpdateState {
    busy: AtomicBool,
    installing: AtomicBool,
    pending: Mutex<Option<Update>>,
}
struct Operation<'a>(&'a AtomicBool);
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
impl UpdateState {
    pub fn is_installing(&self) -> bool {
        self.installing.load(Ordering::SeqCst)
    }
    fn operation(&self) -> Result<Operation<'_>, String> {
        self.busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| "An update operation is already in progress.".to_string())?;
        Ok(Operation(&self.busy))
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    enabled: bool,
    current_version: String,
    version: Option<String>,
    notes: Option<String>,
}
fn main_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Check for updates from the main window.".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn check_app_update(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, UpdateState>,
) -> Result<UpdateInfo, String> {
    main_window(&window)?;
    let _operation = state.operation()?;
    let current_version = app.package_info().version.to_string();
    if cfg!(debug_assertions) {
        return Ok(UpdateInfo {
            enabled: false,
            current_version,
            version: None,
            notes: None,
        });
    }
    let update = app
        .updater_builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| format!("Could not check for updates: {e}"))?;
    let info = UpdateInfo {
        enabled: true,
        current_version,
        version: update.as_ref().map(|u| u.version.clone()),
        notes: update.as_ref().and_then(|u| u.body.clone()),
    };
    *state
        .pending
        .lock()
        .map_err(|_| "Update state is unavailable.")? = update;
    Ok(info)
}

fn verify_user_installation(executable: &Path, home: &Path) -> Result<(), String> {
    if !executable.starts_with(home) {
        return Err("To update without administrator access, install GateTree-Ops in your user folder (macOS: ~/Applications; Windows: your per-user Apps folder).".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn install_app_update(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, UpdateState>,
    workspace: tauri::State<'_, WorkspaceState>,
    expected_version: String,
) -> Result<(), String> {
    main_window(&window)?;
    if cfg!(debug_assertions) {
        return Err("Application replacement is disabled in debug builds.".into());
    }
    let _operation = state.operation()?;
    state.installing.store(true, Ordering::SeqCst);
    let _installing = Operation(&state.installing);
    let mut update = state
        .pending
        .lock()
        .map_err(|_| "Update state is unavailable.")?
        .clone()
        .ok_or("Check for updates before installing.")?;
    if update.version != expected_version {
        return Err("The available update changed. Review it again.".into());
    }
    let home = app
        .path()
        .home_dir()
        .map_err(|e| e.to_string())?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let executable = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    verify_user_installation(&executable, &home)?;
    update.timeout = Some(Duration::from_secs(300));
    // Reserve BEFORE downloading: this also blocks other GUI instances from starting work.
    let was_running = agent_ipc::prepare_update().map_err(|e| e.to_string())?;
    let progress = |phase: &str, downloaded: u64, total: Option<u64>| {
        let _ = app.emit(
            "gatetree-ops-update-progress",
            serde_json::json!({"phase":phase,"downloaded":downloaded,"total":total}),
        );
    };
    let result: Result<(), String> = async {
        let mut downloaded = 0u64;
        let bytes = update
            .download(
                |chunk, total| {
                    downloaded += chunk as u64;
                    progress("downloading", downloaded, total);
                },
                || {},
            )
            .await
            .map_err(|e| format!("Could not download or verify the signed update: {e}"))?;
        progress("backing-up", downloaded, None);
        let backup = workspace
            .backup_before_update()
            .map_err(|e| e.to_string())?;
        if let Some(path) = backup.as_ref() {
            agent_ipc::backup_update_state(path).map_err(|e| e.to_string())?;
        }
        if was_running {
            agent_ipc::refresh_update_reservation().map_err(|e| e.to_string())?;
            agent_ipc::stop().map_err(|e| e.to_string())?;
        }
        progress("installing", downloaded, None);
        update
            .install(bytes)
            .map_err(|e| format!("Could not install the update: {e}"))?;
        Ok(())
    }
    .await;
    if let Err(error) = result {
        if was_running {
            let _ = agent_ipc::cancel_update();
            // If installation failed after stopping, restore the bundled old Agent.
            if let Err(restart_error) = agent_ipc::start() {
                return Err(format!(
                    "{error} Agent restart also failed: {restart_error}"
                ));
            }
        }
        return Err(error);
    }
    // macOS continues here; Windows installer normally exits and relaunches the GUI.
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn updates_require_user_owned_install_location_and_single_operation() {
        assert!(
            verify_user_installation(
                Path::new("/Users/test/Applications/App.app/Contents/MacOS/app"),
                Path::new("/Users/test")
            )
            .is_ok()
        );
        assert!(
            verify_user_installation(
                Path::new("/Applications/App.app/Contents/MacOS/app"),
                Path::new("/Users/test")
            )
            .is_err()
        );
        assert!(
            verify_user_installation(
                Path::new("/Users/test-other/App.app"),
                Path::new("/Users/test")
            )
            .is_err()
        );
        let state = UpdateState::default();
        let operation = state.operation().unwrap();
        assert!(state.operation().is_err());
        drop(operation);
        assert!(state.operation().is_ok());
    }
}
