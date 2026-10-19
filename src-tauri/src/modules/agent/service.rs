use std::process::Command;

use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    agent_ipc::{self, AgentIpcError, AgentProcessStatus},
    modules::workspace::{WorkspaceError, WorkspaceState},
};

use super::model::{AgentActivity, AgentStatus, SshHostKeyInspection};
use gatetree_ops_agent_core::{
    AgentJobSetStatus, SshAuthentication, SshConnectRequest, SshHostKey, SshSessionStatus,
    SshTerminalChunk,
};

pub struct AgentService;

impl AgentService {
    pub fn start() -> Result<AgentStatus, AgentError> {
        Ok(Self::process_status(agent_ipc::start()?))
    }

    pub fn stop() -> Result<AgentStatus, AgentError> {
        Ok(Self::process_status(agent_ipc::stop()?))
    }

    pub fn status() -> AgentStatus {
        let status = agent_ipc::status().unwrap_or_else(|_| AgentProcessStatus::stopped());
        Self::process_status(status)
    }

    pub fn scan_ssh_host_key(host: String, port: u16) -> Result<SshHostKey, AgentError> {
        agent_ipc::scan_ssh_host_key(host, port).map_err(AgentError::from)
    }

    pub fn inspect_ssh_host_key(
        state: &WorkspaceState,
        host: String,
        port: u16,
    ) -> Result<SshHostKeyInspection, AgentError> {
        let host = normalize_ssh_host(&host)?;
        ensure_agent_running()?;
        let scanned = agent_ipc::scan_ssh_host_key(host.clone(), port)?;
        let trusted = state
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT public_key, fingerprint FROM ssh_known_hosts
                         WHERE host = ?1 COLLATE NOCASE AND port = ?2",
                        params![host, port],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()
            })
            .map_err(AgentError::from)?;
        let inspection_state = match &trusted {
            None => "unknown",
            Some((public_key, _)) if public_key == &scanned.public_key => "trusted",
            Some(_) => "changed",
        };
        if inspection_state == "trusted" {
            state
                .with_connection(|connection| {
                    connection.execute(
                        "UPDATE ssh_known_hosts SET last_seen_at = CURRENT_TIMESTAMP
                         WHERE host = ?1 COLLATE NOCASE AND port = ?2",
                        params![host, port],
                    )
                })
                .map_err(AgentError::from)?;
        }
        Ok(SshHostKeyInspection {
            host,
            port,
            state: inspection_state.into(),
            fingerprint: scanned.fingerprint,
            public_key: scanned.public_key,
            trusted_fingerprint: trusted.map(|(_, fingerprint)| fingerprint),
        })
    }

    pub fn trust_ssh_host_key(
        state: &WorkspaceState,
        host: String,
        port: u16,
        expected_public_key: String,
    ) -> Result<SshHostKeyInspection, AgentError> {
        let host = normalize_ssh_host(&host)?;
        ensure_agent_running()?;
        let scanned = agent_ipc::scan_ssh_host_key(host.clone(), port)?;
        if scanned.public_key != expected_public_key {
            return Err(AgentError::HostKeyChangedDuringConfirmation);
        }
        state
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO ssh_known_hosts(host, port, public_key, fingerprint)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(host, port) DO UPDATE SET
                        public_key = excluded.public_key,
                        fingerprint = excluded.fingerprint,
                        first_seen_at = CURRENT_TIMESTAMP,
                        last_seen_at = CURRENT_TIMESTAMP",
                    params![host, port, scanned.public_key, scanned.fingerprint],
                )
            })
            .map_err(AgentError::from)?;
        Ok(SshHostKeyInspection {
            host,
            port,
            state: "trusted".into(),
            fingerprint: scanned.fingerprint,
            public_key: scanned.public_key,
            trusted_fingerprint: None,
        })
    }

    pub fn forget_ssh_host_key(
        state: &WorkspaceState,
        host: String,
        port: u16,
    ) -> Result<(), AgentError> {
        let host = normalize_ssh_host(&host)?;
        state
            .with_connection(|connection| {
                connection.execute(
                    "DELETE FROM ssh_known_hosts WHERE host = ?1 COLLATE NOCASE AND port = ?2",
                    params![host, port],
                )
            })
            .map_err(AgentError::from)?;
        Ok(())
    }

    pub fn ssh_sessions() -> Result<Vec<SshSessionStatus>, AgentError> {
        agent_ipc::list_ssh_sessions().map_err(AgentError::from)
    }

    pub fn job_sets() -> Result<Vec<AgentJobSetStatus>, AgentError> {
        agent_ipc::list_job_sets().map_err(AgentError::from)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn connect_ssh_connection(
        state: &WorkspaceState,
        connection_id: String,
        username_override: Option<String>,
        credential: Option<String>,
        private_key: Option<String>,
        passphrase: Option<String>,
        columns: u16,
        rows: u16,
    ) -> Result<SshSessionStatus, AgentError> {
        let (name, host, port, _stored_username, authentication, trusted_host_key, ssh_transport) =
            state
                .with_connection(|connection| {
                    connection
                        .query_row(
                            "SELECT c.name, c.target, COALESCE(c.port, 22), c.username,
                                c.authentication, k.public_key, c.ssh_transport
                         FROM connections c
                         LEFT JOIN ssh_known_hosts k
                           ON k.host = c.target COLLATE NOCASE AND k.port = COALESCE(c.port, 22)
                         WHERE c.id = ?1 AND c.connection_type = 'ssh'",
                            [&connection_id],
                            |row| {
                                Ok((
                                    row.get::<_, String>(0)?,
                                    row.get::<_, String>(1)?,
                                    row.get::<_, u16>(2)?,
                                    row.get::<_, Option<String>>(3)?,
                                    row.get::<_, Option<String>>(4)?,
                                    row.get::<_, Option<String>>(5)?,
                                    row.get::<_, String>(6)?,
                                ))
                            },
                        )
                        .optional()
                })
                .map_err(AgentError::from)?
                .ok_or(AgentError::SshConnectionNotFound)?;
        let username = username_override.unwrap_or_default().trim().to_owned();
        if username.is_empty() {
            return Err(AgentError::SshUsernameRequired);
        }
        let (authentication, trusted_host_key) = if ssh_transport == "openssh" {
            (SshAuthentication::OpenSsh, String::new())
        } else {
            let trusted_host_key = trusted_host_key.ok_or(AgentError::SshHostKeyNotTrusted)?;
            let authentication = match authentication.as_deref().unwrap_or("password") {
                "password" => SshAuthentication::Password {
                    password: credential.ok_or(AgentError::SshCredentialRequired)?,
                },
                "keyboard-interactive" => SshAuthentication::KeyboardInteractive {
                    response: credential.ok_or(AgentError::SshCredentialRequired)?,
                },
                "private-key" => SshAuthentication::PrivateKey {
                    private_key: private_key.ok_or(AgentError::SshPrivateKeyRequired)?,
                    passphrase,
                },
                "ssh-agent" => SshAuthentication::SshAgent,
                _ => return Err(AgentError::InvalidSshAuthentication),
            };
            (authentication, trusted_host_key)
        };
        ensure_agent_running()?;
        agent_ipc::connect_ssh(SshConnectRequest {
            connection_id,
            name,
            host,
            port,
            username,
            authentication,
            trusted_host_key,
            columns,
            rows,
        })
        .map_err(AgentError::from)
    }

    pub fn disconnect_ssh(session_id: String) -> Result<(), AgentError> {
        match agent_ipc::disconnect_ssh(session_id) {
            Err(error) if missing_ssh_session(&error) => Ok(()),
            result => result.map_err(AgentError::from),
        }
    }

    pub fn read_ssh_terminal(
        session_id: String,
        offset: Option<u64>,
        maximum_bytes: usize,
    ) -> Result<SshTerminalChunk, AgentError> {
        terminal_read_result(
            agent_ipc::read_ssh_terminal(session_id, offset, maximum_bytes),
            offset,
        )
    }

    pub fn write_ssh_terminal(session_id: String, data: Vec<u8>) -> Result<(), AgentError> {
        agent_ipc::write_ssh_terminal(session_id, data).map_err(AgentError::from)
    }

    pub fn resize_ssh_terminal(
        session_id: String,
        columns: u16,
        rows: u16,
    ) -> Result<(), AgentError> {
        agent_ipc::resize_ssh_terminal(session_id, columns, rows).map_err(AgentError::from)
    }

    fn process_status(status: AgentProcessStatus) -> AgentStatus {
        AgentStatus {
            running: status.running,
            label: if status.running { "Running" } else { "Stopped" }.into(),
            can_focus_browser: cfg!(target_os = "macos"),
            pid: status.pid,
            version: status.version,
            protocol_version: status.protocol_version,
            expected_protocol_version: agent_ipc::PROTOCOL_VERSION,
            expected_version: env!("CARGO_PKG_VERSION").into(),
        }
    }

    pub fn activities(state: &WorkspaceState) -> Result<Vec<AgentActivity>, AgentError> {
        state
            .with_connection(load_activities)
            .map_err(AgentError::from)
    }

    pub fn open_url(
        state: &WorkspaceState,
        connection_id: String,
    ) -> Result<AgentActivity, AgentError> {
        let (name, target) = state
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT name, target FROM connections
                         WHERE id = ?1 AND connection_type = 'url'",
                        [&connection_id],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()
            })
            .map_err(AgentError::from)?
            .ok_or(AgentError::UrlConnectionNotFound)?;
        validate_url(&target)?;
        open_in_system_browser(&target)?;

        let id = Uuid::new_v4().to_string();
        state
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO agent_activity(
                        id, connection_id, resource_type, name, target, status
                     ) VALUES (?1, ?2, 'url', ?3, ?4, 'open')",
                    params![id, connection_id, name, target],
                )?;
                activity_by_id(connection, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
            })
            .map_err(AgentError::from)
    }

    pub fn focus_url(state: &WorkspaceState, activity_id: String) -> Result<(), AgentError> {
        let target = state
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT target FROM agent_activity WHERE id = ?1 AND resource_type = 'url'",
                        [&activity_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
            })
            .map_err(AgentError::from)?
            .ok_or(AgentError::ActivityNotFound)?;
        validate_url(&target)?;
        focus_system_browser_tab(&target)
    }

    pub fn delete_activity(state: &WorkspaceState, activity_id: String) -> Result<(), AgentError> {
        let deleted = state
            .with_connection(|connection| {
                connection.execute("DELETE FROM agent_activity WHERE id = ?1", [&activity_id])
            })
            .map_err(AgentError::from)?;
        if deleted == 0 {
            return Err(AgentError::ActivityNotFound);
        }
        Ok(())
    }
}

fn missing_ssh_session(error: &AgentIpcError) -> bool {
    matches!(error, AgentIpcError::Rejected(message) if message == "SSH session was not found")
}

// Agent removes exited sessions while listing them. A terminal that was already
// mounted should finish normally when its next read races that cleanup.
fn terminal_read_result(
    result: Result<SshTerminalChunk, AgentIpcError>,
    offset: Option<u64>,
) -> Result<SshTerminalChunk, AgentError> {
    match result {
        Err(error) if missing_ssh_session(&error) => Ok(SshTerminalChunk {
            data: vec![],
            closed: true,
            next_offset: offset.unwrap_or(0),
            truncated: false,
        }),
        result => result.map_err(AgentError::from),
    }
}

fn ensure_agent_running() -> Result<(), AgentError> {
    if !agent_ipc::status().is_ok_and(|status| status.running) {
        agent_ipc::start()?;
    }
    Ok(())
}

fn normalize_ssh_host(host: &str) -> Result<String, AgentError> {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() || host.contains(['\r', '\n']) {
        Err(AgentError::InvalidSshHost)
    } else {
        Ok(host)
    }
}

fn validate_url(value: &str) -> Result<(), AgentError> {
    let lowercase = value.to_ascii_lowercase();
    if (lowercase.starts_with("https://") || lowercase.starts_with("http://"))
        && !value.contains('\r')
        && !value.contains('\n')
    {
        Ok(())
    } else {
        Err(AgentError::InvalidUrl)
    }
}

#[cfg(target_os = "macos")]
fn open_in_system_browser(url: &str) -> Result<(), AgentError> {
    Command::new("open").arg(url).spawn()?.wait()?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn focus_system_browser_tab(url: &str) -> Result<(), AgentError> {
    const SCRIPT: &str = r#"
on run argv
    set targetURL to item 1 of argv
    tell application "Google Chrome"
        repeat with browserWindow in windows
            set tabIndex to 0
            repeat with browserTab in tabs of browserWindow
                set tabIndex to tabIndex + 1
                if URL of browserTab is targetURL then
                    set active tab index of browserWindow to tabIndex
                    set index of browserWindow to 1
                    activate
                    return true
                end if
            end repeat
        end repeat
    end tell
    return false
end run
"#;
    let output = Command::new("osascript")
        .args(["-e", SCRIPT, url])
        .output()?;
    if output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true" {
        Ok(())
    } else {
        Err(AgentError::BrowserTabNotFound)
    }
}

#[cfg(not(target_os = "macos"))]
fn focus_system_browser_tab(_url: &str) -> Result<(), AgentError> {
    Err(AgentError::BrowserFocusUnsupported)
}

#[cfg(target_os = "windows")]
fn open_in_system_browser(url: &str) -> Result<(), AgentError> {
    Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn()?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn open_in_system_browser(url: &str) -> Result<(), AgentError> {
    Command::new("xdg-open").arg(url).spawn()?;
    Ok(())
}

fn load_activities(connection: &Connection) -> rusqlite::Result<Vec<AgentActivity>> {
    let mut statement = connection.prepare(
        "SELECT id, connection_id, resource_type, name, target, status, opened_at
         FROM agent_activity ORDER BY opened_at DESC",
    )?;
    statement
        .query_map([], activity_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()
}

fn activity_by_id(connection: &Connection, id: &str) -> rusqlite::Result<Option<AgentActivity>> {
    connection
        .query_row(
            "SELECT id, connection_id, resource_type, name, target, status, opened_at
             FROM agent_activity WHERE id = ?1",
            [id],
            activity_from_row,
        )
        .optional()
}

fn activity_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentActivity> {
    Ok(AgentActivity {
        id: row.get(0)?,
        connection_id: row.get(1)?,
        resource_type: row.get(2)?,
        name: row.get(3)?,
        target: row.get(4)?,
        status: row.get(5)?,
        opened_at: row.get(6)?,
    })
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("SSH host is invalid.")]
    InvalidSshHost,
    #[error("The SSH host key changed while it was being confirmed. Check it again.")]
    HostKeyChangedDuringConfirmation,
    #[error("SSH connection was not found.")]
    SshConnectionNotFound,
    #[error("Enter an SSH username before connecting.")]
    SshUsernameRequired,
    #[error("Trust the SSH host key before connecting.")]
    SshHostKeyNotTrusted,
    #[error("SSH authentication requires a credential.")]
    SshCredentialRequired,
    #[error("SSH private-key authentication requires a private key.")]
    SshPrivateKeyRequired,
    #[error("Choose a valid SSH authentication method.")]
    InvalidSshAuthentication,
    #[error("URL connection was not found.")]
    UrlConnectionNotFound,
    #[error("Only HTTP and HTTPS URLs can be opened.")]
    InvalidUrl,
    #[error("Agent activity was not found.")]
    ActivityNotFound,
    #[error("The Chrome tab is no longer open.")]
    BrowserTabNotFound,
    #[cfg(not(target_os = "macos"))]
    #[error("Browser tab focus is not available on this platform.")]
    BrowserFocusUnsupported,
    #[error(transparent)]
    Ipc(#[from] AgentIpcError),
    #[error("Could not open the system browser: {0}")]
    Browser(#[from] std::io::Error),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
}

#[cfg(test)]
mod session_tests {
    use super::*;
    #[test]
    fn reading_a_session_cleaned_up_by_agent_reports_closed_without_hiding_other_errors() {
        let result = terminal_read_result(
            Err(AgentIpcError::Rejected("SSH session was not found".into())),
            Some(123),
        )
        .unwrap();
        assert!(result.closed);
        assert!(result.data.is_empty());
        assert_eq!(result.next_offset, 123);
        assert!(
            terminal_read_result(
                Err(AgentIpcError::Rejected("Permission denied".into())),
                None
            )
            .is_err()
        );
        assert!(terminal_read_result(Err(AgentIpcError::NotRunning), None).is_err());
        let live = terminal_read_result(
            Ok(SshTerminalChunk {
                data: b"output".to_vec(),
                closed: false,
                next_offset: 6,
                truncated: false,
            }),
            None,
        )
        .unwrap();
        assert!(!live.closed);
        assert_eq!(live.data, b"output");
    }
}
