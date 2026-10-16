use std::{fs, path::Path};

use tauri::State;

use crate::modules::workspace::WorkspaceState;

use super::{model::BackupSummary, service::BackupService};

#[tauri::command]
pub fn export_backup(
    state: State<'_, WorkspaceState>,
    path: String,
) -> Result<BackupSummary, String> {
    let (json, summary) = BackupService::export_json(&state).map_err(|error| error.to_string())?;
    fs::write(&path, json).map_err(|error| format!("Could not write backup: {error}"))?;
    restrict_backup_permissions(Path::new(&path))?;
    Ok(summary)
}

#[tauri::command]
pub fn import_backup(
    state: State<'_, WorkspaceState>,
    path: String,
) -> Result<BackupSummary, String> {
    let json =
        fs::read_to_string(path).map_err(|error| format!("Could not read backup: {error}"))?;
    BackupService::import_json(&state, &json).map_err(|error| error.to_string())
}

#[cfg(unix)]
fn restrict_backup_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("Could not protect backup: {error}"))
}

#[cfg(not(unix))]
fn restrict_backup_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}
