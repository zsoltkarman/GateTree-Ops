use std::{fs, path::Path};

use sha2::{Digest, Sha256};
use thiserror::Error;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

const KEYRING_SERVICE: &str = "com.zsoltkarman.gatetreeops.workspace";
const CONFIGURED_MARKER: &str = ".gatetree-ops-biometric";
const LEGACY_MARKER: &str = ".pharos-biometric";

#[derive(Debug)]
pub struct PlatformBiometricStatus {
    pub available: bool,
    pub configured: bool,
    pub label: String,
}

pub async fn status(path: &Path) -> Result<PlatformBiometricStatus, BiometricError> {
    let mut status = implementation::status(&alias(path)).await?;
    status.configured = configured_marker(path).is_file();
    Ok(status)
}

pub async fn store(path: &Path, key: &[u8]) -> Result<(), BiometricError> {
    let credential_alias = alias(path);
    implementation::store(&credential_alias, key).await?;
    if let Err(error) = write_configured_marker(path) {
        let _ = implementation::delete(&credential_alias).await;
        return Err(error);
    }
    Ok(())
}

pub async fn retrieve(path: &Path) -> Result<Vec<u8>, BiometricError> {
    if !configured_marker(path).is_file() {
        return Err(BiometricError::NotConfigured);
    }
    let result = implementation::retrieve(&alias(path)).await;
    if matches!(result, Err(BiometricError::NotConfigured)) {
        let _ = remove_configured_marker(path);
    }
    result
}

pub async fn delete(path: &Path) -> Result<(), BiometricError> {
    implementation::delete(&alias(path)).await?;
    remove_configured_marker(path)
}

fn alias(path: &Path) -> String {
    let digest = Sha256::digest(path.to_string_lossy().as_bytes());
    let hash: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    if configured_marker(path).file_name().and_then(|p| p.to_str()) == Some(LEGACY_MARKER) {
        format!("legacy-pharos:{hash}")
    } else {
        hash
    }
}

fn configured_marker(path: &Path) -> std::path::PathBuf {
    let current = path.join(CONFIGURED_MARKER);
    let legacy = path.join(LEGACY_MARKER);
    if !current.exists() && legacy.is_file() {
        legacy
    } else {
        current
    }
}

fn write_configured_marker(path: &Path) -> Result<(), BiometricError> {
    let marker = configured_marker(path);
    fs::write(&marker, b"1").map_err(|error| BiometricError::Marker(error.to_string()))?;
    restrict_marker_permissions(&marker)
}

fn remove_configured_marker(path: &Path) -> Result<(), BiometricError> {
    for name in [CONFIGURED_MARKER, LEGACY_MARKER] {
        match fs::remove_file(path.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(BiometricError::Marker(error.to_string())),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn restrict_marker_permissions(path: &Path) -> Result<(), BiometricError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| BiometricError::Marker(error.to_string()))
}

#[cfg(not(unix))]
fn restrict_marker_permissions(_path: &Path) -> Result<(), BiometricError> {
    Ok(())
}

#[cfg(target_os = "macos")]
use macos as implementation;
#[cfg(target_os = "windows")]
use windows as implementation;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod implementation {
    use super::{BiometricError, PlatformBiometricStatus};

    pub async fn status(_alias: &str) -> Result<PlatformBiometricStatus, BiometricError> {
        Ok(PlatformBiometricStatus {
            available: false,
            configured: false,
            label: "Biometric unlock".to_owned(),
        })
    }
    pub async fn store(_alias: &str, _key: &[u8]) -> Result<(), BiometricError> {
        Err(BiometricError::Unavailable)
    }
    pub async fn retrieve(_alias: &str) -> Result<Vec<u8>, BiometricError> {
        Err(BiometricError::Unavailable)
    }
    pub async fn delete(_alias: &str) -> Result<(), BiometricError> {
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum BiometricError {
    #[error("Biometric authentication is not available on this device.")]
    Unavailable,
    #[error("Biometric authentication was not completed.")]
    AuthenticationFailed,
    #[error("No biometric unlock key is saved for this workspace.")]
    NotConfigured,
    #[error("Operating-system credential storage error: {0}")]
    CredentialStore(String),
    #[error("Biometric configuration marker error: {0}")]
    Marker(String),
    #[cfg(target_os = "windows")]
    #[error("Biometric provider error: {0}")]
    Provider(String),
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) fn entry(alias: &str) -> Result<keyring::Entry, BiometricError> {
    let (service, alias) = if let Some(legacy) = alias.strip_prefix("legacy-pharos:") {
        ("com.zsoltkarman.pharos.workspace", legacy)
    } else {
        (KEYRING_SERVICE, alias)
    };
    keyring::Entry::new(service, alias)
        .map_err(|error| BiometricError::CredentialStore(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_marker_keeps_its_keyring_alias_and_new_enrollment_uses_new_namespace() {
        let directory =
            std::env::temp_dir().join(format!("gatetree-ops-biometric-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let new_alias = alias(&directory);
        assert!(!new_alias.starts_with("legacy-pharos:"));
        fs::write(directory.join(LEGACY_MARKER), "1").unwrap();
        assert_eq!(alias(&directory), format!("legacy-pharos:{new_alias}"));
        assert_eq!(configured_marker(&directory), directory.join(LEGACY_MARKER));
        remove_configured_marker(&directory).unwrap();
        write_configured_marker(&directory).unwrap();
        assert_eq!(
            configured_marker(&directory),
            directory.join(CONFIGURED_MARKER)
        );
        assert_eq!(alias(&directory), new_alias);
        fs::remove_dir_all(directory).unwrap();
    }
}
