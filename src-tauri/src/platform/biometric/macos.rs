use std::{sync::mpsc, time::Duration};

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::{NSError, NSString};
use objc2_local_authentication::{LABiometryType, LAContext, LAPolicy};

use super::{BiometricError, PlatformBiometricStatus, entry};

const POLICY: LAPolicy = LAPolicy::DeviceOwnerAuthenticationWithBiometrics;

pub async fn status(_alias: &str) -> Result<PlatformBiometricStatus, BiometricError> {
    let context = unsafe { LAContext::new() };
    let available = unsafe { context.canEvaluatePolicy_error(POLICY).is_ok() };
    let label = if available {
        match unsafe { context.biometryType() } {
            LABiometryType::TouchID => "Touch ID",
            LABiometryType::FaceID => "Face ID",
            LABiometryType::OpticID => "Optic ID",
            _ => "Biometric unlock",
        }
    } else {
        "Biometric unlock"
    };
    Ok(PlatformBiometricStatus {
        available,
        configured: false,
        label: label.to_owned(),
    })
}

pub async fn store(alias: &str, key: &[u8]) -> Result<(), BiometricError> {
    authenticate("Enable biometric unlock for this PHAROS workspace")?;
    entry(alias)?
        .set_secret(key)
        .map_err(|error| BiometricError::CredentialStore(error.to_string()))
}

pub async fn retrieve(alias: &str) -> Result<Vec<u8>, BiometricError> {
    authenticate("Unlock your PHAROS workspace")?;
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

fn authenticate(reason: &str) -> Result<(), BiometricError> {
    let context = unsafe { LAContext::new() };
    if unsafe { context.canEvaluatePolicy_error(POLICY).is_err() } {
        return Err(BiometricError::Unavailable);
    }

    let (sender, receiver) = mpsc::channel();
    let reply = RcBlock::new(move |success: Bool, _error: *mut NSError| {
        let _ = sender.send(success.as_bool());
    });
    let reason = NSString::from_str(reason);
    unsafe {
        context.evaluatePolicy_localizedReason_reply(POLICY, &reason, &reply);
    }

    match receiver.recv_timeout(Duration::from_secs(60)) {
        Ok(true) => Ok(()),
        Ok(false) => Err(BiometricError::AuthenticationFailed),
        Err(_) => {
            unsafe { context.invalidate() };
            Err(BiometricError::AuthenticationFailed)
        }
    }
}
