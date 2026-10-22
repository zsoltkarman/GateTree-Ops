use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

use getrandom::fill;
use thiserror::Error;
use zeroize::Zeroizing;

use crate::storage::{EncryptedDatabase, StorageError, derive_key};

use super::model::{WorkspaceInspection, WorkspaceKind, WorkspaceSessionView};

const DATABASE_FILE: &str = "gatetree-ops.sqlite";
const SALT_FILE: &str = "gatetree-ops.sqlite.salt";
// Legacy workspace files stay in place; never mix a database with a different salt.
const LEGACY_DATABASE_FILE: &str = "pharos.sqlite";
const LEGACY_SALT_FILE: &str = "pharos.sqlite.salt";
const SALT_SIZE: usize = 16;
#[cfg(debug_assertions)]
const DEVELOPMENT_KEY_FILE: &str = ".gatetree-ops-dev-unlock";

#[derive(Default)]
pub struct WorkspaceState {
    active: Mutex<Option<ActiveWorkspace>>,
}

struct ActiveWorkspace {
    database: EncryptedDatabase,
    key: Zeroizing<Vec<u8>>,
    path: PathBuf,
    keepass: Option<KeePassSession>,
}

struct KeePassSession {
    agent_pid: u32,
    database_path: String,
    password: Zeroizing<String>,
}

impl WorkspaceState {
    pub fn lock(&self) -> Result<(), WorkspaceError> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| WorkspaceError::StatePoisoned)?;
        *active = None;
        Ok(())
    }

    fn replace(
        &self,
        database: EncryptedDatabase,
        key: Zeroizing<Vec<u8>>,
        path: PathBuf,
    ) -> Result<(), WorkspaceError> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| WorkspaceError::StatePoisoned)?;
        *active = Some(ActiveWorkspace {
            database,
            key,
            path,
            keepass: None,
        });
        Ok(())
    }

    pub(crate) fn cache_keepass_password(
        &self,
        agent_pid: u32,
        database_path: String,
        password: Zeroizing<String>,
    ) -> Result<(), WorkspaceError> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| WorkspaceError::StatePoisoned)?;
        let active = active.as_mut().ok_or(WorkspaceError::NotUnlocked)?;
        active.keepass = Some(KeePassSession {
            agent_pid,
            database_path,
            password,
        });
        Ok(())
    }

    pub(crate) fn keepass_password(
        &self,
        agent_pid: u32,
        database_path: &str,
    ) -> Result<Option<Zeroizing<String>>, WorkspaceError> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| WorkspaceError::StatePoisoned)?;
        let active = active.as_mut().ok_or(WorkspaceError::NotUnlocked)?;
        if active.keepass.as_ref().is_some_and(|session| {
            session.agent_pid != agent_pid || session.database_path != database_path
        }) {
            active.keepass = None;
        }
        Ok(active
            .keepass
            .as_ref()
            .map(|session| session.password.clone()))
    }

    pub fn biometric_material(&self) -> Result<(PathBuf, Zeroizing<Vec<u8>>), WorkspaceError> {
        let active = self
            .active
            .lock()
            .map_err(|_| WorkspaceError::StatePoisoned)?;
        let active = active.as_ref().ok_or(WorkspaceError::NotUnlocked)?;
        let _keep_database_alive = &active.database;
        Ok((active.path.clone(), Zeroizing::new(active.key.to_vec())))
    }

    pub(crate) fn backup_before_update(&self) -> Result<Option<PathBuf>, WorkspaceError> {
        let active = self
            .active
            .lock()
            .map_err(|_| WorkspaceError::StatePoisoned)?;
        let Some(active) = active.as_ref() else {
            return Ok(None);
        };
        active
            .database
            .backup(&database_path(&active.path), &active.key, "before-update")
            .map(Some)
            .map_err(WorkspaceError::from)
    }

    pub(crate) fn backup_encryption_material(&self) -> Result<(Zeroizing<Vec<u8>>, Vec<u8>), WorkspaceError> {
        let active = self.active.lock().map_err(|_| WorkspaceError::StatePoisoned)?;
        let active = active.as_ref().ok_or(WorkspaceError::NotUnlocked)?;
        let salt = fs::read(salt_path(&active.path))?;
        if salt.len() != SALT_SIZE { return Err(WorkspaceError::InvalidSalt); }
        Ok((Zeroizing::new(active.key.to_vec()), salt))
    }

    pub fn with_connection<T>(
        &self,
        operation: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T>,
    ) -> Result<T, WorkspaceError> {
        let active = self
            .active
            .lock()
            .map_err(|_| WorkspaceError::StatePoisoned)?;
        let active = active.as_ref().ok_or(WorkspaceError::NotUnlocked)?;
        operation(active.database.connection())
            .map_err(StorageError::from)
            .map_err(WorkspaceError::from)
    }
}

pub struct WorkspaceService;

impl WorkspaceService {
    pub fn inspect(path: String) -> Result<WorkspaceInspection, WorkspaceError> {
        let directory = validate_directory(&path)?;
        let database_exists = database_path(&directory).exists();
        let salt_exists = salt_path(&directory).exists();

        let kind = match (database_exists, salt_exists) {
            (false, false) => WorkspaceKind::New,
            (true, true) => {
                validate_existing_files(&directory)?;
                WorkspaceKind::Existing
            }
            _ => return Err(WorkspaceError::IncompleteWorkspace),
        };

        Ok(WorkspaceInspection {
            name: workspace_name(&directory),
            path: directory.to_string_lossy().into_owned(),
            kind,
        })
    }

    pub fn create(
        state: &WorkspaceState,
        path: String,
        password: &str,
    ) -> Result<WorkspaceSessionView, WorkspaceError> {
        if password.chars().count() < 12 {
            return Err(WorkspaceError::WeakPassword);
        }
        let inspection = Self::inspect(path)?;
        if !matches!(inspection.kind, WorkspaceKind::New) {
            return Err(WorkspaceError::AlreadyExists);
        }
        let directory = PathBuf::from(&inspection.path);
        let salt_file = salt_path(&directory);
        let database_file = database_path(&directory);
        let mut salt = [0_u8; SALT_SIZE];
        fill(&mut salt).map_err(|_| WorkspaceError::RandomnessUnavailable)?;

        let create_result = (|| {
            write_new_salt(&salt_file, &salt)?;
            let key = derive_key(password, &salt);
            let database = EncryptedDatabase::create_with_key(&database_file, &key)?;
            restrict_file_permissions(&salt_file)?;
            restrict_file_permissions(&database_file)?;
            let cipher_version = database.cipher_version().to_owned();
            state.replace(database, key, directory.clone())?;
            Ok(WorkspaceSessionView {
                path: inspection.path,
                name: inspection.name,
                cipher_version,
            })
        })();

        if create_result.is_err() {
            let _ = fs::remove_file(&database_file);
            let _ = fs::remove_file(&salt_file);
        }
        create_result
    }

    pub fn unlock(
        state: &WorkspaceState,
        path: String,
        password: &str,
    ) -> Result<WorkspaceSessionView, WorkspaceError> {
        let inspection = Self::inspect(path)?;
        if !matches!(inspection.kind, WorkspaceKind::Existing) {
            return Err(WorkspaceError::DoesNotExist);
        }
        let directory = PathBuf::from(&inspection.path);
        let salt = fs::read(salt_path(&directory))?;
        let key = derive_key(password, &salt);
        let database = EncryptedDatabase::open_with_key(&database_path(&directory), &key)?;
        let cipher_version = database.cipher_version().to_owned();
        state.replace(database, key, directory)?;
        Ok(WorkspaceSessionView {
            path: inspection.path,
            name: inspection.name,
            cipher_version,
        })
    }

    pub fn unlock_with_key(
        state: &WorkspaceState,
        path: String,
        key: Vec<u8>,
    ) -> Result<WorkspaceSessionView, WorkspaceError> {
        if key.len() != 32 {
            return Err(WorkspaceError::InvalidBiometricKey);
        }
        let inspection = Self::inspect(path)?;
        if !matches!(inspection.kind, WorkspaceKind::Existing) {
            return Err(WorkspaceError::DoesNotExist);
        }
        let directory = PathBuf::from(&inspection.path);
        let key = Zeroizing::new(key);
        let database = EncryptedDatabase::open_with_key(&database_path(&directory), &key)?;
        let cipher_version = database.cipher_version().to_owned();
        state.replace(database, key, directory)?;
        Ok(WorkspaceSessionView {
            path: inspection.path,
            name: inspection.name,
            cipher_version,
        })
    }

    #[cfg(debug_assertions)]
    pub fn cache_development_key(state: &WorkspaceState) -> Result<(), WorkspaceError> {
        let (path, key) = state.biometric_material()?;
        let key_file = development_key_path(&path);
        fs::write(&key_file, key.as_slice())?;
        restrict_file_permissions(&key_file)
    }

    #[cfg(debug_assertions)]
    pub fn unlock_for_development(
        state: &WorkspaceState,
        path: String,
    ) -> Result<WorkspaceSessionView, WorkspaceError> {
        let inspection = Self::inspect(path)?;
        let key = fs::read(development_key_path(Path::new(&inspection.path)))?;
        Self::unlock_with_key(state, inspection.path, key)
    }
}

fn validate_directory(path: &str) -> Result<PathBuf, WorkspaceError> {
    let candidate = PathBuf::from(path);
    if !candidate.is_dir() {
        return Err(WorkspaceError::DirectoryUnavailable);
    }
    let directory = candidate.canonicalize()?;
    let probe = directory.join(".gatetree-ops-write-test");
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)?
        .write_all(b"gatetree-ops")?;
    fs::remove_file(probe)?;
    Ok(directory)
}

fn validate_existing_files(directory: &Path) -> Result<(), WorkspaceError> {
    let database = database_path(directory);
    let salt = salt_path(directory);
    if !database.is_file() || fs::metadata(&database)?.len() == 0 {
        return Err(WorkspaceError::InvalidDatabase);
    }
    if !salt.is_file() || fs::metadata(&salt)?.len() != SALT_SIZE as u64 {
        return Err(WorkspaceError::InvalidSalt);
    }
    Ok(())
}

fn write_new_salt(path: &Path, salt: &[u8]) -> Result<(), WorkspaceError> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(salt)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn restrict_file_permissions(path: &Path) -> Result<(), WorkspaceError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_file_permissions(_path: &Path) -> Result<(), WorkspaceError> {
    // Windows applies the selected workspace directory's inherited ACL.
    Ok(())
}

fn uses_legacy_files(directory: &Path) -> bool {
    !directory.join(DATABASE_FILE).exists()
        && !directory.join(SALT_FILE).exists()
        && (directory.join(LEGACY_DATABASE_FILE).exists()
            || directory.join(LEGACY_SALT_FILE).exists())
}
fn database_path(directory: &Path) -> PathBuf {
    directory.join(if uses_legacy_files(directory) {
        LEGACY_DATABASE_FILE
    } else {
        DATABASE_FILE
    })
}
fn salt_path(directory: &Path) -> PathBuf {
    directory.join(if uses_legacy_files(directory) {
        LEGACY_SALT_FILE
    } else {
        SALT_FILE
    })
}
#[cfg(debug_assertions)]
fn development_key_path(directory: &Path) -> PathBuf {
    let current = directory.join(DEVELOPMENT_KEY_FILE);
    let legacy = directory.join(".pharos-dev-unlock");
    if !current.exists() && legacy.is_file() {
        legacy
    } else {
        current
    }
}
fn workspace_name(directory: &Path) -> String {
    directory
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("GateTree-Ops Workspace")
        .to_owned()
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("The selected workspace folder is unavailable.")]
    DirectoryUnavailable,
    #[error(
        "This folder contains an incomplete GateTree-Ops workspace. Restore the matching database and salt files, or choose an empty folder."
    )]
    IncompleteWorkspace,
    #[error("The workspace database is invalid or empty.")]
    InvalidDatabase,
    #[error("The workspace encryption salt is invalid.")]
    InvalidSalt,
    #[error("A GateTree-Ops workspace already exists in this folder.")]
    AlreadyExists,
    #[error("No GateTree-Ops workspace exists in this folder.")]
    DoesNotExist,
    #[error("Use a master password containing at least 12 characters.")]
    WeakPassword,
    #[error("Secure random data could not be generated.")]
    RandomnessUnavailable,
    #[error("The workspace state is unavailable.")]
    StatePoisoned,
    #[error("Unlock the workspace before enabling biometric access.")]
    NotUnlocked,
    #[error("The saved biometric unlock key is invalid.")]
    InvalidBiometricKey,
    #[error("Workspace file error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn test_directory() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "gatetree-ops-workspace-test-{}-{unique}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).expect("create test workspace");
        directory
    }

    #[test]
    fn opens_legacy_encrypted_workspace_and_debug_key_without_renaming_or_mixing_files() {
        let directory = std::env::temp_dir().join(format!(
            "gatetree-ops-legacy-workspace-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).unwrap();
        let state = WorkspaceState::default();
        WorkspaceService::create(
            &state,
            directory.to_string_lossy().into_owned(),
            "correct horse battery staple",
        )
        .unwrap();
        state
            .with_connection(|c| {
                c.execute(
                    "INSERT INTO workspace_metadata(key,value) VALUES ('rename-probe','preserved')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        #[cfg(debug_assertions)]
        WorkspaceService::cache_development_key(&state).unwrap();
        state.lock().unwrap();
        assert!(directory.join(DATABASE_FILE).is_file());
        fs::rename(
            directory.join(DATABASE_FILE),
            directory.join(LEGACY_DATABASE_FILE),
        )
        .unwrap();
        fs::rename(directory.join(SALT_FILE), directory.join(LEGACY_SALT_FILE)).unwrap();
        #[cfg(debug_assertions)]
        fs::rename(
            directory.join(DEVELOPMENT_KEY_FILE),
            directory.join(".pharos-dev-unlock"),
        )
        .unwrap();
        assert!(matches!(
            WorkspaceService::inspect(directory.to_string_lossy().into_owned())
                .unwrap()
                .kind,
            WorkspaceKind::Existing
        ));
        WorkspaceService::unlock(
            &state,
            directory.to_string_lossy().into_owned(),
            "correct horse battery staple",
        )
        .unwrap();
        let value: String = state
            .with_connection(|c| {
                c.query_row(
                    "SELECT value FROM workspace_metadata WHERE key='rename-probe'",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap();
        assert_eq!(value, "preserved");
        assert!(!directory.join(DATABASE_FILE).exists());
        state.lock().unwrap();
        #[cfg(debug_assertions)]
        {
            WorkspaceService::unlock_for_development(
                &state,
                directory.to_string_lossy().into_owned(),
            )
            .unwrap();
            state.lock().unwrap();
        }
        // A partial new-format pair must never borrow the legacy database or salt.
        fs::write(directory.join(SALT_FILE), [0_u8; SALT_SIZE]).unwrap();
        assert!(matches!(
            WorkspaceService::inspect(directory.to_string_lossy().into_owned()),
            Err(WorkspaceError::IncompleteWorkspace)
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn creates_locks_and_reopens_an_encrypted_workspace() {
        let directory = test_directory();
        let path = directory.to_string_lossy().into_owned();
        let state = WorkspaceState::default();

        let inspection = WorkspaceService::inspect(path.clone()).expect("inspect empty workspace");
        assert!(matches!(inspection.kind, WorkspaceKind::New));

        let created =
            WorkspaceService::create(&state, path.clone(), "correct horse battery staple")
                .expect("create workspace");
        assert!(created.cipher_version.starts_with('4'));
        assert_ne!(
            &fs::read(database_path(&directory)).expect("read database")[..16],
            b"SQLite format 3\0"
        );

        state
            .cache_keepass_password(
                100,
                "vault.kdbx".into(),
                Zeroizing::new("session-secret".into()),
            )
            .unwrap();
        for _ in 0..2 {
            assert_eq!(
                state
                    .keepass_password(100, "vault.kdbx")
                    .unwrap()
                    .as_deref()
                    .map(String::as_str),
                Some("session-secret")
            );
        }
        assert!(state.keepass_password(200, "vault.kdbx").unwrap().is_none());
        assert!(state.keepass_password(100, "vault.kdbx").unwrap().is_none());
        state
            .cache_keepass_password(
                100,
                "vault.kdbx".into(),
                Zeroizing::new("session-secret".into()),
            )
            .unwrap();
        assert!(state.keepass_password(100, "other.kdbx").unwrap().is_none());
        state
            .cache_keepass_password(
                100,
                "vault.kdbx".into(),
                Zeroizing::new("session-secret".into()),
            )
            .unwrap();
        state.lock().expect("lock workspace");
        let reopened =
            WorkspaceService::unlock(&state, path.clone(), "correct horse battery staple")
                .expect("reopen workspace");
        assert!(state.keepass_password(100, "vault.kdbx").unwrap().is_none());
        assert_eq!(
            reopened.path,
            directory.canonicalize().unwrap().to_string_lossy()
        );

        state.lock().expect("lock workspace");
        assert!(WorkspaceService::unlock(&state, path, "this password is wrong").is_err());
        fs::remove_dir_all(directory).expect("remove test workspace");
    }

    #[test]
    fn rejects_short_master_passwords_without_creating_files() {
        let directory = test_directory();
        let path = directory.to_string_lossy().into_owned();
        let state = WorkspaceState::default();

        assert!(matches!(
            WorkspaceService::create(&state, path, "too-short"),
            Err(WorkspaceError::WeakPassword)
        ));
        assert!(!database_path(&directory).exists());
        assert!(!salt_path(&directory).exists());
        fs::remove_dir_all(directory).expect("remove test workspace");
    }
}
