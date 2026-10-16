use windows::{
    Security::Credentials::UI::{
        UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
    },
    core::HSTRING,
};

use super::{BiometricError, PlatformBiometricStatus, entry};

pub async fn status(_alias: &str) -> Result<PlatformBiometricStatus, BiometricError> {
    let availability = UserConsentVerifier::CheckAvailabilityAsync()
        .map_err(provider_error)?
        .await
        .map_err(provider_error)?;
    Ok(PlatformBiometricStatus {
        available: availability == UserConsentVerifierAvailability::Available,
        configured: false,
        label: "Windows Hello".to_owned(),
    })
}

pub async fn store(alias: &str, key: &[u8]) -> Result<(), BiometricError> {
    authenticate("Enable Windows Hello unlock for this PHAROS workspace").await?;
    entry(alias)?
        .set_secret(key)
        .map_err(|error| BiometricError::CredentialStore(error.to_string()))
}

pub async fn retrieve(alias: &str) -> Result<Vec<u8>, BiometricError> {
    authenticate("Unlock your PHAROS workspace").await?;
    entry(alias)?.get_secret().map_err(|error| match error {
        keyring::Error::NoEntry => BiometricError::NotConfigured,
        other => BiometricError::CredentialStore(other.to_string()),
    })
}

pub async fn delete(alias: &str) -> Result<(), BiometricError> {
    match entry(alias)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(BiometricError::CredentialStore(error.to_string())),
    }
}

async fn authenticate(reason: &str) -> Result<(), BiometricError> {
    let result = UserConsentVerifier::RequestVerificationAsync(&HSTRING::from(reason))
        .map_err(provider_error)?
        .await
        .map_err(provider_error)?;
    if result == UserConsentVerificationResult::Verified {
        Ok(())
    } else if matches!(
        result,
        UserConsentVerificationResult::DeviceNotPresent
            | UserConsentVerificationResult::NotConfiguredForUser
            | UserConsentVerificationResult::DisabledByPolicy
    ) {
        Err(BiometricError::Unavailable)
    } else {
        Err(BiometricError::AuthenticationFailed)
    }
}

fn provider_error(error: impl std::fmt::Display) -> BiometricError {
    BiometricError::Provider(error.to_string())
}
