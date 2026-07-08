//! Ported from `crates/forge-cli/src/apns.rs`'s `ApnsAuth`/`b64url` — see ADR-0012 for why this
//! is a deliberate COPY, not a shared crate, at least for now: this is ~60 lines, already
//! unit-tested in place (the test below is the same test, unchanged), and extracting a shared
//! `forge-apns-core` crate before either call site has actually needed to change in lockstep
//! would be premature plumbing. If you touch JWT construction here, touch it in
//! `crates/forge-cli/src/apns.rs` too (and vice versa) until that extraction happens.

use base64::Engine;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::pkcs8::DecodePrivateKey;

/// How long a signed auth JWT stays valid before re-minting — see the identical constant and
/// rationale in `crates/forge-cli/src/apns.rs`.
const AUTH_TOKEN_TTL_SECS: u64 = 50 * 60;

pub(crate) fn b64url(data: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

#[cfg(test)]
pub(crate) fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s.trim())
        .ok()
}

/// Signs and caches the `authorization: bearer <jwt>` token every APNs request carries. Reuses
/// one JWT for up to [`AUTH_TOKEN_TTL_SECS`] rather than minting one per push, per Apple's own
/// guidance and rate limit on token requests.
pub(crate) struct ApnsAuth {
    signing_key: SigningKey,
    team_id: String,
    key_id: String,
    cached: std::sync::Mutex<Option<(String, u64)>>,
}

impl ApnsAuth {
    pub(crate) fn new(key_pem: &str, team_id: &str, key_id: &str) -> anyhow::Result<Self> {
        let secret = p256::SecretKey::from_pkcs8_pem(key_pem)
            .map_err(|e| anyhow::anyhow!("invalid APNs .p8 key: {e}"))?;
        Ok(Self {
            signing_key: SigningKey::from(secret),
            team_id: team_id.to_string(),
            key_id: key_id.to_string(),
            cached: std::sync::Mutex::new(None),
        })
    }

    #[cfg(test)]
    pub(crate) fn from_scalar(bytes: &[u8], team_id: &str, key_id: &str) -> Self {
        Self {
            signing_key: SigningKey::from(
                p256::SecretKey::from_slice(bytes).expect("valid P-256 scalar"),
            ),
            team_id: team_id.to_string(),
            key_id: key_id.to_string(),
            cached: std::sync::Mutex::new(None),
        }
    }

    pub(crate) fn bearer_token(&self, now_unix: u64) -> String {
        let mut cached = self.cached.lock().unwrap();
        if let Some((tok, minted_at)) = cached.as_ref() {
            if now_unix.saturating_sub(*minted_at) < AUTH_TOKEN_TTL_SECS {
                return tok.clone();
            }
        }
        let header = b64url(format!(r#"{{"alg":"ES256","kid":"{}"}}"#, self.key_id).as_bytes());
        let claims = b64url(
            serde_json::json!({ "iss": self.team_id, "iat": now_unix })
                .to_string()
                .as_bytes(),
        );
        let signing_input = format!("{header}.{claims}");
        let sig: Signature = self.signing_key.sign(signing_input.as_bytes());
        let token = format!("{signing_input}.{}", b64url(&sig.to_bytes()));
        *cached = Some((token.clone(), now_unix));
        token
    }
}

/// The host for a given environment string — identical routing rule to
/// `crates/forge-cli/src/apns.rs`'s `ApnsNotifier::host`.
pub(crate) fn host(environment: &str) -> &'static str {
    if environment == "production" {
        "https://api.push.apple.com"
    } else {
        "https://api.sandbox.push.apple.com"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::Verifier;
    use p256::ecdsa::VerifyingKey;

    /// Identical to `crates/forge-cli/src/apns.rs`'s `bearer_token_is_verifiable_with_the_public_key`
    /// — proves the ported JWT construction is byte-for-byte the same, since a subtly different
    /// signature here is exactly the kind of bug Apple would silently reject in production.
    #[test]
    fn bearer_token_is_verifiable_with_the_public_key() {
        let scalar = [9u8; 32];
        let auth = ApnsAuth::from_scalar(&scalar, "TEAM123456", "KEY7890AB");
        let token = auth.bearer_token(1_700_000_000);

        let mut parts = token.split('.');
        let (h, c, s) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        assert!(parts.next().is_none(), "exactly three JWT segments");

        let header: serde_json::Value = serde_json::from_slice(&b64url_decode(h).unwrap()).unwrap();
        assert_eq!(header["alg"], "ES256");
        assert_eq!(header["kid"], "KEY7890AB");

        let claims: serde_json::Value = serde_json::from_slice(&b64url_decode(c).unwrap()).unwrap();
        assert_eq!(claims["iss"], "TEAM123456");
        assert_eq!(claims["iat"], 1_700_000_000u64);

        let sig_bytes = b64url_decode(s).unwrap();
        let sig = Signature::from_slice(&sig_bytes).expect("64-byte r||s signature");
        let secret = p256::SecretKey::from_slice(&scalar).unwrap();
        let vk = VerifyingKey::from(secret.public_key());
        vk.verify(format!("{h}.{c}").as_bytes(), &sig)
            .expect("signature verifies with the advertised key");
    }

    #[test]
    fn bearer_token_is_cached_and_reminted_after_ttl() {
        let auth = ApnsAuth::from_scalar(&[3u8; 32], "TEAM", "KEY");
        let first = auth.bearer_token(1_000_000);
        let same = auth.bearer_token(1_000_000 + AUTH_TOKEN_TTL_SECS - 1);
        assert_eq!(first, same, "reused while within the TTL");
        let fresh = auth.bearer_token(1_000_000 + AUTH_TOKEN_TTL_SECS);
        assert_ne!(first, fresh, "re-minted once the cached token goes stale");
    }

    #[test]
    fn host_routes_production_and_everything_else_to_sandbox() {
        assert_eq!(host("production"), "https://api.push.apple.com");
        assert_eq!(host("sandbox"), "https://api.sandbox.push.apple.com");
        assert_eq!(host("garbage"), "https://api.sandbox.push.apple.com");
    }
}
