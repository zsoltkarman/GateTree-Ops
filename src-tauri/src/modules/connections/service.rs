use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;
use uuid::Uuid;

use crate::modules::workspace::{WorkspaceError, WorkspaceState};

use super::model::{ConnectionFolder, ConnectionItem};

pub struct ConnectionsService;

impl ConnectionsService {
    pub fn list_folders(state: &WorkspaceState) -> Result<Vec<ConnectionFolder>, ConnectionsError> {
        state
            .with_connection(load_folders)
            .map_err(ConnectionsError::from)
    }

    pub fn create_folder(
        state: &WorkspaceState,
        name: String,
        parent_id: Option<String>,
    ) -> Result<ConnectionFolder, ConnectionsError> {
        let name = validated_name(name)?;
        validate_parent(state, parent_id.as_deref())?;
        ensure_unique_name(state, &name, parent_id.as_deref(), None)?;
        let id = Uuid::new_v4().to_string();
        state
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO connection_folders(id, name, parent_id) VALUES (?1, ?2, ?3)",
                    params![id, name, parent_id],
                )?;
                folder_by_id(connection, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
            })
            .map_err(ConnectionsError::from)
    }

    pub fn rename_folder(
        state: &WorkspaceState,
        id: String,
        name: String,
    ) -> Result<ConnectionFolder, ConnectionsError> {
        let name = validated_name(name)?;
        let folder = get_folder(state, &id)?;
        ensure_unique_name(state, &name, folder.parent_id.as_deref(), Some(&id))?;
        state
            .with_connection(|connection| {
                connection.execute(
                    "UPDATE connection_folders
                     SET name = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                    params![id, name],
                )?;
                folder_by_id(connection, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
            })
            .map_err(ConnectionsError::from)
    }

    pub fn move_folder(
        state: &WorkspaceState,
        id: String,
        parent_id: Option<String>,
    ) -> Result<ConnectionFolder, ConnectionsError> {
        let folder = get_folder(state, &id)?;
        validate_parent(state, parent_id.as_deref())?;
        if parent_id.as_deref() == Some(id.as_str()) {
            return Err(ConnectionsError::CircularMove);
        }
        if let Some(parent) = parent_id.as_deref()
            && is_descendant(state, &id, parent)?
        {
            return Err(ConnectionsError::CircularMove);
        }
        ensure_unique_name(state, &folder.name, parent_id.as_deref(), Some(&id))?;
        state
            .with_connection(|connection| {
                connection.execute(
                    "UPDATE connection_folders
                     SET parent_id = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                    params![id, parent_id],
                )?;
                folder_by_id(connection, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
            })
            .map_err(ConnectionsError::from)
    }

    pub fn delete_folder(state: &WorkspaceState, id: String) -> Result<(), ConnectionsError> {
        let changed = state
            .with_connection(|connection| {
                connection.execute("DELETE FROM connection_folders WHERE id = ?1", [id])
            })
            .map_err(ConnectionsError::from)?;
        if changed == 0 {
            return Err(ConnectionsError::FolderNotFound);
        }
        Ok(())
    }

    pub fn list_connections(
        state: &WorkspaceState,
    ) -> Result<Vec<ConnectionItem>, ConnectionsError> {
        state
            .with_connection(load_connections)
            .map_err(ConnectionsError::from)
    }

    pub fn create_connection(
        state: &WorkspaceState,
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
    ) -> Result<ConnectionItem, ConnectionsError> {
        validate_parent(state, Some(&folder_id))?;
        let connection_type = validated_connection_type(connection_type)?;
        let name = validated_connection_value(name, ConnectionsError::EmptyConnectionName)?;
        let target = validated_connection_value(target, ConnectionsError::EmptyConnectionTarget)?;
        let tag = tag.trim().to_owned();
        let (username, authentication, credential_reference, ssh_transport) = validate_ssh_fields(
            state,
            &connection_type,
            username,
            authentication,
            credential_reference,
            ssh_transport,
        )?;
        let duplicate = state
            .with_connection(|connection| {
                connection.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM connections
                        WHERE folder_id = ?1 AND name = ?2 COLLATE NOCASE
                    )",
                    params![folder_id, name],
                    |row| row.get::<_, bool>(0),
                )
            })
            .map_err(ConnectionsError::from)?;
        if duplicate {
            return Err(ConnectionsError::DuplicateConnectionName);
        }

        let id = Uuid::new_v4().to_string();
        state
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO connections(
                        id, folder_id, connection_type, name, target, port, tag,
                        username, authentication, credential_reference, ssh_transport
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        id,
                        folder_id,
                        connection_type,
                        name,
                        target,
                        port,
                        tag,
                        username,
                        authentication,
                        credential_reference,
                        ssh_transport
                    ],
                )?;
                connection_by_id(connection, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
            })
            .map_err(ConnectionsError::from)
    }

    pub fn move_connection(
        state: &WorkspaceState,
        id: String,
        folder_id: String,
    ) -> Result<ConnectionItem, ConnectionsError> {
        validate_parent(state, Some(&folder_id))?;
        let item = get_connection(state, &id)?;
        let duplicate = state
            .with_connection(|connection| {
                connection.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM connections
                        WHERE folder_id = ?1 AND name = ?2 COLLATE NOCASE AND id != ?3
                    )",
                    params![folder_id, item.name, id],
                    |row| row.get::<_, bool>(0),
                )
            })
            .map_err(ConnectionsError::from)?;
        if duplicate {
            return Err(ConnectionsError::DuplicateConnectionName);
        }
        state
            .with_connection(|connection| {
                connection.execute(
                    "UPDATE connections
                     SET folder_id = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                    params![id, folder_id],
                )?;
                connection_by_id(connection, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
            })
            .map_err(ConnectionsError::from)
    }

    pub fn update_connection(
        state: &WorkspaceState,
        id: String,
        name: String,
        target: String,
        port: Option<u16>,
        tag: String,
        username: Option<String>,
        authentication: Option<String>,
        credential_reference: Option<String>,
        ssh_transport: Option<String>,
    ) -> Result<ConnectionItem, ConnectionsError> {
        let item = get_connection(state, &id)?;
        let name = validated_connection_value(name, ConnectionsError::EmptyConnectionName)?;
        let target = validated_connection_value(target, ConnectionsError::EmptyConnectionTarget)?;
        let tag = tag.trim().to_owned();
        let (username, authentication, credential_reference, ssh_transport) = validate_ssh_fields(
            state,
            &item.connection_type,
            username,
            authentication,
            credential_reference,
            ssh_transport,
        )?;
        let duplicate = state
            .with_connection(|connection| {
                connection.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM connections
                        WHERE folder_id = ?1 AND name = ?2 COLLATE NOCASE AND id != ?3
                    )",
                    params![item.folder_id, name, id],
                    |row| row.get::<_, bool>(0),
                )
            })
            .map_err(ConnectionsError::from)?;
        if duplicate {
            return Err(ConnectionsError::DuplicateConnectionName);
        }
        state
            .with_connection(|connection| {
                connection.execute(
                    "UPDATE connections
                     SET name = ?2, target = ?3, port = ?4, tag = ?5,
                         username = ?6, authentication = ?7, credential_reference = ?8,
                         ssh_transport = ?9,
                         updated_at = CURRENT_TIMESTAMP
                     WHERE id = ?1",
                    params![
                        id,
                        name,
                        target,
                        port,
                        tag,
                        username,
                        authentication,
                        credential_reference,
                        ssh_transport
                    ],
                )?;
                connection_by_id(connection, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
            })
            .map_err(ConnectionsError::from)
    }
}

fn validated_connection_type(value: String) -> Result<String, ConnectionsError> {
    let value = value.to_lowercase();
    if matches!(value.as_str(), "ssh" | "rdp" | "url") {
        Ok(value)
    } else {
        Err(ConnectionsError::InvalidConnectionType)
    }
}

fn validate_ssh_fields(
    state: &WorkspaceState,
    connection_type: &str,
    username: Option<String>,
    authentication: Option<String>,
    credential_reference: Option<String>,
    ssh_transport: Option<String>,
) -> Result<(Option<String>, Option<String>, Option<String>, String), ConnectionsError> {
    if connection_type != "ssh" {
        return Ok((None, None, None, "embedded".into()));
    }

    let username = optional_trimmed(username);
    let authentication =
        optional_trimmed(authentication).ok_or(ConnectionsError::InvalidSshAuthentication)?;
    if !matches!(
        authentication.as_str(),
        "password" | "private-key" | "keyboard-interactive" | "ssh-agent"
    ) {
        return Err(ConnectionsError::InvalidSshAuthentication);
    }

    let credential_reference = optional_trimmed(credential_reference);
    let ssh_transport = optional_trimmed(ssh_transport).unwrap_or_else(|| "embedded".into());
    if !matches!(ssh_transport.as_str(), "embedded" | "openssh") {
        return Err(ConnectionsError::InvalidSshTransport);
    }
    if let Some(reference) = credential_reference.as_deref() {
        let exists = state
            .with_connection(|connection| {
                connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM vault_entries WHERE id = ?1)",
                    [reference],
                    |row| row.get::<_, bool>(0),
                )
            })
            .map_err(ConnectionsError::from)?;
        if !exists {
            return Err(ConnectionsError::CredentialReferenceNotFound);
        }
    }

    Ok((
        username,
        Some(authentication),
        credential_reference,
        ssh_transport,
    ))
}

fn optional_trimmed(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn validated_connection_value(
    value: String,
    error: ConnectionsError,
) -> Result<String, ConnectionsError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(error)
    } else {
        Ok(value)
    }
}

fn validated_name(name: String) -> Result<String, ConnectionsError> {
    let name = name.trim().to_owned();
    if name.is_empty() {
        Err(ConnectionsError::EmptyFolderName)
    } else if name.chars().count() > 120 {
        Err(ConnectionsError::FolderNameTooLong)
    } else {
        Ok(name)
    }
}

fn validate_parent(
    state: &WorkspaceState,
    parent_id: Option<&str>,
) -> Result<(), ConnectionsError> {
    let Some(parent_id) = parent_id else {
        return Ok(());
    };
    let exists = state
        .with_connection(|connection| {
            connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM connection_folders WHERE id = ?1)",
                [parent_id],
                |row| row.get::<_, bool>(0),
            )
        })
        .map_err(ConnectionsError::from)?;
    if exists {
        Ok(())
    } else {
        Err(ConnectionsError::ParentNotFound)
    }
}

fn ensure_unique_name(
    state: &WorkspaceState,
    name: &str,
    parent_id: Option<&str>,
    excluded_id: Option<&str>,
) -> Result<(), ConnectionsError> {
    let exists = state
        .with_connection(|connection| {
            connection.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM connection_folders
                    WHERE parent_id IS ?1 AND name = ?2 COLLATE NOCASE
                      AND (?3 IS NULL OR id != ?3)
                )",
                params![parent_id, name, excluded_id],
                |row| row.get::<_, bool>(0),
            )
        })
        .map_err(ConnectionsError::from)?;
    if exists {
        Err(ConnectionsError::DuplicateFolderName)
    } else {
        Ok(())
    }
}

fn is_descendant(
    state: &WorkspaceState,
    folder_id: &str,
    possible_descendant_id: &str,
) -> Result<bool, ConnectionsError> {
    state
        .with_connection(|connection| {
            connection.query_row(
                "WITH RECURSIVE descendants(id) AS (
                    SELECT id FROM connection_folders WHERE parent_id = ?1
                    UNION ALL
                    SELECT folder.id FROM connection_folders folder
                    JOIN descendants ON folder.parent_id = descendants.id
                )
                SELECT EXISTS(SELECT 1 FROM descendants WHERE id = ?2)",
                params![folder_id, possible_descendant_id],
                |row| row.get(0),
            )
        })
        .map_err(ConnectionsError::from)
}

fn get_folder(state: &WorkspaceState, id: &str) -> Result<ConnectionFolder, ConnectionsError> {
    state
        .with_connection(|connection| folder_by_id(connection, id))
        .map_err(ConnectionsError::from)?
        .ok_or(ConnectionsError::FolderNotFound)
}

fn load_folders(connection: &Connection) -> rusqlite::Result<Vec<ConnectionFolder>> {
    let mut statement = connection.prepare(
        "SELECT id, name, parent_id, created_at, updated_at
         FROM connection_folders ORDER BY name COLLATE NOCASE",
    )?;
    statement
        .query_map([], folder_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()
}

fn folder_by_id(connection: &Connection, id: &str) -> rusqlite::Result<Option<ConnectionFolder>> {
    connection
        .query_row(
            "SELECT id, name, parent_id, created_at, updated_at
             FROM connection_folders WHERE id = ?1",
            [id],
            folder_from_row,
        )
        .optional()
}

fn folder_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConnectionFolder> {
    Ok(ConnectionFolder {
        id: row.get(0)?,
        name: row.get(1)?,
        parent_id: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

fn load_connections(connection: &Connection) -> rusqlite::Result<Vec<ConnectionItem>> {
    let mut statement = connection.prepare(
        "SELECT id, folder_id, connection_type, name, target, port, tag,
                username, authentication, credential_reference, ssh_transport, created_at, updated_at
         FROM connections ORDER BY name COLLATE NOCASE",
    )?;
    statement
        .query_map([], connection_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()
}

fn connection_by_id(connection: &Connection, id: &str) -> rusqlite::Result<Option<ConnectionItem>> {
    connection
        .query_row(
            "SELECT id, folder_id, connection_type, name, target, port, tag,
                    username, authentication, credential_reference, ssh_transport, created_at, updated_at
             FROM connections WHERE id = ?1",
            [id],
            connection_from_row,
        )
        .optional()
}

fn get_connection(state: &WorkspaceState, id: &str) -> Result<ConnectionItem, ConnectionsError> {
    state
        .with_connection(|connection| connection_by_id(connection, id))
        .map_err(ConnectionsError::from)?
        .ok_or(ConnectionsError::ConnectionNotFound)
}

fn connection_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConnectionItem> {
    Ok(ConnectionItem {
        id: row.get(0)?,
        folder_id: row.get(1)?,
        connection_type: row.get(2)?,
        name: row.get(3)?,
        target: row.get(4)?,
        port: row.get(5)?,
        tag: row.get(6)?,
        username: row.get(7)?,
        authentication: row.get(8)?,
        credential_reference: row.get(9)?,
        ssh_transport: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

#[derive(Debug, Error)]
pub enum ConnectionsError {
    #[error("Folder name cannot be empty.")]
    EmptyFolderName,
    #[error("Folder name cannot be longer than 120 characters.")]
    FolderNameTooLong,
    #[error("A folder with this name already exists here.")]
    DuplicateFolderName,
    #[error("Folder was not found.")]
    FolderNotFound,
    #[error("Parent folder was not found.")]
    ParentNotFound,
    #[error("A folder cannot be moved into itself or one of its subfolders.")]
    CircularMove,
    #[error("Connection name cannot be empty.")]
    EmptyConnectionName,
    #[error("Connection target cannot be empty.")]
    EmptyConnectionTarget,
    #[error("Choose a valid SSH authentication method.")]
    InvalidSshAuthentication,
    #[error("Choose embedded SSH or the local OpenSSH configuration.")]
    InvalidSshTransport,
    #[error("The selected credential reference was not found.")]
    CredentialReferenceNotFound,
    #[error("Connection type must be SSH, RDP, or URL.")]
    InvalidConnectionType,
    #[error("A connection with this name already exists in this folder.")]
    DuplicateConnectionName,
    #[error("Connection was not found.")]
    ConnectionNotFound,
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::modules::{vaults::VaultsService, workspace::WorkspaceService};

    use super::*;

    #[test]
    fn manages_nested_connection_folders_and_rejects_cycles() {
        let directory = std::env::temp_dir().join(format!("pharos-connections-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).expect("create test directory");
        let state = WorkspaceState::default();
        WorkspaceService::create(
            &state,
            directory.to_string_lossy().into_owned(),
            "a-valid-test-password",
        )
        .expect("create workspace");

        let root = ConnectionsService::create_folder(&state, "Production".into(), None)
            .expect("create root");
        let child =
            ConnectionsService::create_folder(&state, "Databases".into(), Some(root.id.clone()))
                .expect("create child");
        let credential = VaultsService::create_entry(
            &state,
            "Database account".into(),
            "Accounts/Database".into(),
        )
        .expect("create credential reference");
        let connection = ConnectionsService::create_connection(
            &state,
            child.id.clone(),
            "ssh".into(),
            "Database host".into(),
            "db.example.com".into(),
            Some(22),
            "production".into(),
            Some("database".into()),
            Some("password".into()),
            Some(credential.id.clone()),
            None,
        )
        .expect("create connection");
        assert_eq!(connection.connection_type, "ssh");
        assert_eq!(
            connection.credential_reference.as_deref(),
            Some(credential.id.as_str())
        );
        let archive =
            ConnectionsService::create_folder(&state, "Archive".into(), Some(root.id.clone()))
                .expect("create destination folder");
        let moved = ConnectionsService::move_connection(&state, connection.id, archive.id.clone())
            .expect("move connection");
        assert_eq!(moved.folder_id, archive.id);
        let updated = ConnectionsService::update_connection(
            &state,
            moved.id,
            "Primary database".into(),
            "db.internal.example.com".into(),
            Some(2222),
            "critical".into(),
            Some("opc".into()),
            Some("ssh-agent".into()),
            Some(credential.id),
            None,
        )
        .expect("update connection");
        assert_eq!(updated.tag, "critical");
        assert_eq!(updated.username.as_deref(), Some("opc"));
        assert_eq!(updated.authentication.as_deref(), Some("ssh-agent"));
        let draft = ConnectionsService::create_connection(
            &state,
            archive.id.clone(),
            "ssh".into(),
            "Incomplete SSH host".into(),
            "pending.example.com".into(),
            Some(22),
            "".into(),
            None,
            Some("password".into()),
            None,
            None,
        )
        .expect("save SSH connection without a username");
        assert!(draft.username.is_none());
        ConnectionsService::update_connection(
            &state,
            draft.id,
            "Incomplete SSH host renamed".into(),
            draft.target,
            draft.port,
            "draft".into(),
            None,
            draft.authentication,
            None,
            None,
        )
        .expect("edit SSH connection without a username");
        assert!(matches!(
            ConnectionsService::move_folder(&state, root.id.clone(), Some(child.id.clone())),
            Err(ConnectionsError::CircularMove)
        ));

        let renamed = ConnectionsService::rename_folder(&state, child.id.clone(), "Servers".into())
            .expect("rename child");
        assert_eq!(renamed.name, "Servers");
        ConnectionsService::delete_folder(&state, root.id).expect("delete subtree");
        assert!(
            ConnectionsService::list_folders(&state)
                .expect("list folders")
                .is_empty()
        );

        state.lock().expect("lock workspace");
        fs::remove_dir_all(directory).expect("remove test directory");
    }
}
