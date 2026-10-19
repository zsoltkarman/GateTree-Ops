use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use pbkdf2::pbkdf2_hmac;
use rusqlite::{Connection, OpenFlags};
use sha2::Sha256;
use thiserror::Error;
use zeroize::{Zeroize, Zeroizing};

use super::migrations::{LATEST_SCHEMA_VERSION, run_migrations};

const KDF_ITERATIONS: u32 = 310_000;

pub struct EncryptedDatabase {
    connection: Connection,
    cipher_version: String,
}

impl EncryptedDatabase {
    pub fn create_with_key(path: &Path, raw_key: &[u8]) -> Result<Self, StorageError> {
        if path.exists() {
            return Err(StorageError::AlreadyExists);
        }
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )?;
        let mut database = Self::configure(connection, raw_key)?;
        run_migrations(&mut database.connection)?;
        Ok(database)
    }

    pub fn open_with_key(path: &Path, raw_key: &[u8]) -> Result<Self, StorageError> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )?;
        let mut database = Self::configure(connection, raw_key)?;
        let has_migrations: bool = database.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_migrations')", [], |r| r.get(0))?;
        if has_migrations {
            let current: i64 = database.connection.query_row(
                "SELECT COALESCE(MAX(version),0) FROM schema_migrations",
                [],
                |r| r.get(0),
            )?;
            if current > LATEST_SCHEMA_VERSION {
                return Err(StorageError::NewerSchema);
            }
            if current < LATEST_SCHEMA_VERSION {
                database.backup(path, raw_key, "before-migration")?;
            }
        }
        run_migrations(&mut database.connection)?;
        Ok(database)
    }

    /// A consistent online SQLCipher backup, including committed WAL contents.
    /// The destination has the same encryption key; plaintext is never written.
    pub(crate) fn backup(
        &self,
        path: &Path,
        raw_key: &[u8],
        reason: &str,
    ) -> Result<PathBuf, StorageError> {
        let parent = path.parent().ok_or(StorageError::InvalidBackupPath)?;
        let destination = parent
            .join(".gatetree-ops-backups")
            .join(format!("{reason}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&destination)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&destination, fs::Permissions::from_mode(0o700))?;
        }
        let result = (|| {
            let name = path.file_name().ok_or(StorageError::InvalidBackupPath)?;
            let copy = destination.join(name);
            let mut target = Connection::open(&copy)?;
            let mut key = STANDARD.encode(raw_key);
            let keyed = target.pragma_update(None, "key", &key);
            key.zeroize();
            keyed?;
            {
                let backup = rusqlite::backup::Backup::new(&self.connection, &mut target)?;
                let deadline = Instant::now() + Duration::from_secs(120);
                loop {
                    if matches!(backup.step(128)?, rusqlite::backup::StepResult::Done) {
                        break;
                    }
                    if Instant::now() >= deadline {
                        return Err(StorageError::BackupTimeout);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            drop(target);
            let salt = path.with_file_name(format!("{}.salt", name.to_string_lossy()));
            fs::copy(
                &salt,
                destination.join(salt.file_name().ok_or(StorageError::InvalidBackupPath)?),
            )?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                for entry in fs::read_dir(&destination)? {
                    fs::set_permissions(entry?.path(), fs::Permissions::from_mode(0o600))?;
                }
            }
            Ok(destination.clone())
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&destination);
        }
        result
    }

    pub fn cipher_version(&self) -> &str {
        &self.cipher_version
    }

    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }

    fn configure(connection: Connection, raw_key: &[u8]) -> Result<Self, StorageError> {
        let mut key = STANDARD.encode(raw_key);

        connection.pragma_update(None, "key", &key)?;
        key.zeroize();
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;

        let cipher_version: String = connection
            .query_row("PRAGMA cipher_version;", [], |row| row.get(0))
            .map_err(|_| StorageError::CipherUnavailable)?;
        if cipher_version.trim().is_empty() {
            return Err(StorageError::CipherUnavailable);
        }

        connection
            .query_row("SELECT count(*) FROM sqlite_master;", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|_| StorageError::InvalidPassword)?;

        Ok(Self {
            connection,
            cipher_version,
        })
    }
}

pub fn derive_key(password: &str, salt: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut key = Zeroizing::new(vec![0_u8; 32]);
    pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, KDF_ITERATIONS, &mut key);
    key
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error(
        "The workspace was created by a newer application. Install that version or restore a matching backup."
    )]
    NewerSchema,
    #[error("Workspace backup timed out. Close other workspace users and retry the update.")]
    BackupTimeout,
    #[error("Invalid workspace backup path.")]
    InvalidBackupPath,
    #[error("Could not save the encrypted workspace backup: {0}")]
    BackupIo(#[from] std::io::Error),
    #[error("The encrypted database already exists.")]
    AlreadyExists,
    #[error("SQLCipher is unavailable in this build.")]
    CipherUnavailable,
    #[error("The master password is incorrect or the workspace is damaged.")]
    InvalidPassword,
    #[error("Encrypted database error: {0}")]
    Database(#[from] rusqlite::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    fn workspace() -> (PathBuf, PathBuf, Vec<u8>) {
        let root =
            std::env::temp_dir().join(format!("gatetree-ops-upgrade-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("gatetree-ops.sqlite");
        fs::write(root.join("gatetree-ops.sqlite.salt"), [42u8; 16]).unwrap();
        (root, path, vec![7; 32])
    }
    #[test]
    fn online_backup_keeps_wal_data_encrypted_and_includes_the_salt() {
        let (root, path, key) = workspace();
        let database = EncryptedDatabase::create_with_key(&path, &key).unwrap();
        database.connection.execute("INSERT INTO workspace_metadata(key,value) VALUES ('backup-test','private deployment data')",[]).unwrap();
        let backup = database.backup(&path, &key, "before-update").unwrap();
        assert_eq!(
            fs::read(backup.join("gatetree-ops.sqlite.salt")).unwrap(),
            vec![42u8; 16]
        );
        let bytes = fs::read(backup.join("gatetree-ops.sqlite")).unwrap();
        assert!(!bytes.starts_with(b"SQLite format 3"));
        assert!(!bytes.windows(23).any(|w| w == b"private deployment data"));
        let restored =
            EncryptedDatabase::open_with_key(&backup.join("gatetree-ops.sqlite"), &key).unwrap();
        assert_eq!(
            restored
                .connection
                .query_row(
                    "SELECT value FROM workspace_metadata WHERE key='backup-test'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "private deployment data"
        );
        drop(restored);
        drop(database);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn migration_backs_up_old_schema_once_and_refuses_downgrading_newer_schema() {
        let (root, path, key) = workspace();
        let database = EncryptedDatabase::create_with_key(&path, &key).unwrap();
        database
            .connection
            .execute_batch(
                "DROP TABLE quick_launch_items; ALTER TABLE connections DROP COLUMN icon; ALTER TABLE connections DROP COLUMN credential_mode; ALTER TABLE connection_folders DROP COLUMN credential_mode; ALTER TABLE connection_folders DROP COLUMN credential_reference; DELETE FROM schema_migrations WHERE version>=20;",
            )
            .unwrap();
        drop(database);
        let database = EncryptedDatabase::open_with_key(&path, &key).unwrap();
        let backups = fs::read_dir(root.join(".gatetree-ops-backups"))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(backups.len(), 1);
        let backup_connection =
            Connection::open(backups[0].path().join("gatetree-ops.sqlite")).unwrap();
        let old = EncryptedDatabase::configure(backup_connection, &key).unwrap();
        assert_eq!(
            old.connection
                .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            19
        );
        drop(old);
        drop(database);
        let database = EncryptedDatabase::open_with_key(&path, &key).unwrap();
        assert_eq!(
            fs::read_dir(root.join(".gatetree-ops-backups"))
                .unwrap()
                .count(),
            1
        );
        database
            .connection
            .execute("INSERT INTO schema_migrations(version) VALUES (999)", [])
            .unwrap();
        drop(database);
        assert!(matches!(
            EncryptedDatabase::open_with_key(&path, &key),
            Err(StorageError::NewerSchema)
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
