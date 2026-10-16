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
    )
    .map_err(|error| error.to_string())
}
