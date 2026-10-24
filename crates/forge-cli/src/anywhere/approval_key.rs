use anyhow::{bail, Result};

use super::decode_base64_array;

/// The relay hands back the key the new device registered. A mismatch means the relay swapped
/// it, so the approval would wrap the Account Data Key for someone else.
pub(super) fn ensure_expected_signing_key(relayed: &str, expected: &str) -> Result<()> {
    let relayed = decode_base64_array::<32>(relayed, "device signing key")?;
    let expected = decode_base64_array::<32>(expected, "expected signing key")?;
    if relayed != expected {
        bail!("the device approval request does not carry the expected signing key; nothing was approved");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    use super::ensure_expected_signing_key;

    #[test]
    fn unattended_approval_requires_the_exact_signing_key_the_local_app_generated() {
        let key = URL_SAFE_NO_PAD.encode([7u8; 32]);
        let other = URL_SAFE_NO_PAD.encode([8u8; 32]);
        assert!(ensure_expected_signing_key(&key, &key).is_ok());
        assert!(ensure_expected_signing_key(&other, &key).is_err());
        assert!(ensure_expected_signing_key(&key, "not-a-key").is_err());
    }
}
