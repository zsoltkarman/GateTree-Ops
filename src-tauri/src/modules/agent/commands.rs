use rusqlite::OptionalExtension;
use tauri::State;

use crate::modules::workspace::WorkspaceState;

use super::{
    model::{AgentActivity, AgentStatus, SshHostKeyInspection},
    service::AgentService,
};
use gatetree_ops_agent_core::{AgentJobSetStatus, SshHostKey, SshSessionStatus, SshTerminalChunk};

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

#[tauri::command]
pub async fn connect_agent_ssh_with_keepass(
    app: tauri::AppHandle,
    connection_id: String,
    columns: u16,
    rows: u16,
) -> Result<SshSessionStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;
        let state = app.state::<WorkspaceState>();
        let selection = crate::modules::connections::ConnectionsService::resolve_account(&state, &connection_id).map_err(|e| e.to_string())?;
        if selection.mode != "keepass" { return Err("This connection no longer uses a KeePass account. Connect again.".into()); }
        let reference = selection.credential_reference.ok_or("The assigned KeePass account was deleted. Choose another account.")?;
        let (entry_path, authentication, transport): (String, String, String) = state.with_connection(|db| db.query_row(
            "SELECT v.entry_path, COALESCE(c.authentication,'password'), c.ssh_transport FROM vault_entries v JOIN connections c ON c.id=?2 AND c.connection_type='ssh' WHERE v.id=?1", [&reference, &connection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))).map_err(|e| e.to_string())?;
        if transport != "openssh" && !matches!(authentication.as_str(), "password" | "keyboard-interactive" | "ssh-agent") {
            return Err("KeePass account login supports Password, Keyboard interactive, or SSH agent. Use manual login for a private key.".into());
        }
        let pid = AgentService::status().pid.ok_or("Start Agent and unlock KeePass first.")?;
        let password = crate::modules::vaults::session_keepass_password(&state, pid)?.ok_or("Unlock KeePass for this session first.")?;
        let database = state.with_connection(|db| db.query_row("SELECT value FROM vault_settings WHERE key='keepass_database_path'", [], |row| row.get::<_, String>(0))).map_err(|e| e.to_string())?;
        let account = crate::modules::vaults::read_keepass_entry(&database, &entry_path, &password)?;
        if AgentService::status().pid != Some(pid) { return Err("Agent restarted during login. Unlock KeePass again.".into()); }
        let current = crate::modules::connections::ConnectionsService::resolve_account(&state, &connection_id).map_err(|e| e.to_string())?;
        if current.mode != "keepass" || current.credential_reference.as_deref() != Some(&reference) {
            return Err("The connection account changed during login. Connect again.".into());
        }
        AgentService::connect_ssh_connection(&state, connection_id, Some(account.username), Some(account.password), None, None, columns, rows).map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string())?
}
