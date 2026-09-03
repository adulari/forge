//! Whether a CLI bridge has credentials at all — the cheap, no-subprocess check that keeps a
//! keyless first run from paying a full failover sweep to learn what `forge doctor` reports in
//! milliseconds.
//!
//! Two evidence sources, both process-scoped:
//!
//! * **Static** — the CLI's own on-disk credential file (and the API-key env vars it honours).
//!   Only used to prove [`CliCredentials::Absent`] where the CLI is known to keep its tokens in a
//!   file on this platform; anywhere it might use an OS keychain the verdict stays
//!   [`CliCredentials::Unknown`], because a wrong `Absent` would silently remove a working
//!   provider from the mesh.
//! * **Live** — [`note_unauthenticated`], recorded when the CLI itself says it is not signed in
//!   (a model-discovery probe or a failed turn). This is what covers bridges whose credential
//!   store Forge cannot locate.
//!
//! The cache is deliberately per-PROCESS: a user who signs a CLI in gets the real verdict on the
//! next `forge run`, and a long-lived `forge serve` re-probes the file on its next process. It is
//! never persisted.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use super::CliKind;

/// What Forge knows about a bridge's login state right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliCredentials {
    /// Positive evidence of credentials (a token file, or an API key the CLI honours).
    Present,
    /// Positive evidence of NO credentials: the CLI said so, or its credential file is missing on
    /// a platform where that file is the only place it can be.
    Absent,
    /// No evidence either way — treat the bridge as routable and let the turn decide.
    Unknown,
}

/// Live "this CLI is not signed in" verdicts observed in this process, keyed by bridge prefix.
fn live_verdicts() -> &'static Mutex<HashMap<&'static str, bool>> {
    static LIVE: OnceLock<Mutex<HashMap<&'static str, bool>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record that `kind` told us it is not authenticated. `evidence` is logged, never shown: it is
/// the CLI's raw output and may contain a full OAuth URL.
pub fn note_unauthenticated(kind: CliKind, evidence: &str) {
    tracing::debug!(
        "{} reported no credentials; skipping it for the rest of this process: {evidence}",
        kind.prefix()
    );
    if let Ok(mut map) = live_verdicts().lock() {
        map.insert(kind.prefix(), true);
    }
}

/// Forget every live verdict. Tests only — the process cache is otherwise write-once per bridge.
#[cfg(test)]
pub fn reset_live_verdicts() {
    if let Ok(mut map) = live_verdicts().lock() {
        map.clear();
    }
}

/// Serializes the tests that mutate this module's process-global state (env vars and the live
/// verdict cache), which the whole crate's test binary shares.
#[cfg(test)]
pub fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn noted_unauthenticated(kind: CliKind) -> bool {
    live_verdicts()
        .lock()
        .map(|map| map.get(kind.prefix()).copied().unwrap_or(false))
        .unwrap_or(false)
}

/// A non-empty environment variable.
fn env_set(key: &str) -> bool {
    std::env::var(key).is_ok_and(|v| !v.trim().is_empty())
}

fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

/// The claude config dir the CLI itself reads (`$CLAUDE_CONFIG_DIR`, else `~/.claude`). Mirrors
/// `claude_bridge_home::real_claude_config_dir`, which resolves the same pair for the isolated
/// bridge home.
fn claude_config_dir() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(std::path::PathBuf::from(dir));
    }
    Some(home_dir()?.join(".claude"))
}

/// `${CODEX_HOME:-~/.codex}/auth.json` — the same resolution `quota::codex_cli_detected_plan`
/// uses to read the CLI's ChatGPT plan.
fn codex_auth_path() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("CODEX_HOME") {
        return Some(std::path::PathBuf::from(dir).join("auth.json"));
    }
    Some(home_dir()?.join(".codex").join("auth.json"))
}

/// Static, file/env-only credential evidence for one bridge. No subprocess, no network.
fn static_credentials(kind: CliKind) -> CliCredentials {
    match kind {
        CliKind::ClaudeCode => {
            if env_set("ANTHROPIC_API_KEY") || env_set("CLAUDE_CODE_OAUTH_TOKEN") {
                return CliCredentials::Present;
            }
            let Some(dir) = claude_config_dir() else {
                return CliCredentials::Unknown;
            };
            if dir.join(".credentials.json").exists() {
                return CliCredentials::Present;
            }
            // macOS Claude Code keeps its OAuth tokens in the login keychain, so a missing file
            // proves nothing there. Elsewhere the file is the store.
            if cfg!(target_os = "macos") {
                CliCredentials::Unknown
            } else {
                CliCredentials::Absent
            }
        }
        CliKind::Codex => {
            if env_set("OPENAI_API_KEY") {
                return CliCredentials::Present;
            }
            match codex_auth_path() {
                Some(path) if path.exists() => CliCredentials::Present,
                Some(_) => CliCredentials::Absent,
                None => CliCredentials::Unknown,
            }
        }
        // Antigravity stores its Google session inside its VS Code-derived profile, whose layout
        // Forge does not track. Its own "please sign in" output (via `note_unauthenticated`) is
        // the only verdict we trust for it.
        CliKind::Antigravity => CliCredentials::Unknown,
    }
}

/// The current credential verdict for `kind`: a live "not signed in" report wins over static
/// evidence, because the CLI is the authority on its own login.
pub fn credentials(kind: CliKind) -> CliCredentials {
    if noted_unauthenticated(kind) {
        return CliCredentials::Absent;
    }
    static_credentials(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::test_guard as env_guard;

    struct ScopedEnv(Vec<(&'static str, Option<std::ffi::OsString>)>);

    impl ScopedEnv {
        fn set(vars: &[(&'static str, Option<&str>)]) -> Self {
            let saved = vars
                .iter()
                .map(|(k, _)| (*k, std::env::var_os(k)))
                .collect();
            for (k, v) in vars {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
            Self(saved)
        }
    }

    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            for (k, v) in &self.0 {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    /// The defect this module exists for: on a clean HOME every bridge that keeps its token in a
    /// file must be knowably credential-less BEFORE anything is spawned.
    #[test]
    fn a_clean_home_is_absent_for_the_file_backed_bridges() {
        let _guard = env_guard();
        let home = tempfile::tempdir().unwrap();
        let _env = ScopedEnv::set(&[
            ("HOME", home.path().to_str()),
            ("USERPROFILE", home.path().to_str()),
            ("CLAUDE_CONFIG_DIR", None),
            ("CODEX_HOME", None),
            ("ANTHROPIC_API_KEY", None),
            ("CLAUDE_CODE_OAUTH_TOKEN", None),
            ("OPENAI_API_KEY", None),
        ]);
        assert_eq!(static_credentials(CliKind::Codex), CliCredentials::Absent);
        let claude = static_credentials(CliKind::ClaudeCode);
        if cfg!(target_os = "macos") {
            assert_eq!(
                claude,
                CliCredentials::Unknown,
                "the macOS keychain means a missing file proves nothing"
            );
        } else {
            assert_eq!(claude, CliCredentials::Absent);
        }
        assert_eq!(
            static_credentials(CliKind::Antigravity),
            CliCredentials::Unknown,
            "a bridge whose credential store we cannot locate must never be assumed logged out"
        );
    }

    /// A false `Absent` silently deletes a working provider from the mesh, so every positive
    /// signal must win.
    #[test]
    fn a_token_file_or_an_api_key_is_present() {
        let _guard = env_guard();
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();
        std::fs::write(home.path().join(".claude/.credentials.json"), "{}").unwrap();
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        std::fs::write(home.path().join(".codex/auth.json"), "{}").unwrap();
        let _env = ScopedEnv::set(&[
            ("HOME", home.path().to_str()),
            ("USERPROFILE", home.path().to_str()),
            ("CLAUDE_CONFIG_DIR", None),
            ("CODEX_HOME", None),
            ("ANTHROPIC_API_KEY", None),
            ("CLAUDE_CODE_OAUTH_TOKEN", None),
            ("OPENAI_API_KEY", None),
        ]);
        assert_eq!(
            static_credentials(CliKind::ClaudeCode),
            CliCredentials::Present
        );
        assert_eq!(static_credentials(CliKind::Codex), CliCredentials::Present);

        let empty = tempfile::tempdir().unwrap();
        let _env = ScopedEnv::set(&[
            ("HOME", empty.path().to_str()),
            ("USERPROFILE", empty.path().to_str()),
            ("ANTHROPIC_API_KEY", Some("sk-test")),
            ("OPENAI_API_KEY", Some("sk-test")),
        ]);
        assert_eq!(
            static_credentials(CliKind::ClaudeCode),
            CliCredentials::Present,
            "an API key the CLI honours is a credential"
        );
        assert_eq!(static_credentials(CliKind::Codex), CliCredentials::Present);
    }

    /// The live verdict is what covers a bridge with no locatable credential file, and it must
    /// override even positive static evidence — the CLI is the authority on its own login.
    #[test]
    fn a_live_not_signed_in_report_overrides_static_evidence() {
        let _guard = env_guard();
        reset_live_verdicts();
        let _env = ScopedEnv::set(&[("OPENAI_API_KEY", Some("sk-test"))]);
        assert_eq!(credentials(CliKind::Codex), CliCredentials::Present);
        note_unauthenticated(CliKind::Codex, "codex error: Not logged in");
        assert_eq!(credentials(CliKind::Codex), CliCredentials::Absent);
        assert_ne!(
            credentials(CliKind::Antigravity),
            CliCredentials::Absent,
            "a verdict must not leak across bridges"
        );
        reset_live_verdicts();
        assert_eq!(
            credentials(CliKind::Codex),
            CliCredentials::Present,
            "the verdict is process-scoped state, not a persisted fact"
        );
    }
}
