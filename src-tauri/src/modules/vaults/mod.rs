mod commands;
mod keepass;
mod model;
mod service;

pub(crate) use keepass::read_keepass_entry;

pub use commands::{
    clear_keepass_database_path, create_vault_entry, delete_vault_entry, get_vaults,
    set_keepass_database_path, update_vault_entry, validate_and_set_keepass_database_path,
};
#[cfg(test)]
pub(crate) use service::VaultsService;

pub(crate) fn validate_workspace_keepass(
    state: &crate::modules::workspace::WorkspaceState,
    password: &str,
) -> Result<String, String> {
    let vault = service::VaultsService::snapshot(state).map_err(|error| error.to_string())?;
    if vault.database_path.trim().is_empty() {
        return Err("Choose a KeePass database in Vaults first.".into());
    }
    keepass::validate_keepass_database(&vault.database_path, password)?;
    Ok(vault.database_path)
}

pub(crate) fn session_keepass_password(
    state: &crate::modules::workspace::WorkspaceState,
    agent_pid: u32,
) -> Result<Option<zeroize::Zeroizing<String>>, String> {
    let vault = service::VaultsService::snapshot(state).map_err(|error| error.to_string())?;
    state
        .keepass_password(agent_pid, &vault.database_path)
        .map_err(|error| error.to_string())
}
