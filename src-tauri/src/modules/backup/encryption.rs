use pbkdf2::pbkdf2_hmac;
use ring::{aead, hkdf};
use sha2::Sha256;
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"GTOBACKP";
const VERSION: u8 = 1;
const SALT_LEN: usize = 16;
const HEADER_LEN: usize = 8 + 1 + SALT_LEN + SALT_LEN + 12;
// This is the workspace password derivation used by backup format version 1.
const WORKSPACE_ITERATIONS_V1: u32 = 310_000;

fn backup_key(workspace_key: &[u8], salt: &[u8]) -> Result<aead::LessSafeKey, String> {
    let extracted = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(workspace_key);
    let info = [b"gatetree-ops-encrypted-backup-v1".as_slice()];
    let expanded = extracted
        .expand(&info, hkdf::HKDF_SHA256)
        .map_err(|_| "Could not derive backup key.")?;
    let mut key = Zeroizing::new([0_u8; 32]);
    expanded
        .fill(key.as_mut())
        .map_err(|_| "Could not derive backup key.")?;
    let key = aead::UnboundKey::new(&aead::AES_256_GCM, key.as_ref())
        .map_err(|_| "Could not initialize backup encryption.")?;
    Ok(aead::LessSafeKey::new(key))
}

pub(super) fn encrypt(
    json: &str,
    workspace_key: &[u8],
    workspace_salt: &[u8],
) -> Result<Vec<u8>, String> {
    if workspace_key.len() != 32 || workspace_salt.len() != SALT_LEN {
        return Err("Invalid workspace encryption material.".into());
    }
    // Each backup gets an independent derived key and a fresh 96-bit nonce.
    let mut random = [0_u8; SALT_LEN + 12];
    getrandom::fill(&mut random).map_err(|_| "Could not generate backup encryption randomness.")?;
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    header.push(VERSION);
    header.extend_from_slice(workspace_salt);
    header.extend_from_slice(&random);
    let key = backup_key(workspace_key, &random[..SALT_LEN])?;
    let nonce = aead::Nonce::try_assume_unique_for_key(&random[SALT_LEN..])
        .map_err(|_| "Invalid backup nonce.")?;
    let mut encrypted = Zeroizing::new(json.as_bytes().to_vec());
    key.seal_in_place_append_tag(nonce, aead::Aad::from(header.as_slice()), &mut *encrypted)
        .map_err(|_| "Could not encrypt backup.")?;
    header.extend_from_slice(&encrypted);
    Ok(header)
}

pub(super) fn decode(bytes: &[u8], password: &str) -> Result<Zeroizing<String>, String> {
    if !bytes.starts_with(MAGIC) {
        // Keep existing JSON backups importable; exports are always encrypted.
        if bytes
            .iter()
            .copied()
            .find(|byte| !byte.is_ascii_whitespace())
            != Some(b'{')
        {
            return Err("This is not a GateTree-Ops backup file.".into());
        }
        return String::from_utf8(bytes.to_vec())
            .map(Zeroizing::new)
            .map_err(|_| "Invalid backup text.".into());
    }
    if bytes.len() < HEADER_LEN + aead::AES_256_GCM.tag_len() {
        return Err("The encrypted backup is incomplete or damaged.".into());
    }
    if bytes[8] != VERSION {
        return Err("This encrypted backup version is not supported. Update GateTree-Ops.".into());
    }
    if password.is_empty() {
        return Err("Enter the workspace password used when this backup was exported.".into());
    }
    let workspace_salt = &bytes[9..25];
    let backup_salt = &bytes[25..41];
    let mut workspace_key = Zeroizing::new([0_u8; 32]);
    pbkdf2_hmac::<Sha256>(
        password.as_bytes(),
        workspace_salt,
        WORKSPACE_ITERATIONS_V1,
        workspace_key.as_mut(),
    );
    let key = backup_key(workspace_key.as_ref(), backup_salt)?;
    let nonce = aead::Nonce::try_assume_unique_for_key(&bytes[41..HEADER_LEN])
        .map_err(|_| "Invalid backup nonce.")?;
    let mut decrypted = Zeroizing::new(bytes[HEADER_LEN..].to_vec());
    let plaintext = key
        .open_in_place(nonce, aead::Aad::from(&bytes[..HEADER_LEN]), &mut decrypted)
        .map_err(|_| "Incorrect backup workspace password, or the backup is damaged.")?;
    let text = std::str::from_utf8(plaintext).map_err(|_| "Invalid decrypted backup text.")?;
    Ok(Zeroizing::new(text.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_backups_are_randomized_authenticated_and_password_protected() {
        let password = "workspace-test-password";
        let salt = [42_u8; SALT_LEN];
        let key = crate::storage::derive_key(password, &salt);
        let json = "{\"account\":\"private-account\",\"note\":\"árvíztűrő\"}";
        let first = encrypt(json, &key, &salt).unwrap();
        let second = encrypt(json, &key, &salt).unwrap();
        assert_ne!(first, second);
        assert!(
            !first
                .windows(b"private-account".len())
                .any(|part| part == b"private-account")
        );
        assert_eq!(&*decode(&first, password).unwrap(), json);
        assert_eq!(&*decode(&second, password).unwrap(), json);
        assert!(
            decode(&first, "wrong-password")
                .unwrap_err()
                .contains("Incorrect")
        );
        assert!(decode(&first, "").is_err());
        // Workspace salt, HKDF salt, nonce, ciphertext and tag are authenticated.
        for index in [9, 25, 41, HEADER_LEN, first.len() - 1] {
            let mut damaged = first.clone();
            damaged[index] ^= 1;
            assert!(
                decode(&damaged, password).is_err(),
                "accepted damaged byte {index}"
            );
        }
        let mut future = first.clone();
        future[8] = VERSION + 1;
        assert!(decode(&future, password).unwrap_err().contains("version"));
        for length in [0, 7, 8, HEADER_LEN, first.len() - 1] {
            assert!(decode(&first[..length], password).is_err());
        }
        assert_eq!(&*decode(json.as_bytes(), "").unwrap(), json);
        assert!(decode(b"not a backup", password).is_err());
    }
}
