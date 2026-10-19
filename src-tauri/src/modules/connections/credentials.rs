use super::{
    model::ConnectionFolder,
    service::{ConnectionsError, ConnectionsService, get_folder},
};
use crate::modules::workspace::WorkspaceState;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use std::collections::HashSet;
use tauri::State;

pub(super) fn validate_binding(
    state: &WorkspaceState,
    mode: Option<String>,
    reference: Option<String>,
) -> Result<(String, Option<String>), ConnectionsError> {
    let reference = reference.filter(|s| !s.trim().is_empty());
    let mode = mode.unwrap_or_else(|| {
        if reference.is_some() {
            "keepass"
        } else {
            "inherit"
        }
        .into()
    });
    match mode.as_str() {
        "inherit" | "manual" => Ok((mode, None)),
        "keepass" => {
            let reference = reference.ok_or(ConnectionsError::CredentialRequired)?;
            let exists = state.with_connection(|db| {
                db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM vault_entries WHERE id=?1)",
                    [&reference],
                    |row| row.get::<_, bool>(0),
                )
            })?;
            if !exists {
                return Err(ConnectionsError::CredentialReferenceNotFound);
            }
            Ok((mode, Some(reference)))
        }
        _ => Err(ConnectionsError::InvalidCredentialMode),
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedAccount {
    pub mode: String,
    pub credential_reference: Option<String>,
    pub account_name: Option<String>,
    pub source_folder_id: Option<String>,
    pub source_name: String,
}
impl ConnectionsService {
    pub fn set_folder_credentials(
        state: &WorkspaceState,
        id: String,
        mode: String,
        reference: Option<String>,
    ) -> Result<ConnectionFolder, ConnectionsError> {
        get_folder(state, &id)?;
        let (mode, reference) = validate_binding(state, Some(mode), reference)?;
        state.with_connection(|db| {
            db.execute("UPDATE connection_folders SET credential_mode=?2, credential_reference=?3, updated_at=CURRENT_TIMESTAMP WHERE id=?1", params![id, mode, reference])?;
            Ok(())
        })?;
        get_folder(state, &id)
    }
    pub fn resolve_account(
        state: &WorkspaceState,
        id: &str,
    ) -> Result<ResolvedAccount, ConnectionsError> {
        state.with_connection(|db| {
            let (mut mode, mut reference, folder, mut source_name, connection_type): (String, Option<String>, String, String, String) = db.query_row(
                "SELECT credential_mode, credential_reference, folder_id, name, connection_type FROM connections WHERE id=?1", [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))?;
            if connection_type == "url" { return Ok(ResolvedAccount { mode: "manual".into(), credential_reference: None, account_name: None, source_folder_id: None, source_name }); }
            let mut next = Some(folder);
            let mut source_folder_id = None;
            let mut visited = HashSet::new();
            while mode == "inherit" {
                let Some(folder_id) = next.take() else { break; };
                if !visited.insert(folder_id.clone()) { return Err(rusqlite::Error::InvalidQuery); }
                let (folder_mode, folder_reference, parent, name): (String, Option<String>, Option<String>, String) = db.query_row(
                    "SELECT credential_mode, credential_reference, parent_id, name FROM connection_folders WHERE id=?1", [&folder_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?;
                mode = folder_mode; reference = folder_reference; next = parent;
                if mode != "inherit" { source_folder_id = Some(folder_id); source_name = name; }
            }
            if mode == "inherit" { mode = "manual".into(); }
            let account_name = if let Some(reference) = &reference {
                db.query_row("SELECT name FROM vault_entries WHERE id=?1", [reference], |row| row.get(0)).optional()?
            } else { None };
            Ok(ResolvedAccount { mode, credential_reference: reference, account_name, source_folder_id, source_name })
        }).map_err(ConnectionsError::from)
    }
}
#[tauri::command]
pub fn set_connection_folder_credentials(
    state: State<'_, WorkspaceState>,
    id: String,
    mode: String,
    credential_reference: Option<String>,
) -> Result<ConnectionFolder, String> {
    ConnectionsService::set_folder_credentials(&state, id, mode, credential_reference)
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn resolve_connection_account(
    state: State<'_, WorkspaceState>,
    connection_id: String,
) -> Result<ResolvedAccount, String> {
    ConnectionsService::resolve_account(&state, &connection_id).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::{vaults::VaultsService, workspace::WorkspaceService};
    #[test]
    fn nearest_account_overrides_manual_opt_out_moves_and_deletions_are_live() {
        let directory =
            std::env::temp_dir().join(format!("gatetree-ops-account-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let state = WorkspaceState::default();
        WorkspaceService::create(
            &state,
            directory.to_string_lossy().into_owned(),
            "a-valid-test-password",
        )
        .unwrap();
        let a =
            VaultsService::create_entry(&state, "Account A".into(), "Accounts/A".into()).unwrap();
        let b =
            VaultsService::create_entry(&state, "Account B".into(), "Accounts/B".into()).unwrap();
        let root = ConnectionsService::create_folder(&state, "Root".into(), None).unwrap();
        let child =
            ConnectionsService::create_folder(&state, "Child".into(), Some(root.id.clone()))
                .unwrap();
        let deep = ConnectionsService::create_folder(&state, "Deep".into(), Some(child.id.clone()))
            .unwrap();
        let other = ConnectionsService::create_folder(&state, "Other".into(), None).unwrap();
        ConnectionsService::set_folder_credentials(
            &state,
            root.id.clone(),
            "keepass".into(),
            Some(a.id.clone()),
        )
        .unwrap();
        ConnectionsService::set_folder_credentials(
            &state,
            other.id.clone(),
            "keepass".into(),
            Some(b.id.clone()),
        )
        .unwrap();
        let ssh = ConnectionsService::create_connection(
            &state,
            deep.id.clone(),
            "ssh".into(),
            "SSH host".into(),
            "host.test".into(),
            Some(22),
            "".into(),
            None,
            Some("password".into()),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let rdp = ConnectionsService::create_connection(
            &state,
            deep.id.clone(),
            "rdp".into(),
            "RDP host".into(),
            "desktop.test".into(),
            Some(3389),
            "".into(),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        for id in [&ssh.id, &rdp.id] {
            let resolved = ConnectionsService::resolve_account(&state, id).unwrap();
            assert_eq!(
                resolved.credential_reference.as_deref(),
                Some(a.id.as_str())
            );
            assert_eq!(resolved.source_folder_id.as_deref(), Some(root.id.as_str()));
        }
        ConnectionsService::set_folder_credentials(
            &state,
            child.id.clone(),
            "keepass".into(),
            Some(b.id.clone()),
        )
        .unwrap();
        assert_eq!(
            ConnectionsService::resolve_account(&state, &ssh.id)
                .unwrap()
                .credential_reference,
            Some(b.id.clone())
        );
        ConnectionsService::set_folder_credentials(&state, child.id.clone(), "manual".into(), None)
            .unwrap();
        assert_eq!(
            ConnectionsService::resolve_account(&state, &ssh.id)
                .unwrap()
                .mode,
            "manual"
        );
        let edit = |mode: &str, reference: Option<String>| {
            ConnectionsService::update_connection(
                &state,
                ssh.id.clone(),
                ssh.name.clone(),
                ssh.target.clone(),
                ssh.port,
                "".into(),
                None,
                Some("password".into()),
                reference,
                None,
                None,
                Some(mode.into()),
            )
        };
        edit("keepass", Some(a.id.clone())).unwrap();
        assert_eq!(
            ConnectionsService::resolve_account(&state, &ssh.id)
                .unwrap()
                .credential_reference,
            Some(a.id.clone())
        );
        assert!(edit("keepass", Some("missing".into())).is_err());
        assert_eq!(
            ConnectionsService::resolve_account(&state, &ssh.id)
                .unwrap()
                .credential_reference,
            Some(a.id.clone())
        );
        edit("manual", None).unwrap();
        ConnectionsService::set_folder_credentials(
            &state,
            child.id.clone(),
            "inherit".into(),
            None,
        )
        .unwrap();
        assert_eq!(
            ConnectionsService::resolve_account(&state, &ssh.id)
                .unwrap()
                .mode,
            "manual"
        );
        edit("inherit", None).unwrap();
        ConnectionsService::move_folder(&state, child.id.clone(), Some(other.id.clone())).unwrap();
        assert_eq!(
            ConnectionsService::resolve_account(&state, &ssh.id)
                .unwrap()
                .credential_reference,
            Some(b.id.clone())
        );
        state.lock().unwrap();
        WorkspaceService::unlock(
            &state,
            directory.to_string_lossy().into_owned(),
            "a-valid-test-password",
        )
        .unwrap();
        assert_eq!(
            ConnectionsService::resolve_account(&state, &rdp.id)
                .unwrap()
                .credential_reference,
            Some(b.id.clone())
        );
        VaultsService::delete_entry(&state, b.id).unwrap();
        let missing = ConnectionsService::resolve_account(&state, &ssh.id).unwrap();
        assert_eq!(missing.mode, "keepass");
        assert!(missing.credential_reference.is_none());
        assert!(missing.account_name.is_none());
        state.lock().unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
