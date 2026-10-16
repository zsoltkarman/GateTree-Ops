use std::path::Path;

use base64::{Engine, engine::general_purpose::STANDARD};
use pbkdf2::pbkdf2_hmac;
use rusqlite::{Connection, OpenFlags};
use sha2::Sha256;
use thiserror::Error;
use zeroize::{Zeroize, Zeroizing};

use super::migrations::run_migrations;

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
        run_migrations(&mut database.connection)?;
        Ok(database)
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
    #[error("The encrypted database already exists.")]
    AlreadyExists,
    #[error("SQLCipher is unavailable in this build.")]
    CipherUnavailable,
    #[error("The master password is incorrect or the workspace is damaged.")]
    InvalidPassword,
    #[error("Encrypted database error: {0}")]
    Database(#[from] rusqlite::Error),
}
