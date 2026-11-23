use tauri::State;

use crate::modules::workspace::WorkspaceState;

use super::{
    model::{ConnectionFolder, ConnectionItem},
    service::ConnectionsService,
};

#[tauri::command]
pub fn list_connection_folders(
    state: State<'_, WorkspaceState>,
) -> Result<Vec<ConnectionFolder>, String> {
    ConnectionsService::list_folders(&state).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn create_connection_folder(
    state: State<'_, WorkspaceState>,
    name: String,
    parent_id: Option<String>,
) -> Result<ConnectionFolder, String> {
    ConnectionsService::create_folder(&state, name, parent_id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn rename_connection_folder(
    state: State<'_, WorkspaceState>,
    id: String,
    name: String,
) -> Result<ConnectionFolder, String> {
    ConnectionsService::rename_folder(&state, id, name).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn move_connection_folder(
    state: State<'_, WorkspaceState>,
    id: String,
    parent_id: Option<String>,
) -> Result<ConnectionFolder, String> {
    ConnectionsService::move_folder(&state, id, parent_id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_connection_folder(
    state: State<'_, WorkspaceState>,
    id: String,
) -> Result<(), String> {
    ConnectionsService::delete_folder(&state, id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn list_connection_items(
    state: State<'_, WorkspaceState>,
) -> Result<Vec<ConnectionItem>, String> {
    ConnectionsService::list_connections(&state).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn create_connection_item(
    state: State<'_, WorkspaceState>,
    folder_id: String,
    connection_type: String,
    name: String,
    target: String,
    port: Option<u16>,
    tag: String,
    username: Option<String>,
    authentication: Option<String>,
    credential_reference: Option<String>,
    ssh_transport: Option<String>,
    icon: Option<String>,
    credential_mode: Option<String>,
) -> Result<ConnectionItem, String> {
    ConnectionsService::create_connection(
        &state,
        folder_id,
        connection_type,
        name,
        target,
        port,
        tag,
        username,
        authentication,
        credential_reference,
        ssh_transport,
        icon,
        credential_mode,
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn move_connection_item(
    state: State<'_, WorkspaceState>,
    id: String,
    folder_id: String,
) -> Result<ConnectionItem, String> {
    ConnectionsService::move_connection(&state, id, folder_id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn update_connection_item(
    state: State<'_, WorkspaceState>,
    id: String,
    name: String,
    target: String,
    port: Option<u16>,
    tag: String,
    username: Option<String>,
    authentication: Option<String>,
    credential_reference: Option<String>,
    ssh_transport: Option<String>,
    icon: Option<String>,
    credential_mode: Option<String>,
) -> Result<ConnectionItem, String> {
    ConnectionsService::update_connection(
        &state,
        id,
        name,
        target,
        port,
        tag,
        username,
        authentication,
        credential_reference,
        ssh_transport,
        icon,
        credential_mode,
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_connection_ssh_tunnels(
    state: State<'_, WorkspaceState>,
    id: String,
    local_tunnels: Vec<gatetree_ops_agent_core::SshLocalTunnel>,
) -> Result<ConnectionItem, String> {
    save_ssh_tunnels(&state, id, local_tunnels)
}

fn save_ssh_tunnels(
    state: &WorkspaceState,
    id: String,
    local_tunnels: Vec<gatetree_ops_agent_core::SshLocalTunnel>,
) -> Result<ConnectionItem, String> {
    gatetree_ops_agent_core::SshLocalTunnel::validate_all(&local_tunnels)?;
    let rules = serde_json::to_string(&local_tunnels).map_err(|e| e.to_string())?;
    state.with_connection(|db| {
        let tx = db.unchecked_transaction()?;
        let changed = tx.execute(
            "UPDATE connections SET ssh_transport = CASE WHEN ?2 = '[]' THEN ssh_transport ELSE 'openssh' END,
             updated_at = CURRENT_TIMESTAMP WHERE id=?1 AND connection_type='ssh'", rusqlite::params![id, rules])?;
        if changed == 0 { return Err(rusqlite::Error::QueryReturnedNoRows); }
        tx.execute("INSERT INTO connection_ssh_tunnels(connection_id,rules) VALUES(?1,?2)
            ON CONFLICT(connection_id) DO UPDATE SET rules=excluded.rules", rusqlite::params![id,rules])?;
        tx.commit()?;
        super::service::connection_by_id(db, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
    }).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::workspace::WorkspaceService;
    use gatetree_ops_agent_core::SshLocalTunnel;

    #[test]
    fn tunnel_rules_round_trip_preserve_connection_fields_and_cascade_on_delete() {
        let directory =
            std::env::temp_dir().join(format!("gatetree-tunnels-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let state = WorkspaceState::default();
        WorkspaceService::create(
            &state,
            directory.to_string_lossy().into_owned(),
            "a-valid-test-password",
        )
        .unwrap();
        let folder = ConnectionsService::create_folder(&state, "Servers".into(), None).unwrap();
        let saved = ConnectionsService::create_connection(
            &state,
            folder.id.clone(),
            "ssh".into(),
            "Centreon".into(),
            "centreon2.iad.icprod.oracleindustry.com".into(),
            Some(22),
            "SQL".into(),
            Some("operator".into()),
            Some("password".into()),
            None,
            Some("embedded".into()),
            None,
            Some("manual".into()),
        )
        .unwrap();
        assert!(saved.local_tunnels.is_empty());
        let rules = vec![SshLocalTunnel {
            source_port: 13306,
            destination_host: "127.0.0.1".into(),
            destination_port: 3306,
        }];
        let updated = save_ssh_tunnels(&state, saved.id.clone(), rules.clone()).unwrap();
        assert_eq!(updated.ssh_transport, "openssh");
        assert_eq!(updated.local_tunnels, rules);
        assert_eq!(updated.username, saved.username);
        assert_eq!(updated.target, saved.target);
        assert_eq!(updated.credential_mode, "manual");
        assert_eq!(
            ConnectionsService::list_connections(&state).unwrap()[0].local_tunnels,
            rules
        );
        assert!(
            save_ssh_tunnels(
                &state,
                saved.id.clone(),
                vec![rules[0].clone(), rules[0].clone()]
            )
            .is_err()
        );
        assert_eq!(
            ConnectionsService::list_connections(&state).unwrap()[0].local_tunnels,
            rules
        );
        let cleared = save_ssh_tunnels(&state, saved.id.clone(), vec![]).unwrap();
        assert!(cleared.local_tunnels.is_empty());
        assert_eq!(cleared.ssh_transport, "openssh");
        save_ssh_tunnels(&state, saved.id, rules).unwrap();
        ConnectionsService::delete_folder(&state, folder.id).unwrap();
        let count: i64 = state
            .with_connection(|db| {
                db.query_row("SELECT count(*) FROM connection_ssh_tunnels", [], |row| {
                    row.get(0)
                })
            })
            .unwrap();
        assert_eq!(count, 0);
        drop(state);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
