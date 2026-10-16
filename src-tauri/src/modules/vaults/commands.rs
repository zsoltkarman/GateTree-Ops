use tauri::State;

use crate::modules::workspace::WorkspaceState;

use super::{
    keepass::{read_keepass_entry, validate_keepass_database},
    model::{VaultEntry, VaultSnapshot},
    service::VaultsService,
};

#[tauri::command]
pub fn get_vaults(state: State<'_, WorkspaceState>) -> Result<VaultSnapshot, String> {
    VaultsService::snapshot(&state).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_keepass_database_path(
    state: State<'_, WorkspaceState>,
    path: String,
) -> Result<String, String> {
    VaultsService::set_database_path(&state, path).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn validate_and_set_keepass_database_path(
    state: State<'_, WorkspaceState>,
    path: String,
    master_password: String,
) -> Result<String, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("Choose a KeePass database first.".into());
    }
    if master_password.is_empty() {
        return Err("KeePass master password is required.".into());
    }
    validate_keepass_database(path, &master_password)?;

    let entries = VaultsService::snapshot(&state)
        .map_err(|error| error.to_string())?
        .entries;
    let mut failures = Vec::new();
    for entry in &entries {
        if let Err(error) = read_keepass_entry(path, &entry.entry_path, &master_password) {
            failures.push(format!("{} ({}): {}", entry.name, entry.entry_path, error));
        }
    }
    if !failures.is_empty() {
        return Err(format!(
            "KeePass validation failed for {} of {} entries:\n{}",
            failures.len(),
            entries.len(),
            failures.join("\n")
        ));
    }

    VaultsService::set_database_path(&state, path.to_owned()).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn clear_keepass_database_path(state: State<'_, WorkspaceState>) -> Result<(), String> {
    VaultsService::clear_database_path(&state).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn create_vault_entry(
    state: State<'_, WorkspaceState>,
    name: String,
    entry_path: String,
) -> Result<VaultEntry, String> {
    VaultsService::create_entry(&state, name, entry_path).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn update_vault_entry(
    state: State<'_, WorkspaceState>,
    id: String,
    name: String,
    entry_path: String,
) -> Result<VaultEntry, String> {
    VaultsService::update_entry(&state, id, name, entry_path).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_vault_entry(state: State<'_, WorkspaceState>, id: String) -> Result<(), String> {
    VaultsService::delete_entry(&state, id).map_err(|error| error.to_string())
}
