//! The signed description of a published UI bundle, and the rules for accepting one.

use base64::Engine;
use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A bundle larger than this is refused before it is downloaded.
pub const MAX_BUNDLE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub runtime_version: String,
    pub bundle_id: String,
    /// Unix seconds of the commit the bundle was built from.
    pub created_at: u64,
    pub sha256: String,
    pub size: u64,
    pub min_app_version: String,
}

pub fn manifest_name(runtime: &str) -> String {
    format!("manifest-{runtime}.json")
}

pub fn bundle_name(runtime: &str, bundle_id: &str) -> String {
    format!("bundle-{runtime}-{bundle_id}.zip")
}

pub fn signature_name(asset: &str) -> String {
    format!("{asset}.sig")
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Verifies `data` against a Tauri-format (base64 of a minisign file) key and signature.
pub fn verify_signature(pubkey_b64: &str, data: &[u8], signature_b64: &str) -> Result<(), String> {
    let decode = |label: &str, value: &str| -> Result<String, String> {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(value.trim())
            .map_err(|error| format!("{label} is not base64: {error}"))?;
        String::from_utf8(raw).map_err(|_| format!("{label} is not text"))
    };
    let key = PublicKey::decode(&decode("public key", pubkey_b64)?)
        .map_err(|error| format!("invalid public key: {error}"))?;
    let signature = Signature::decode(&decode("signature", signature_b64)?)
        .map_err(|error| format!("invalid signature: {error}"))?;
    key.verify(data, &signature, true)
        .map_err(|error| format!("signature does not verify: {error}"))
}

fn is_token(value: &str, min: usize, max: usize, allow: fn(char) -> bool) -> bool {
    (min..=max).contains(&value.len()) && value.chars().all(allow)
}

impl Manifest {
    /// Verifies the signature first, then parses; unsigned bytes are never interpreted.
    pub fn parse_signed(
        pubkey_b64: &str,
        raw: &[u8],
        signature_b64: &str,
    ) -> Result<Manifest, String> {
        if raw.len() > MAX_MANIFEST_BYTES {
            return Err("manifest is too large".to_string());
        }
        verify_signature(pubkey_b64, raw, signature_b64)?;
        let manifest: Manifest =
            serde_json::from_slice(raw).map_err(|error| format!("invalid manifest: {error}"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    // These strings become file names and directory names, so they are restricted to a plain
    // alphabet instead of being trusted because the manifest is signed.
    fn validate(&self) -> Result<(), String> {
        if !is_token(&self.runtime_version, 1, 64, |c| {
            c.is_ascii_alphanumeric() || c == '.' || c == '-'
        }) {
            return Err("manifest runtime_version is malformed".to_string());
        }
        if !is_token(&self.bundle_id, 7, 64, |c| c.is_ascii_hexdigit()) {
            return Err("manifest bundle_id is malformed".to_string());
        }
        if self.sha256.len() != 64 || !self.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("manifest sha256 is malformed".to_string());
        }
        if self.size == 0 || self.size > MAX_BUNDLE_BYTES {
            return Err("manifest size is out of range".to_string());
        }
        semver::Version::parse(&self.min_app_version)
            .map_err(|_| "manifest min_app_version is malformed".to_string())?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skip {
    RuntimeMismatch,
    AppTooOld,
    Blacklisted,
    NotNewer,
}

/// What this install already knows, for deciding whether a manifest is worth downloading.
pub struct Acceptance<'a> {
    pub runtime_version: &'a str,
    pub app_version: &'a str,
    /// The newest `created_at` among the embedded UI, the active bundle and the pending bundle.
    pub floor_created_at: u64,
    pub known_ids: &'a [&'a str],
    pub blacklist: &'a [String],
}

pub fn accept(manifest: &Manifest, known: &Acceptance<'_>) -> Result<(), Skip> {
    if manifest.runtime_version != known.runtime_version {
        return Err(Skip::RuntimeMismatch);
    }
    let required =
        semver::Version::parse(&manifest.min_app_version).map_err(|_| Skip::AppTooOld)?;
    let running = semver::Version::parse(known.app_version).map_err(|_| Skip::AppTooOld)?;
    if running < required {
        return Err(Skip::AppTooOld);
    }
    if known.blacklist.iter().any(|id| id == &manifest.bundle_id) {
        return Err(Skip::Blacklisted);
    }
    if known.known_ids.contains(&manifest.bundle_id.as_str())
        || manifest.created_at <= known.floor_created_at
    {
        return Err(Skip::NotNewer);
    }
    Ok(())
}

#[cfg(test)]
pub mod test_support {
    use super::*;

    pub struct TestKey {
        pub pubkey_b64: String,
        secret: minisign::SecretKey,
    }

    pub fn test_key() -> TestKey {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let pubkey_text = pair.pk.to_box().unwrap().to_string();
        TestKey {
            pubkey_b64: base64::engine::general_purpose::STANDARD.encode(pubkey_text),
            secret: pair.sk,
        }
    }

    impl TestKey {
        pub fn sign(&self, data: &[u8]) -> String {
            let signature = minisign::sign(
                None,
                &self.secret,
                std::io::Cursor::new(data),
                Some("test"),
                Some("test"),
            )
            .unwrap();
            base64::engine::general_purpose::STANDARD.encode(signature.to_string())
        }
    }

    pub fn manifest(runtime: &str, id: &str, created_at: u64, bundle: &[u8]) -> Manifest {
        Manifest {
            runtime_version: runtime.to_string(),
            bundle_id: id.to_string(),
            created_at,
            sha256: sha256_hex(bundle),
            size: bundle.len() as u64,
            min_app_version: "0.0.0".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    fn signed(key: &TestKey, manifest: &Manifest) -> (Vec<u8>, String) {
        let raw = serde_json::to_vec(manifest).unwrap();
        let signature = key.sign(&raw);
        (raw, signature)
    }

    fn acceptance<'a>(blacklist: &'a [String], known: &'a [&'a str]) -> Acceptance<'a> {
        Acceptance {
            runtime_version: "rt1",
            app_version: "2.23.6",
            floor_created_at: 100,
            known_ids: known,
            blacklist,
        }
    }

    #[test]
    fn parses_a_correctly_signed_manifest() {
        let key = test_key();
        let manifest = manifest("rt1", "abcdef0123", 200, b"zip");
        let (raw, signature) = signed(&key, &manifest);
        assert_eq!(
            Manifest::parse_signed(&key.pubkey_b64, &raw, &signature).unwrap(),
            manifest
        );
    }

    #[test]
    fn rejects_a_tampered_manifest() {
        let key = test_key();
        let manifest = manifest("rt1", "abcdef0123", 200, b"zip");
        let (raw, signature) = signed(&key, &manifest);
        let tampered = String::from_utf8(raw).unwrap().replace("200", "900");
        assert!(Manifest::parse_signed(&key.pubkey_b64, tampered.as_bytes(), &signature).is_err());
    }

    #[test]
    fn rejects_a_signature_from_another_key() {
        let key = test_key();
        let other = test_key();
        let manifest = manifest("rt1", "abcdef0123", 200, b"zip");
        let (raw, signature) = signed(&other, &manifest);
        assert!(Manifest::parse_signed(&key.pubkey_b64, &raw, &signature).is_err());
    }

    #[test]
    fn rejects_garbage_signatures_and_keys() {
        let key = test_key();
        let raw = b"{}";
        assert!(Manifest::parse_signed(&key.pubkey_b64, raw, "").is_err());
        assert!(Manifest::parse_signed(&key.pubkey_b64, raw, "bm90IGEgc2ln").is_err());
        assert!(Manifest::parse_signed("", raw, &key.sign(raw)).is_err());
    }

    #[test]
    fn rejects_path_like_identifiers_even_when_signed() {
        let key = test_key();
        for (runtime, id) in [
            ("../x", "abcdef0123"),
            ("rt1", "../../etc"),
            ("rt/1", "abcdef0123"),
        ] {
            let manifest = manifest(runtime, id, 200, b"zip");
            let (raw, signature) = signed(&key, &manifest);
            assert!(
                Manifest::parse_signed(&key.pubkey_b64, &raw, &signature).is_err(),
                "{runtime} {id}"
            );
        }
    }

    #[test]
    fn runtime_mismatch_is_ignored() {
        let manifest = manifest("other", "abcdef0123", 200, b"zip");
        assert_eq!(
            accept(&manifest, &acceptance(&[], &[])),
            Err(Skip::RuntimeMismatch)
        );
    }

    #[test]
    fn only_strictly_newer_bundles_are_accepted() {
        for created_at in [50, 100] {
            let manifest = manifest("rt1", "abcdef0123", created_at, b"zip");
            assert_eq!(
                accept(&manifest, &acceptance(&[], &[])),
                Err(Skip::NotNewer)
            );
        }
        let newer = manifest("rt1", "abcdef0123", 101, b"zip");
        assert_eq!(accept(&newer, &acceptance(&[], &[])), Ok(()));
        assert_eq!(
            accept(&newer, &acceptance(&[], &["abcdef0123"])),
            Err(Skip::NotNewer)
        );
    }

    #[test]
    fn blacklisted_bundles_are_never_retried() {
        let manifest = manifest("rt1", "abcdef0123", 500, b"zip");
        let blacklist = vec!["abcdef0123".to_string()];
        assert_eq!(
            accept(&manifest, &acceptance(&blacklist, &[])),
            Err(Skip::Blacklisted)
        );
    }

    #[test]
    fn min_app_version_gates_old_apps() {
        let mut manifest = manifest("rt1", "abcdef0123", 500, b"zip");
        manifest.min_app_version = "2.24.0".to_string();
        assert_eq!(
            accept(&manifest, &acceptance(&[], &[])),
            Err(Skip::AppTooOld)
        );
        manifest.min_app_version = "2.23.6".to_string();
        assert_eq!(accept(&manifest, &acceptance(&[], &[])), Ok(()));
    }
}
