//! OAuth 2.0 foundation for OAuth-protected MCP servers (RFC-mcp-oauth, PR1). This module owns the
//! **pure, offline-testable** half: config + token types, PKCE (RFC 7636), the authorize-URL
//! builder (RFC 6749 + 8252 loopback), discovery-metadata parsing (RFC 9728 + 8414), and keyring
//! token storage (ADR-0007 — tokens live in the keyring, never in config/logs).
//!
//! The networked half (metadata fetch, token exchange/refresh, the loopback listener + browser
//! open, connect-time integration) lands in forge-mcp + forge-cli (PR2); it builds on these types.

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ConfigError;

/// Per-server OAuth config (the `[servers.auth.oauth]` table). All optional — discovered at login
/// and persisted back. Presence of this (vs a static token) marks a server as OAuth.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthConfig {
    /// Authorization-server issuer. Discovered from the 401's resource-metadata when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    /// Scopes to request (the server may narrow them).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    /// Client id — set after dynamic registration (RFC 7591), or pinned manually.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Pin the loopback redirect port (firewalled hosts); ephemeral (`:0`) when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_port: Option<u16>,
}

/// Tokens persisted (keyring only) per server under `mcp-oauth:<server>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Unix seconds when the access token expires (0 = unknown / no expiry).
    #[serde(default)]
    pub expires_at: i64,
    pub token_endpoint: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

impl OAuthTokens {
    /// Whether the access token is expired (or within `skew` seconds of it) as of `now`. An
    /// `expires_at` of 0 means "unknown" → treated as not-expired (let the server 401 if stale).
    pub fn is_expired(&self, now: i64, skew: i64) -> bool {
        self.expires_at != 0 && now + skew >= self.expires_at
    }
}

/// RFC 9728 Protected Resource Metadata (the 401's `resource_metadata` doc).
#[derive(Debug, Clone, Deserialize)]
pub struct ProtectedResourceMetadata {
    #[serde(default)]
    pub authorization_servers: Vec<String>,
}

/// RFC 8414 Authorization Server Metadata (`/.well-known/oauth-authorization-server`).
#[derive(Debug, Clone, Deserialize)]
pub struct AuthServerMetadata {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
}

/// A PKCE pair (RFC 7636): the `verifier` is the secret kept locally; the `challenge` is the
/// S256 hash sent in the authorize request and proven later by presenting the verifier.
#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    /// Generate a fresh PKCE pair: a 32-byte CSPRNG verifier (base64url, ~43 chars) and its
    /// S256 challenge `base64url(sha256(verifier))`.
    pub fn generate() -> Pkce {
        let bytes: [u8; 32] = rand::random();
        let verifier = b64url(&bytes);
        Pkce::from_verifier(verifier)
    }

    /// Build the pair from a given verifier (used by tests against the RFC 7636 vector).
    pub fn from_verifier(verifier: String) -> Pkce {
        let digest = Sha256::digest(verifier.as_bytes());
        Pkce {
            challenge: b64url(&digest),
            verifier,
        }
    }
}

/// A random URL-safe `state` (CSRF guard for the authorize round-trip).
pub fn random_state() -> String {
    let bytes: [u8; 16] = rand::random();
    b64url(&bytes)
}

/// base64url **without padding** (RFC 7636 §A / RFC 4648 §5).
fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Percent-encode a query-parameter value (RFC 3986 unreserved stays literal).
fn pct(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Build the authorization-request URL (RFC 6749 §4.1.1 + PKCE S256). `redirect_uri` is the
/// loopback callback (RFC 8252). Scopes are space-joined; `state` + `challenge` bind the request.
pub fn authorize_url(
    authorization_endpoint: &str,
    client_id: &str,
    redirect_uri: &str,
    scopes: &[String],
    state: &str,
    code_challenge: &str,
) -> String {
    let sep = if authorization_endpoint.contains('?') {
        '&'
    } else {
        '?'
    };
    let scope = scopes.join(" ");
    format!(
        "{authorization_endpoint}{sep}response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        pct(client_id),
        pct(redirect_uri),
        pct(&scope),
        pct(state),
        pct(code_challenge),
    )
}

/// Keyring key for a server's OAuth tokens — distinct from the static `mcp:<server>` bearer key.
pub fn oauth_keyring_key(server: &str) -> String {
    format!("mcp-oauth:{server}")
}

/// Persist a server's OAuth tokens (keyring, encrypted-file fallback; ADR-0007: never in
/// config/logs). Updates the *active* account only — see [`OAuthAccountStore`].
pub fn store_oauth_tokens(server: &str, tokens: &OAuthTokens) -> Result<(), ConfigError> {
    store_active_tokens(&oauth_keyring_key(server), tokens)
}

/// Load a server's *active* OAuth tokens, or `None` if none stored / unreadable.
pub fn load_oauth_tokens(server: &str) -> Option<OAuthTokens> {
    load_active_tokens(&oauth_keyring_key(server))
}

/// Delete a server's stored OAuth tokens — every account (`forge mcp logout`). Idempotent:
/// `Ok(false)` if none.
pub fn clear_oauth_tokens(server: &str) -> Result<bool, ConfigError> {
    clear_account_store(&oauth_keyring_key(server))
}

/// Add (or overwrite) an OAuth account for `server` and make it active.
pub fn add_oauth_account(server: &str, id: &str, tokens: &OAuthTokens) -> Result<(), ConfigError> {
    add_account(&oauth_keyring_key(server), id, tokens)
}

/// `(id, tokens, is_active)` for every OAuth account stored for `server`.
pub fn list_oauth_accounts(server: &str) -> Vec<(String, OAuthTokens, bool)> {
    list_accounts(&oauth_keyring_key(server))
}

/// Switch `server`'s active OAuth account. Errors if `id` isn't stored.
pub fn switch_oauth_account(server: &str, id: &str) -> Result<(), ConfigError> {
    switch_account(&oauth_keyring_key(server), id)
}

/// Remove one OAuth account for `server`. Promotes a remaining account to active if the removed
/// one was active; deletes the whole entry if none remain. `Ok(false)` if `id` wasn't stored.
pub fn remove_oauth_account(server: &str, id: &str) -> Result<bool, ConfigError> {
    remove_account(&oauth_keyring_key(server), id)
}

/// First free `account-N` id for a fresh `server` login with no better label available.
pub fn next_oauth_account_id(server: &str) -> String {
    next_default_account_id(&oauth_keyring_key(server))
}

// ---------------------------------------------------------------------------------------------
// Multi-account storage (RFC multi-account-oauth). One keyring entry (`mcp-oauth:<server>` or
// `provider-oauth:<provider>`, see [`crate::provider_oauth`]) holds N accounts with one "active".
// Kept generic here — keyed by the FULL keyring key string — so both OAuth kinds share one
// implementation. Split into a pure half (parsing/mutation on [`OAuthAccountStore`], fully
// offline-testable) and thin I/O wrappers over [`crate::secret_store`].
// ---------------------------------------------------------------------------------------------

/// Current on-disk shape. v1 (pre-multi-account) was a bare [`OAuthTokens`] blob; migrated
/// transparently on read by [`parse_account_store`], never requiring a manual step.
const ACCOUNT_STORE_VERSION: u32 = 2;

/// The account id a migrated v1 store (or a store created before any account has a real label)
/// uses.
pub const DEFAULT_ACCOUNT_ID: &str = "default";

/// N accounts under one keyring entry, with one marked active. `load_oauth_tokens` /
/// `store_oauth_tokens` / `clear_oauth_tokens` (and their `provider_oauth` counterparts) are
/// "active-account" views over this that preserve the pre-multi-account single-account behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthAccountStore {
    pub version: u32,
    pub active: String,
    pub accounts: std::collections::BTreeMap<String, OAuthTokens>,
}

impl OAuthAccountStore {
    /// Build a single-account store (also used by tests injecting an in-memory account source).
    pub fn new_single(id: impl Into<String>, tokens: OAuthTokens) -> Self {
        let id = id.into();
        let mut accounts = std::collections::BTreeMap::new();
        accounts.insert(id.clone(), tokens);
        Self {
            version: ACCOUNT_STORE_VERSION,
            active: id,
            accounts,
        }
    }

    /// The active account's tokens.
    pub fn active_tokens(&self) -> Option<&OAuthTokens> {
        self.accounts.get(&self.active)
    }

    /// Add (or overwrite) an account and make it active.
    pub fn add(&mut self, id: &str, tokens: OAuthTokens) {
        self.accounts.insert(id.to_string(), tokens);
        self.active = id.to_string();
    }

    /// Overwrite the active account's tokens in place (the refresh path) — every other account
    /// is left untouched.
    pub fn set_active_tokens(&mut self, tokens: OAuthTokens) {
        self.accounts.insert(self.active.clone(), tokens);
    }

    /// Overwrite one account's tokens by id (rotation refresh path). Errors if `id` isn't stored.
    /// Unlike [`set_active_tokens`], this does **not** require `id` to be the active account — so a
    /// rotated non-active account can refresh without clobbering the wrong slot.
    pub fn set_tokens(&mut self, id: &str, tokens: OAuthTokens) -> Result<(), ConfigError> {
        if !self.accounts.contains_key(id) {
            return Err(ConfigError::Keyring(format!("no account '{id}' stored")));
        }
        self.accounts.insert(id.to_string(), tokens);
        Ok(())
    }

    /// Tokens for a specific account id, if stored.
    pub fn tokens_for(&self, id: &str) -> Option<&OAuthTokens> {
        self.accounts.get(id)
    }

    /// Account ids in stable (`BTreeMap`) order — the rotation pool's round-robin sequence.
    pub fn account_ids(&self) -> Vec<String> {
        self.accounts.keys().cloned().collect()
    }

    /// `(id, tokens, is_active)` for every stored account, in id order.
    pub fn list(&self) -> Vec<(String, OAuthTokens, bool)> {
        self.accounts
            .iter()
            .map(|(id, tokens)| (id.clone(), tokens.clone(), *id == self.active))
            .collect()
    }

    /// Switch the active account. Errors if `id` isn't stored.
    pub fn switch(&mut self, id: &str) -> Result<(), ConfigError> {
        if !self.accounts.contains_key(id) {
            return Err(ConfigError::Keyring(format!("no account '{id}' stored")));
        }
        self.active = id.to_string();
        Ok(())
    }

    /// Remove one account. If it was active, promote the first remaining account (id order) to
    /// active. Returns whether `id` was actually stored.
    pub fn remove(&mut self, id: &str) -> bool {
        if self.accounts.remove(id).is_none() {
            return false;
        }
        if self.active == id {
            if let Some(next) = self.accounts.keys().next() {
                self.active = next.clone();
            }
        }
        true
    }
}

/// Parse a stored blob into an [`OAuthAccountStore`], transparently migrating a v1 bare
/// [`OAuthTokens`] JSON into a single-account v2 store under [`DEFAULT_ACCOUNT_ID`]. `None` if
/// `json` matches neither shape.
pub fn parse_account_store(json: &str) -> Option<OAuthAccountStore> {
    if let Ok(store) = serde_json::from_str::<OAuthAccountStore>(json) {
        return Some(store);
    }
    let tokens: OAuthTokens = serde_json::from_str(json).ok()?;
    Some(OAuthAccountStore::new_single(DEFAULT_ACCOUNT_ID, tokens))
}

/// First `account-N` (1-based) id not already present in `existing`.
pub fn next_default_id(existing: &std::collections::BTreeMap<String, OAuthTokens>) -> String {
    let mut n = 1u32;
    loop {
        let candidate = format!("account-{n}");
        if !existing.contains_key(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Load the account store at `key` (transparent v1→v2 migration on read). `None` if nothing is
/// stored / unreadable.
pub fn load_account_store(key: &str) -> Option<OAuthAccountStore> {
    crate::secret_store::get(key).and_then(|json| parse_account_store(&json))
}

/// Persist an account store at `key`.
pub fn save_account_store(key: &str, store: &OAuthAccountStore) -> Result<(), ConfigError> {
    let json = serde_json::to_string(store).map_err(|e| ConfigError::Keyring(e.to_string()))?;
    crate::secret_store::set(key, &json)
}

/// The active account's tokens at `key`.
pub fn load_active_tokens(key: &str) -> Option<OAuthTokens> {
    load_account_store(key).and_then(|s| s.active_tokens().cloned())
}

/// Update the active account's tokens, creating a single-account store (id
/// [`DEFAULT_ACCOUNT_ID`]) if none exists yet.
pub fn store_active_tokens(key: &str, tokens: &OAuthTokens) -> Result<(), ConfigError> {
    let mut store = load_account_store(key)
        .unwrap_or_else(|| OAuthAccountStore::new_single(DEFAULT_ACCOUNT_ID, tokens.clone()));
    store.set_active_tokens(tokens.clone());
    save_account_store(key, &store)
}

/// Load one account's tokens by id at `key`. `None` if nothing is stored or `id` is missing.
pub fn load_account_tokens(key: &str, id: &str) -> Option<OAuthTokens> {
    load_account_store(key).and_then(|s| s.tokens_for(id).cloned())
}

/// Overwrite one account's tokens by id at `key`. Errors if nothing is stored or `id` isn't one
/// of the stored accounts. Used by the multi-account rotation refresh path so a non-active
/// account can refresh without touching the active slot.
pub fn store_account_tokens(key: &str, id: &str, tokens: &OAuthTokens) -> Result<(), ConfigError> {
    let mut store = load_account_store(key)
        .ok_or_else(|| ConfigError::Keyring("no accounts stored".to_string()))?;
    store.set_tokens(id, tokens.clone())?;
    save_account_store(key, &store)
}

/// Delete the whole entry (every account) at `key`. Idempotent: `Ok(false)` if none.
pub fn clear_account_store(key: &str) -> Result<bool, ConfigError> {
    crate::secret_store::delete(key)
}

/// Add (or overwrite) an account at `key` and make it active.
pub fn add_account(key: &str, id: &str, tokens: &OAuthTokens) -> Result<(), ConfigError> {
    let mut store = load_account_store(key).unwrap_or_else(|| OAuthAccountStore {
        version: ACCOUNT_STORE_VERSION,
        active: id.to_string(),
        accounts: Default::default(),
    });
    store.add(id, tokens.clone());
    save_account_store(key, &store)
}

/// `(id, tokens, is_active)` for every account stored at `key`. Empty if nothing is stored.
pub fn list_accounts(key: &str) -> Vec<(String, OAuthTokens, bool)> {
    load_account_store(key)
        .map(|s| s.list())
        .unwrap_or_default()
}

/// Switch the active account at `key`. Errors if nothing is stored, or `id` isn't one of the
/// stored accounts.
pub fn switch_account(key: &str, id: &str) -> Result<(), ConfigError> {
    let mut store = load_account_store(key)
        .ok_or_else(|| ConfigError::Keyring("no accounts stored".to_string()))?;
    store.switch(id)?;
    save_account_store(key, &store)
}

/// Remove one account at `key`. If it was active, promote a remaining account to active; if none
/// remain, delete the whole entry. `Ok(false)` if `id` wasn't stored (or nothing was stored).
pub fn remove_account(key: &str, id: &str) -> Result<bool, ConfigError> {
    let Some(mut store) = load_account_store(key) else {
        return Ok(false);
    };
    if !store.remove(id) {
        return Ok(false);
    }
    if store.accounts.is_empty() {
        crate::secret_store::delete(key)?;
    } else {
        save_account_store(key, &store)?;
    }
    Ok(true)
}

/// First free `account-N` id for a fresh login at `key` (no email/JWT label available).
pub fn next_default_account_id(key: &str) -> String {
    let existing = load_account_store(key)
        .map(|s| s.accounts)
        .unwrap_or_default();
    next_default_id(&existing)
}

// ---------------------------------------------------------------------------------------------
// Multi-account OAuth rotation pool (docs/design/oauth-account-rotation.md)
// ---------------------------------------------------------------------------------------------

/// Round-robin pool of OAuth account ids for one keyring entry. Mirrors the API-key [`KeyPool`]
/// pattern: engages only when ≥2 accounts are stored; `next` advances an atomic cursor. The pool
/// holds **ids** (not live tokens) so a refresh of one account never races the pool snapshot —
/// callers load/refresh tokens for the returned id on each pick.
#[derive(Debug, Default)]
pub struct OAuthAccountPool {
    /// Stable-ordered account ids (BTreeMap key order at snapshot time).
    ids: Vec<String>,
    cursor: std::sync::atomic::AtomicUsize,
    /// Manual-active account id at snapshot time (rotation seed / list UX); unused by `next`.
    active: Option<String>,
}

impl OAuthAccountPool {
    /// Snapshot the account store at `key`. With <2 accounts the pool is empty (`has_rotation`
    /// is false) and callers fall through to the single-active path.
    pub fn from_keyring(key: &str) -> Self {
        let Some(store) = load_account_store(key) else {
            return Self::default();
        };
        Self::from_store(&store)
    }

    /// Build a pool from an in-memory store (tests + callers that already loaded the store).
    pub fn from_store(store: &OAuthAccountStore) -> Self {
        let ids = store.account_ids();
        if ids.len() < 2 {
            return Self::default();
        }
        // Seed the cursor at the manual-active account so the first `next` after process start
        // prefers the user's `--switch` choice, then round-robins from there.
        let start = ids.iter().position(|id| id == &store.active).unwrap_or(0);
        Self {
            ids,
            cursor: std::sync::atomic::AtomicUsize::new(start),
            active: Some(store.active.clone()),
        }
    }

    /// Construct a pool from an explicit id list (offline unit tests). ≥2 ids required for
    /// rotation; fewer yields an empty pool.
    pub fn from_ids(ids: Vec<String>) -> Self {
        if ids.len() < 2 {
            return Self::default();
        }
        Self {
            ids,
            cursor: std::sync::atomic::AtomicUsize::new(0),
            active: None,
        }
    }

    /// Whether this pool has ≥2 accounts and therefore supports intra-provider account rotation.
    pub fn has_rotation(&self) -> bool {
        self.ids.len() >= 2
    }

    /// Number of accounts in the pool (0 when rotation is off).
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The next account id (round-robin), or `None` when the pool has <2 accounts.
    pub fn next(&self) -> Option<String> {
        if self.ids.len() < 2 {
            return None;
        }
        let i = self
            .cursor
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            % self.ids.len();
        Some(self.ids[i].clone())
    }

    /// Manual-active id at snapshot time, if any.
    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// Stable-ordered account ids in this pool.
    pub fn ids(&self) -> &[String] {
        &self.ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc7636_vector() {
        // RFC 7636 Appendix B known-answer test.
        let p = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".to_string());
        assert_eq!(p.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn generated_pkce_is_url_safe_and_verifiable() {
        let p = Pkce::generate();
        assert!(p.verifier.len() >= 43, "verifier ≥43 chars (RFC 7636)");
        assert!(
            !p.verifier.contains(['+', '/', '=']),
            "base64url, no padding"
        );
        assert!(!p.challenge.contains(['+', '/', '=']));
        // The challenge is reproducible from the verifier.
        assert_eq!(
            Pkce::from_verifier(p.verifier.clone()).challenge,
            p.challenge
        );
    }

    #[test]
    fn authorize_url_has_required_params_and_encodes() {
        let url = authorize_url(
            "https://auth.example/authorize",
            "client 1",
            "http://127.0.0.1:8080/callback",
            &["mcp".into(), "offline".into()],
            "xyz",
            "CHAL",
        );
        assert!(url.starts_with("https://auth.example/authorize?response_type=code"));
        assert!(url.contains("client_id=client%201"), "space encoded: {url}");
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A8080%2Fcallback"));
        assert!(url.contains("scope=mcp%20offline"));
        assert!(url.contains("code_challenge=CHAL&code_challenge_method=S256"));
        assert!(url.contains("state=xyz"));
    }

    #[test]
    fn authorize_url_appends_with_amp_when_endpoint_has_query() {
        let url = authorize_url(
            "https://a/x?foo=1",
            "c",
            "http://127.0.0.1/cb",
            &[],
            "s",
            "ch",
        );
        assert!(
            url.contains("x?foo=1&response_type=code"),
            "uses & not ?: {url}"
        );
    }

    #[test]
    fn protected_resource_metadata_parses() {
        let m: ProtectedResourceMetadata = serde_json::from_str(
            r#"{"resource":"https://helm/mcp","authorization_servers":["https://helm"]}"#,
        )
        .unwrap();
        assert_eq!(m.authorization_servers, vec!["https://helm".to_string()]);
    }

    #[test]
    fn auth_server_metadata_parses_with_optional_registration() {
        let m: AuthServerMetadata = serde_json::from_str(
            r#"{"issuer":"https://helm","authorization_endpoint":"https://helm/authorize",
                "token_endpoint":"https://helm/token"}"#,
        )
        .unwrap();
        assert_eq!(m.authorization_endpoint, "https://helm/authorize");
        assert_eq!(m.token_endpoint, "https://helm/token");
        assert!(m.registration_endpoint.is_none());
    }

    #[test]
    fn tokens_round_trip_json_and_expiry_logic() {
        let t = OAuthTokens {
            access_token: "at".into(),
            refresh_token: Some("rt".into()),
            expires_at: 1000,
            token_endpoint: "https://helm/token".into(),
            client_id: "cid".into(),
            scopes: vec!["mcp".into()],
        };
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<OAuthTokens>(&json).unwrap(), t);
        // Expired within the skew window, fresh well before it; 0 = unknown = never expired.
        assert!(t.is_expired(950, 60), "950+60 >= 1000");
        assert!(!t.is_expired(800, 60));
        let unknown = OAuthTokens { expires_at: 0, ..t };
        assert!(!unknown.is_expired(i64::MAX - 1, 60));
    }

    #[test]
    fn keyring_key_is_namespaced() {
        assert_eq!(oauth_keyring_key("helm"), "mcp-oauth:helm");
    }

    fn sample_tokens(label: &str) -> OAuthTokens {
        OAuthTokens {
            access_token: format!("at-{label}"),
            refresh_token: Some(format!("rt-{label}")),
            expires_at: 1000,
            token_endpoint: "https://helm/token".into(),
            client_id: "cid".into(),
            scopes: vec!["mcp".into()],
        }
    }

    #[test]
    fn v1_bare_tokens_blob_migrates_to_a_single_default_account() {
        let t = sample_tokens("a");
        let v1_json = serde_json::to_string(&t).unwrap();
        let store = parse_account_store(&v1_json).expect("v1 blob should parse");
        assert_eq!(store.version, ACCOUNT_STORE_VERSION);
        assert_eq!(store.active, DEFAULT_ACCOUNT_ID);
        assert_eq!(store.accounts.len(), 1);
        assert_eq!(store.accounts.get(DEFAULT_ACCOUNT_ID), Some(&t));
        assert_eq!(store.active_tokens(), Some(&t));
    }

    #[test]
    fn v2_store_round_trips_without_migration() {
        let mut store = OAuthAccountStore::new_single("default", sample_tokens("a"));
        store.add("work", sample_tokens("b"));
        let json = serde_json::to_string(&store).unwrap();
        let parsed = parse_account_store(&json).unwrap();
        assert_eq!(parsed, store);
        assert_eq!(parsed.active, "work");
    }

    #[test]
    fn add_list_switch_remove_round_trip_with_active_promotion() {
        let mut store = OAuthAccountStore::new_single("personal", sample_tokens("p"));
        store.add("work", sample_tokens("w"));
        assert_eq!(store.active, "work", "add() makes the new account active");

        let listed = store.list();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|(id, _, active)| id == "work" && *active));
        assert!(listed
            .iter()
            .any(|(id, _, active)| id == "personal" && !*active));

        store.switch("personal").unwrap();
        assert_eq!(store.active, "personal");
        assert_eq!(store.active_tokens(), Some(&sample_tokens("p")));

        assert!(
            store.switch("nope").is_err(),
            "switching to an unknown id must error"
        );

        // Removing the active account promotes a remaining one (id order: "work" < ...).
        assert!(store.remove("personal"));
        assert_eq!(store.active, "work", "active promoted after removing it");
        assert_eq!(store.accounts.len(), 1);

        // Removing the last account empties the map (the I/O wrapper deletes the whole entry).
        assert!(store.remove("work"));
        assert!(store.accounts.is_empty());

        assert!(!store.remove("gone"), "removing an unknown id is a no-op");
    }

    #[test]
    fn set_active_tokens_updates_only_the_active_account() {
        let mut store = OAuthAccountStore::new_single("personal", sample_tokens("p"));
        store.add("work", sample_tokens("w"));
        store.switch("personal").unwrap();

        let refreshed = OAuthTokens {
            access_token: "refreshed".into(),
            ..sample_tokens("p")
        };
        store.set_active_tokens(refreshed.clone());

        assert_eq!(store.accounts.get("personal"), Some(&refreshed));
        assert_eq!(
            store.accounts.get("work"),
            Some(&sample_tokens("w")),
            "the non-active account must be untouched by a refresh"
        );
        assert_eq!(store.active, "personal");
    }

    #[test]
    fn set_tokens_updates_only_the_named_account() {
        let mut store = OAuthAccountStore::new_single("personal", sample_tokens("p"));
        store.add("work", sample_tokens("w"));
        // Active is "work" after add; refresh the non-active "personal" account by id.
        assert_eq!(store.active, "work");
        let refreshed = OAuthTokens {
            access_token: "refreshed-personal".into(),
            ..sample_tokens("p")
        };
        store.set_tokens("personal", refreshed.clone()).unwrap();
        assert_eq!(store.accounts.get("personal"), Some(&refreshed));
        assert_eq!(
            store.accounts.get("work"),
            Some(&sample_tokens("w")),
            "active account must be untouched when refreshing another by id"
        );
        assert_eq!(store.active, "work", "active pointer must not move");
        assert!(store.set_tokens("nope", sample_tokens("x")).is_err());
    }

    #[test]
    fn account_pool_round_robins_and_skips_single_account() {
        let pool = OAuthAccountPool::from_ids(vec!["a".into(), "b".into(), "c".into()]);
        assert!(pool.has_rotation());
        assert_eq!(pool.next().as_deref(), Some("a"));
        assert_eq!(pool.next().as_deref(), Some("b"));
        assert_eq!(pool.next().as_deref(), Some("c"));
        assert_eq!(pool.next().as_deref(), Some("a"), "wraps");

        let single = OAuthAccountPool::from_ids(vec!["only".into()]);
        assert!(!single.has_rotation());
        assert_eq!(single.next(), None);

        let empty = OAuthAccountPool::default();
        assert!(!empty.has_rotation());
        assert!(empty.is_empty());
    }

    #[test]
    fn account_pool_from_store_seeds_cursor_at_active() {
        let mut store = OAuthAccountStore::new_single("alice", sample_tokens("a"));
        store.add("bob", sample_tokens("b"));
        store.add("carol", sample_tokens("c"));
        store.switch("bob").unwrap();
        // BTreeMap order: alice, bob, carol — active bob is index 1.
        let pool = OAuthAccountPool::from_store(&store);
        assert!(pool.has_rotation());
        assert_eq!(pool.active(), Some("bob"));
        assert_eq!(pool.next().as_deref(), Some("bob"));
        assert_eq!(pool.next().as_deref(), Some("carol"));
        assert_eq!(pool.next().as_deref(), Some("alice"));
    }

    #[test]
    fn next_default_id_finds_first_free_slot() {
        let mut existing = std::collections::BTreeMap::new();
        assert_eq!(next_default_id(&existing), "account-1");
        existing.insert("account-1".to_string(), sample_tokens("a"));
        assert_eq!(next_default_id(&existing), "account-2");
        existing.insert("account-3".to_string(), sample_tokens("c"));
        // account-2 is still free even though account-3 is taken.
        assert_eq!(next_default_id(&existing), "account-2");
    }
}
