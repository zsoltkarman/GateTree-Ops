use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;
use uuid::Uuid;

use crate::modules::workspace::{WorkspaceError, WorkspaceState};

use super::model::{VaultEntry, VaultSnapshot};

const DATABASE_PATH_KEY: &str = "keepass_database_path";

pub struct VaultsService;

impl VaultsService {
    pub fn snapshot(state: &WorkspaceState) -> Result<VaultSnapshot, VaultError> {
        state
            .with_connection(|connection| {
                let database_path = connection
                    .query_row(
                        "SELECT value FROM vault_settings WHERE key = ?1",
                        [DATABASE_PATH_KEY],
                        |row| row.get(0),
                    )
                    .optional()?
                    .unwrap_or_default();
                Ok(VaultSnapshot {
                    database_path,
                    entries: load_entries(connection)?,
                })
            })
            .map_err(VaultError::from)
    }

    pub fn set_database_path(state: &WorkspaceState, path: String) -> Result<String, VaultError> {
        let path = validated_value(path, VaultError::EmptyDatabasePath)?;
        state
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO vault_settings(key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
                    params![DATABASE_PATH_KEY, path],
                )?;
                Ok(path)
            })
            .map_err(VaultError::from)
    }

    pub fn clear_database_path(state: &WorkspaceState) -> Result<(), VaultError> {
        state
            .with_connection(|connection| {
                connection.execute(
                    "DELETE FROM vault_settings WHERE key = ?1",
                    [DATABASE_PATH_KEY],
                )?;
                Ok(())
            })
            .map_err(VaultError::from)
    }

    pub fn create_entry(
        state: &WorkspaceState,
        name: String,
        entry_path: String,
    ) -> Result<VaultEntry, VaultError> {
        let name = validated_value(name, VaultError::EmptyName)?;
        let entry_path = validated_value(entry_path, VaultError::EmptyEntryPath)?;
        let id = Uuid::new_v4().to_string();
        state
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO vault_entries(id, name, entry_path) VALUES (?1, ?2, ?3)",
                    params![id, name, entry_path],
                )?;
                entry_by_id(connection, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
            })
            .map_err(VaultError::from)
    }

    pub fn update_entry(
        state: &WorkspaceState,
        id: String,
        name: String,
        entry_path: String,
    ) -> Result<VaultEntry, VaultError> {
        let name = validated_value(name, VaultError::EmptyName)?;
        let entry_path = validated_value(entry_path, VaultError::EmptyEntryPath)?;
        state
            .with_connection(|connection| {
                let changed = connection.execute(
                    "UPDATE vault_entries
                     SET name = ?2, entry_path = ?3, updated_at = CURRENT_TIMESTAMP
                     WHERE id = ?1",
                    params![id, name, entry_path],
                )?;
                if changed == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                entry_by_id(connection, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
            })
            .map_err(VaultError::from)
    }

    pub fn delete_entry(state: &WorkspaceState, id: String) -> Result<(), VaultError> {
        state
            .with_connection(|connection| {
                connection.execute("DELETE FROM vault_entries WHERE id = ?1", [id])?;
                Ok(())
            })
            .map_err(VaultError::from)
    }
}

fn validated_value(value: String, error: VaultError) -> Result<String, VaultError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(error)
    } else {
        Ok(value)
    }
}

fn load_entries(connection: &Connection) -> rusqlite::Result<Vec<VaultEntry>> {
    let mut statement = connection.prepare(
        "SELECT id, name, entry_path, created_at, updated_at
         FROM vault_entries ORDER BY name COLLATE NOCASE",
    )?;
    statement
        .query_map([], entry_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()
}

fn entry_by_id(connection: &Connection, id: &str) -> rusqlite::Result<Option<VaultEntry>> {
    connection
        .query_row(
            "SELECT id, name, entry_path, created_at, updated_at FROM vault_entries WHERE id = ?1",
            [id],
            entry_from_row,
        )
        .optional()
}

fn entry_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<VaultEntry> {
    Ok(VaultEntry {
        id: row.get(0)?,
        name: row.get(1)?,
        entry_path: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("Choose a KeePass database first.")]
    EmptyDatabasePath,
    #[error("Name cannot be empty.")]
    EmptyName,
    #[error("KeePass entry path cannot be empty.")]
    EmptyEntryPath,
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::modules::workspace::WorkspaceService;

    use super::*;

    #[test]
    fn stores_keepass_references_in_the_encrypted_workspace() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "gatetree-ops-vaults-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).expect("create test directory");
        let state = WorkspaceState::default();
        WorkspaceService::create(
            &state,
            directory.to_string_lossy().into_owned(),
            "correct horse battery staple",
        )
        .expect("create encrypted workspace");

        VaultsService::set_database_path(&state, "/secure/team.kdbx".to_owned())
            .expect("save database path");
        let entry = VaultsService::create_entry(
            &state,
            "ldap_prod".to_owned(),
            "Accounts/Production/ldap_prod".to_owned(),
        )
        .expect("create entry");
        VaultsService::update_entry(
            &state,
            entry.id.clone(),
            "ldap_icprod".to_owned(),
            "Accounts/ICPROD/ldap_icprod".to_owned(),
        )
        .expect("update entry");

        let snapshot = VaultsService::snapshot(&state).expect("load vaults");
        assert_eq!(snapshot.database_path, "/secure/team.kdbx");
        assert_eq!(snapshot.entries[0].name, "ldap_icprod");

        VaultsService::delete_entry(&state, entry.id).expect("delete entry");
        VaultsService::clear_database_path(&state).expect("clear database path");
        let snapshot = VaultsService::snapshot(&state).expect("final load");
        assert!(snapshot.database_path.is_empty());
        assert!(snapshot.entries.is_empty());

        state.lock().expect("lock workspace");
        fs::remove_dir_all(directory).expect("remove test directory");
    }
}
