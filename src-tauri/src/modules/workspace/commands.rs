use tauri::State;
use zeroize::Zeroize;

use crate::platform::biometric;

use super::{
    model::{BiometricStatusView, WorkspaceInspection, WorkspaceSessionView},
    service::{WorkspaceService, WorkspaceState},
};

#[tauri::command]
pub fn inspect_workspace(path: String) -> Result<WorkspaceInspection, String> {
    WorkspaceService::inspect(path).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn create_workspace(
    state: State<'_, WorkspaceState>,
    path: String,
    mut password: String,
) -> Result<WorkspaceSessionView, String> {
    let result =
        WorkspaceService::create(&state, path, &password).map_err(|error| error.to_string());
    #[cfg(debug_assertions)]
    if result.is_ok() {
        let _ = WorkspaceService::cache_development_key(&state);
    }
    password.zeroize();
    result
}

#[tauri::command]
pub fn unlock_workspace(
    state: State<'_, WorkspaceState>,
    path: String,
    mut password: String,
) -> Result<WorkspaceSessionView, String> {
    let result =
        WorkspaceService::unlock(&state, path, &password).map_err(|error| error.to_string());
    #[cfg(debug_assertions)]
    if result.is_ok() {
        let _ = WorkspaceService::cache_development_key(&state);
    }
    password.zeroize();
    result
}

#[tauri::command]
pub fn lock_workspace(
    state: State<'_, WorkspaceState>,
    terminals: State<'_, crate::modules::local_terminal::LocalTerminals>,
) -> Result<(), String> {
    terminals.clear();
    state.lock().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn unlock_workspace_for_development(
    state: State<'_, WorkspaceState>,
    path: String,
) -> Result<WorkspaceSessionView, String> {
    #[cfg(debug_assertions)]
    {
        return WorkspaceService::unlock_for_development(&state, path)
            .map_err(|error| error.to_string());
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = (state, path);
        Err("Development unlock is disabled in release builds.".to_owned())
    }
}

#[tauri::command]
pub async fn biometric_status(path: String) -> Result<BiometricStatusView, String> {
    let inspection = WorkspaceService::inspect(path).map_err(|error| error.to_string())?;
    let status = biometric::status(std::path::Path::new(&inspection.path))
        .await
        .map_err(|error| error.to_string())?;
    Ok(BiometricStatusView {
        available: status.available,
        configured: status.configured,
        label: status.label,
    })
}

#[tauri::command]
pub async fn enable_biometric_unlock(
    state: State<'_, WorkspaceState>,
) -> Result<BiometricStatusView, String> {
    let (path, key) = state
        .biometric_material()
        .map_err(|error| error.to_string())?;
    biometric::store(&path, &key)
        .await
        .map_err(|error| error.to_string())?;
    let status = biometric::status(&path)
        .await
        .map_err(|error| error.to_string())?;
    Ok(BiometricStatusView {
        available: status.available,
        configured: status.configured,
        label: status.label,
    })
}

#[tauri::command]
pub async fn disable_biometric_unlock(path: String) -> Result<(), String> {
    let inspection = WorkspaceService::inspect(path).map_err(|error| error.to_string())?;
    biometric::delete(std::path::Path::new(&inspection.path))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn unlock_workspace_with_biometrics(
    state: State<'_, WorkspaceState>,
    path: String,
) -> Result<WorkspaceSessionView, String> {
    let inspection = WorkspaceService::inspect(path).map_err(|error| error.to_string())?;
    let mut key = biometric::retrieve(std::path::Path::new(&inspection.path))
        .await
        .map_err(|error| error.to_string())?;
    let result = WorkspaceService::unlock_with_key(&state, inspection.path, key.clone())
        .map_err(|error| error.to_string());
    #[cfg(debug_assertions)]
    if result.is_ok() {
        let _ = WorkspaceService::cache_development_key(&state);
    }
    key.zeroize();
    result
}
