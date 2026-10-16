use rusqlite::OptionalExtension;
use tauri::State;

use crate::modules::workspace::WorkspaceState;

use super::{
    model::{AgentActivity, AgentStatus, SshHostKeyInspection},
    service::AgentService,
};
use pharos_agent_core::{AgentJobSetStatus, SshHostKey, SshSessionStatus, SshTerminalChunk};

#[tauri::command]
pub fn start_agent() -> Result<AgentStatus, String> {
    AgentService::start().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn stop_agent() -> Result<AgentStatus, String> {
    AgentService::stop().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn agent_status() -> AgentStatus {
    AgentService::status()
}

#[tauri::command]
pub fn scan_agent_ssh_host_key(host: String, port: u16) -> Result<SshHostKey, String> {
    AgentService::scan_ssh_host_key(host, port).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn inspect_agent_ssh_host_key(
    state: State<'_, WorkspaceState>,
    host: String,
    port: u16,
) -> Result<SshHostKeyInspection, String> {
    AgentService::inspect_ssh_host_key(&state, host, port).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn trust_agent_ssh_host_key(
    state: State<'_, WorkspaceState>,
    host: String,
    port: u16,
    expected_public_key: String,
) -> Result<SshHostKeyInspection, String> {
    AgentService::trust_ssh_host_key(&state, host, port, expected_public_key)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn forget_agent_ssh_host_key(
    state: State<'_, WorkspaceState>,
    host: String,
    port: u16,
) -> Result<(), String> {
    AgentService::forget_ssh_host_key(&state, host, port).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn list_agent_ssh_sessions() -> Result<Vec<SshSessionStatus>, String> {
    AgentService::ssh_sessions().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn list_agent_job_sets(
    state: State<'_, WorkspaceState>,
) -> Result<Vec<AgentJobSetStatus>, String> {
    let mut job_sets = AgentService::job_sets().map_err(|error| error.to_string())?;
    state
        .with_connection(|connection| {
            for job_set in &mut job_sets {
                job_set.plan_number = connection
                    .query_row(
                        "SELECT plan_number FROM deployment_plans WHERE id = ?1",
                        [&job_set.id],
                        |row| row.get(0),
                    )
                    .optional()?
                    .unwrap_or(job_set.plan_number);
            }
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    Ok(job_sets)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn connect_agent_ssh_connection(
    state: State<'_, WorkspaceState>,
    connection_id: String,
    username: Option<String>,
    credential: Option<String>,
    private_key: Option<String>,
    passphrase: Option<String>,
    columns: u16,
    rows: u16,
) -> Result<SshSessionStatus, String> {
    AgentService::connect_ssh_connection(
        &state,
        connection_id,
        username,
        credential,
        private_key,
        passphrase,
        columns,
        rows,
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn disconnect_agent_ssh_session(session_id: String) -> Result<(), String> {
    AgentService::disconnect_ssh(session_id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn read_agent_ssh_terminal(
    session_id: String,
    offset: Option<u64>,
    maximum_bytes: usize,
) -> Result<SshTerminalChunk, String> {
    AgentService::read_ssh_terminal(session_id, offset, maximum_bytes)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn write_agent_ssh_terminal(session_id: String, data: Vec<u8>) -> Result<(), String> {
    AgentService::write_ssh_terminal(session_id, data).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn resize_agent_ssh_terminal(
    session_id: String,
    columns: u16,
    rows: u16,
) -> Result<(), String> {
    AgentService::resize_ssh_terminal(session_id, columns, rows).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn list_agent_activity(state: State<'_, WorkspaceState>) -> Result<Vec<AgentActivity>, String> {
    AgentService::activities(&state).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn open_url_connection(
    state: State<'_, WorkspaceState>,
    connection_id: String,
) -> Result<AgentActivity, String> {
    AgentService::open_url(&state, connection_id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn focus_url_activity(
    state: State<'_, WorkspaceState>,
    activity_id: String,
) -> Result<(), String> {
    AgentService::focus_url(&state, activity_id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_agent_activity(
    state: State<'_, WorkspaceState>,
    activity_id: String,
) -> Result<(), String> {
    AgentService::delete_activity(&state, activity_id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn kill_agent() -> Result<AgentStatus, String> {
    crate::agent_ipc::kill().map_err(|error| error.to_string())?;
    Ok(AgentService::status())
}
