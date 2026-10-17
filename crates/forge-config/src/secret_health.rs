//! Read-only validation of the encrypted-file secret store (`secrets.enc` + `secret.key`).
//!
//! `forge doctor` used to report the store healthy whenever a backend was selected. This really
//! decrypts every entry, and never creates, repairs or rewrites either file.

use std::path::Path;

use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStoreHealth {
    /// No `secrets.enc`: nothing stored in the file fallback.
    Absent,
    /// Every entry decrypted.
    Healthy { entries: usize },
    /// The file, its key, or at least one entry cannot be read; the text says what and how to fix.
    Corrupt(String),
}

/// Validate the store at `secrets` using the key file at `keyfile`.
pub fn check_file_store(secrets: &Path, keyfile: &Path) -> FileStoreHealth {
    let bytes = match std::fs::read(secrets) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return FileStoreHealth::Absent,
        Err(e) => {
            return FileStoreHealth::Corrupt(format!("cannot read {}: {e}", secrets.display()))
        }
    };
    let map: std::collections::BTreeMap<String, String> = match serde_json::from_slice(&bytes) {
        Ok(map) => map,
        Err(e) => {
            return FileStoreHealth::Corrupt(format!(
                "{} is corrupt ({e}); move it aside and re-add keys with `forge auth`",
                secrets.display()
            ))
        }
    };
    if map.is_empty() {
        return FileStoreHealth::Healthy { entries: 0 };
    }
    let key = match std::fs::read(keyfile) {
        Ok(raw) if raw.len() == 32 => Key::try_from(raw.as_slice()).ok(),
        Ok(raw) => {
            return FileStoreHealth::Corrupt(format!(
                "{} is {} bytes, expected 32; {} stored secret(s) cannot be decrypted. Restore it \
                 from a backup, or delete it and secrets.enc and re-add keys with `forge auth`",
                keyfile.display(),
                raw.len(),
                map.len()
            ))
        }
        Err(e) => {
            return FileStoreHealth::Corrupt(format!(
                "cannot read {} ({e}); {} stored secret(s) cannot be decrypted. Restore it from a \
                 backup, or delete secrets.enc and re-add keys with `forge auth`",
                keyfile.display(),
                map.len()
            ))
        }
    };
    let Some(key) = key else {
        return FileStoreHealth::Corrupt(format!("{} is not a valid key", keyfile.display()));
    };
    let cipher = ChaCha20Poly1305::new(&key);
    let broken: Vec<&str> = map
        .iter()
        .filter(|(_, encoded)| !decrypts(&cipher, encoded))
        .map(|(name, _)| name.as_str())
        .collect();
    if broken.is_empty() {
        FileStoreHealth::Healthy { entries: map.len() }
    } else {
        FileStoreHealth::Corrupt(format!(
            "{} of {} entries in {} do not decrypt ({}); the key file does not match or the entries \
             are damaged. Re-add them with `forge auth <provider>`",
            broken.len(),
            map.len(),
            secrets.display(),
            broken.join(", ")
        ))
    }
}

fn decrypts(cipher: &ChaCha20Poly1305, encoded: &str) -> bool {
    let Ok(blob) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
        return false;
    };
    if blob.len() < 12 {
        return false;
    }
    let (nonce, ciphertext) = blob.split_at(12);
    let Ok(nonce) = Nonce::try_from(nonce) else {
        return false;
    };
    cipher.decrypt(&nonce, ciphertext).is_ok()
}

/// [`check_file_store`] against the per-user config directory.
pub fn file_store_health() -> FileStoreHealth {
    match crate::config_dir() {
        Some(dir) => check_file_store(&dir.join("secrets.enc"), &dir.join("secret.key")),
        None => FileStoreHealth::Absent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seal(key: &[u8; 32], value: &str) -> String {
        let cipher = ChaCha20Poly1305::new(&Key::try_from(key.as_slice()).unwrap());
        let nonce_bytes = [7u8; 12];
        let nonce = Nonce::try_from(nonce_bytes.as_slice()).unwrap();
        let mut blob = nonce_bytes.to_vec();
        blob.extend(cipher.encrypt(&nonce, value.as_bytes()).unwrap());
        base64::engine::general_purpose::STANDARD.encode(blob)
    }

    fn write(dir: &Path, key: &[u8; 32], entries: &[(&str, String)]) {
        std::fs::write(dir.join("secret.key"), key).unwrap();
        let map: std::collections::BTreeMap<_, _> = entries.iter().cloned().collect();
        std::fs::write(dir.join("secrets.enc"), serde_json::to_vec(&map).unwrap()).unwrap();
    }

    fn check(dir: &Path) -> FileStoreHealth {
        check_file_store(&dir.join("secrets.enc"), &dir.join("secret.key"))
    }

    #[test]
    fn absent_healthy_and_corrupt_are_told_apart() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(check(dir.path()), FileStoreHealth::Absent);

        let key = [3u8; 32];
        write(
            dir.path(),
            &key,
            &[("groq", seal(&key, "sk-1")), ("x", seal(&key, "y"))],
        );
        assert_eq!(check(dir.path()), FileStoreHealth::Healthy { entries: 2 });

        std::fs::write(dir.path().join("secrets.enc"), b"{ not json").unwrap();
        let FileStoreHealth::Corrupt(text) = check(dir.path()) else {
            panic!("garbage must be corrupt");
        };
        assert!(text.contains("is corrupt"), "{text}");
    }

    #[test]
    fn a_wrong_or_missing_key_and_a_damaged_entry_are_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let key = [3u8; 32];
        write(
            dir.path(),
            &key,
            &[("groq", seal(&key, "sk-1")), ("bad", "AAAA".into())],
        );
        let FileStoreHealth::Corrupt(text) = check(dir.path()) else {
            panic!("a damaged entry must be corrupt");
        };
        assert!(text.contains("1 of 2") && text.contains("bad"), "{text}");

        write(dir.path(), &[9u8; 32], &[("groq", seal(&key, "sk-1"))]);
        assert!(matches!(check(dir.path()), FileStoreHealth::Corrupt(_)));

        std::fs::remove_file(dir.path().join("secret.key")).unwrap();
        let FileStoreHealth::Corrupt(text) = check(dir.path()) else {
            panic!("missing key must be corrupt");
        };
        assert!(text.contains("secret.key"), "{text}");
        assert!(
            !dir.path().join("secret.key").exists(),
            "must not create a key"
        );
    }
}
