//! How the bridged `claude` process authenticates, and the environment it is spawned with.
//!
//! The bridge runs with its own `CLAUDE_CONFIG_DIR` (hook isolation, see `claude_bridge_home`), so
//! Claude Code's refresh lock for the user's interactive login does not cover it. Sharing that
//! login with the bridge let the bridged claude refresh and rotate the single-use refresh token on
//! its own, which invalidated the user's main `~/.claude` login about once a day. The bridge
//! therefore never touches `.credentials.json`; it authenticates with a long-lived token from
//! `claude setup-token` (`CLAUDE_CODE_OAUTH_TOKEN`, or the `claude_oauth_token` Forge secret).
//! Without a token (or an `ANTHROPIC_API_KEY`, which has no refresh) the bridge is not configured:
//! [`bridge_auth_present`] is false, routing skips it quietly, and a direct spawn reports
//! [`BRIDGE_SETUP_HINT`] once.

use std::ffi::OsString;
use std::path::PathBuf;

pub const OAUTH_TOKEN_ENV: &str = "CLAUDE_CODE_OAUTH_TOKEN";
const CLAUDE_AUTO_MEMORY_ENV: &str = "CLAUDE_CODE_DISABLE_AUTO_MEMORY";

/// What the claude bridge process authenticates with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeAuth {
    OAuthToken(String),
    ApiKey,
    Missing,
}

/// The one actionable line shown when the bridge has no token.
pub const BRIDGE_SETUP_HINT: &str = "claude bridge is not configured: run `claude setup-token`, \
then `forge auth claude_oauth_token` (or export CLAUDE_CODE_OAUTH_TOKEN for forge serve). Forge \
no longer shares your interactive `claude` login with the bridge.";

fn env_set(key: &str) -> bool {
    std::env::var(key).is_ok_and(|v| !v.trim().is_empty())
}

// Test seam: bridge tests drive fake `claude` scripts and need a configured bridge. A test that
// checks the unconfigured path turns this off for its own thread.
#[cfg(test)]
thread_local! {
    pub(crate) static ASSUME_TOKEN: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

pub fn resolve_bridge_auth() -> BridgeAuth {
    if let Some(token) = forge_config::claude_oauth_token() {
        BridgeAuth::OAuthToken(token)
    } else if env_set("ANTHROPIC_API_KEY") {
        BridgeAuth::ApiKey
    } else if cfg!(test) && assume_test_token() {
        BridgeAuth::OAuthToken("test-token".into())
    } else {
        BridgeAuth::Missing
    }
}

#[cfg(test)]
fn assume_test_token() -> bool {
    ASSUME_TOKEN.with(|c| c.get())
}
#[cfg(not(test))]
fn assume_test_token() -> bool {
    false
}

pub fn bridge_auth_present() -> bool {
    resolve_bridge_auth() != BridgeAuth::Missing
}

/// Pure inputs of [`bridge_env_from`], so the env is testable without touching process state.
pub struct BridgeEnvInputs {
    pub auth: BridgeAuth,
    pub proxy: Option<String>,
    pub auto_memory_user_set: bool,
    pub real_home: Option<PathBuf>,
    pub bridge_home: Option<PathBuf>,
}

/// The environment of every `claude` process Forge drives. `Err` carries [`BRIDGE_SETUP_HINT`].
///
/// - `CLAUDE_CODE_OAUTH_TOKEN`: the explicit long-lived token, never the interactive login.
/// - `CLAUDE_CONFIG_DIR`: the hook-free mirror (which never contains `.credentials.json`).
/// - `ANTHROPIC_BASE_URL`: the Headroom proxy when configured.
/// - `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` unless the user set it: Forge owns session memory.
pub fn bridge_env_from(
    inputs: BridgeEnvInputs,
) -> Result<Vec<(&'static str, OsString)>, &'static str> {
    let mut env: Vec<(&'static str, OsString)> = Vec::new();
    match &inputs.auth {
        BridgeAuth::Missing => return Err(BRIDGE_SETUP_HINT),
        BridgeAuth::OAuthToken(token) => env.push((OAUTH_TOKEN_ENV, token.into())),
        BridgeAuth::ApiKey => {}
    }
    if let Some(proxy) = inputs.proxy {
        env.push(("ANTHROPIC_BASE_URL", proxy.into()));
    }
    if !inputs.auto_memory_user_set {
        env.push((CLAUDE_AUTO_MEMORY_ENV, "1".into()));
    }
    if let (Some(real_home), Some(bridge_home)) = (inputs.real_home, inputs.bridge_home) {
        match crate::claude_bridge_home::prepare_claude_bridge_home(&real_home, &bridge_home) {
            Ok(()) => env.push(("CLAUDE_CONFIG_DIR", bridge_home.into_os_string())),
            Err(e) => tracing::warn!(
                "failed to prepare isolated claude bridge home ({e}); running with the user's own \
claude config (hooks may fire)"
            ),
        }
    }
    Ok(env)
}

/// [`bridge_env_from`] over the live process state.
pub fn claude_bridge_env() -> Result<Vec<(&'static str, OsString)>, &'static str> {
    bridge_env_from(BridgeEnvInputs {
        auth: resolve_bridge_auth(),
        proxy: forge_config::headroom_route().map(str::to_string),
        auto_memory_user_set: std::env::var_os(CLAUDE_AUTO_MEMORY_ENV).is_some(),
        real_home: crate::claude_bridge_home::real_claude_config_dir(),
        bridge_home: forge_config::config_dir()
            .map(|base| base.join(crate::claude_bridge_home::BRIDGE_HOME_DIR_NAME)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(
        auth: BridgeAuth,
        real: &std::path::Path,
        bridge: &std::path::Path,
    ) -> BridgeEnvInputs {
        BridgeEnvInputs {
            auth,
            proxy: None,
            auto_memory_user_set: false,
            real_home: Some(real.to_path_buf()),
            bridge_home: Some(bridge.to_path_buf()),
        }
    }

    fn get<'a>(env: &'a [(&'static str, OsString)], key: &str) -> Option<&'a OsString> {
        env.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    #[test]
    fn token_is_passed_explicitly_and_config_dir_is_the_bridge_home() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join(".credentials.json"), "{}").unwrap();
        let bridge = root.path().join("bridge");
        let env =
            bridge_env_from(inputs(BridgeAuth::OAuthToken("tok".into()), &real, &bridge)).unwrap();
        assert_eq!(get(&env, OAUTH_TOKEN_ENV).unwrap(), "tok");
        assert_eq!(get(&env, "CLAUDE_CONFIG_DIR").unwrap(), bridge.as_os_str());
        assert!(
            std::fs::symlink_metadata(bridge.join(".credentials.json")).is_err(),
            "the interactive login is never exposed to the bridge"
        );
        assert!(real.join(".credentials.json").exists());
    }

    #[test]
    fn api_key_auth_passes_no_token_but_keeps_isolation() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        let bridge = root.path().join("bridge");
        let env = bridge_env_from(inputs(BridgeAuth::ApiKey, &real, &bridge)).unwrap();
        assert!(get(&env, OAUTH_TOKEN_ENV).is_none());
        assert!(get(&env, "CLAUDE_CONFIG_DIR").is_some());
    }

    #[test]
    fn no_token_fails_with_the_setup_hint_and_builds_nothing() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        let bridge = root.path().join("bridge");
        let err = bridge_env_from(inputs(BridgeAuth::Missing, &real, &bridge)).unwrap_err();
        assert!(err.contains("claude setup-token"));
        assert!(err.contains("forge auth claude_oauth_token"));
        assert!(!bridge.exists());
    }

    #[test]
    fn auto_memory_and_proxy_follow_their_inputs() {
        let root = tempfile::tempdir().unwrap();
        let mut i = inputs(
            BridgeAuth::OAuthToken("t".into()),
            &root.path().join("missing"),
            &root.path().join("bridge"),
        );
        i.proxy = Some("http://127.0.0.1:1".into());
        i.auto_memory_user_set = true;
        let env = bridge_env_from(i).unwrap();
        assert!(get(&env, CLAUDE_AUTO_MEMORY_ENV).is_none());
        assert_eq!(
            get(&env, "ANTHROPIC_BASE_URL").unwrap(),
            "http://127.0.0.1:1"
        );
    }
}
