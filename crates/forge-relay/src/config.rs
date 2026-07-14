//! Relay configuration, loaded once at startup from the environment (Fly.io secrets/env vars
//! land here the same way `crate::apns::ApnsConfig::from_env` reads them in `forge-cli`).

/// Default allowlisted topics if `FORGE_RELAY_ALLOWED_TOPICS` is unset — the one app this relay
/// exists for today, plus its Live Activity variant (see `crates/forge-cli/src/apns.rs`'s
/// `APNS_BUNDLE_ID` + `dispatch_live_activity`'s `{bundle}.push-type.liveactivity` topic).
const DEFAULT_ALLOWED_TOPICS: &str = "dev.adulari.forge,dev.adulari.forge.push-type.liveactivity";

pub(crate) struct RelayConfig {
    pub(crate) team_id: String,
    pub(crate) key_id: String,
    pub(crate) key_pem: String,
    pub(crate) allowed_topics: Vec<String>,
    pub(crate) relay_token: Option<String>,
    pub(crate) port: u16,
    /// Per-device-token / per-IP send budget within `rate_window_secs` (see `ratelimit.rs`).
    pub(crate) rate_limit_per_window: u32,
    pub(crate) rate_window_secs: u64,
    /// Circuit breaker: total accepted sends/24h across all callers, above which the relay
    /// starts rejecting (loudly) rather than silently dropping — see module docs on `main.rs`.
    pub(crate) daily_send_cap: u64,
}

impl RelayConfig {
    /// Reads the Apple credential (`FORGE_APNS_TEAM_ID`/`_KEY_ID`/`_KEY_PATH`, same var names
    /// `forge-cli`'s `ApnsConfig::from_env` uses — this process is the one place they're
    /// actually meant to live) plus relay-specific tuning. Unlike `forge-cli`'s optional-feature
    /// degrade-to-`None`, this binary's entire purpose is being an APNs sender, so a missing
    /// credential is a hard startup error, not a graceful no-op.
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        let team_id = std::env::var("FORGE_APNS_TEAM_ID")
            .map_err(|_| anyhow::anyhow!("FORGE_APNS_TEAM_ID is required"))?;
        let key_id = std::env::var("FORGE_APNS_KEY_ID")
            .map_err(|_| anyhow::anyhow!("FORGE_APNS_KEY_ID is required"))?;
        // Either the raw PEM (Fly secret set directly as an env var, no file on disk at all —
        // preferred: the .p8 never touches the container filesystem) or a path to it.
        let key_pem = match std::env::var("FORGE_APNS_KEY_PEM") {
            Ok(pem) => pem,
            Err(_) => {
                let path = std::env::var("FORGE_APNS_KEY_PATH").map_err(|_| {
                    anyhow::anyhow!("FORGE_APNS_KEY_PEM or FORGE_APNS_KEY_PATH is required")
                })?;
                std::fs::read_to_string(&path)
                    .map_err(|e| anyhow::anyhow!("reading FORGE_APNS_KEY_PATH ({path}): {e}"))?
            }
        };
        let allowed_topics = std::env::var("FORGE_RELAY_ALLOWED_TOPICS")
            .unwrap_or_else(|_| DEFAULT_ALLOWED_TOPICS.to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let relay_token = std::env::var("FORGE_RELAY_TOKEN")
            .ok()
            .filter(|token| !token.is_empty());
        let port = std::env::var("PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8787);
        let rate_limit_per_window = std::env::var("FORGE_RELAY_RATE_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);
        let rate_window_secs = std::env::var("FORGE_RELAY_RATE_WINDOW_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);
        let daily_send_cap = std::env::var("FORGE_RELAY_DAILY_SEND_CAP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50_000);

        Ok(Self {
            team_id,
            key_id,
            key_pem,
            allowed_topics,
            relay_token,
            port,
            rate_limit_per_window,
            rate_window_secs,
            daily_send_cap,
        })
    }
}
