use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

use tauri::State;
use zeroize::Zeroizing;

use crate::modules::workspace::WorkspaceState;

use super::{model::BackupSummary, service::BackupService};

#[tauri::command]
pub fn export_backup(
    state: State<'_, WorkspaceState>,
    path: String,
) -> Result<BackupSummary, String> {
    let (encrypted, summary) = BackupService::export_encrypted(&state)?;
    write_encrypted_backup(Path::new(&path), &encrypted)?;
    Ok(summary)
}

#[tauri::command]
pub fn import_backup(
    state: State<'_, WorkspaceState>,
    path: String,
    password: Option<String>,
) -> Result<BackupSummary, String> {
    let password = Zeroizing::new(password.unwrap_or_default());
    let bytes =
        Zeroizing::new(fs::read(path).map_err(|error| format!("Could not read backup: {error}"))?);
    BackupService::import_bytes(&state, &bytes, &password)
}

fn write_encrypted_backup(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let temporary = parent.join(format!(".gatetree-backup-{}.tmp", uuid::Uuid::new_v4()));
    let mut created = false;
    let result = (|| -> std::io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        created = true;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() && created {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| format!("Could not write backup: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_file_is_replaced_only_after_successful_write_and_temporary_files_are_cleaned() {
        let directory =
            std::env::temp_dir().join(format!("gatetree-backup-file-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("backup.gtbak");
        write_encrypted_backup(&path, b"encrypted first").unwrap();
        write_encrypted_backup(&path, b"encrypted second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"encrypted second");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let blocked = directory.join("existing-directory");
        fs::create_dir(&blocked).unwrap();
        assert!(write_encrypted_backup(&blocked, b"encrypted data").is_err());
        assert!(blocked.is_dir());
        assert_eq!(fs::read(&path).unwrap(), b"encrypted second");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 2);
        fs::remove_dir_all(directory).unwrap();
    }
}
