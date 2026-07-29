//! First-run onboarding and subscription quota support for interactive chat.

use super::*;

/// On a fresh machine (no keys, no bridge, no config) offer the `forge init` wizard before the
/// first chat. Skipped for `--mock`, non-interactive shells, and once anything is configured.
/// Declining writes an (empty) config so we don't nag on every launch.
pub(crate) fn maybe_first_run_setup(mock: bool) -> Result<()> {
    if mock || !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Ok(());
    }
    let has_any_key = forge_config::known_key_providers().any(forge_config::has_api_key);
    let any_bridge = forge_provider::CliKind::all().iter().any(|k| k.available());
    if !needs_onboarding(has_any_key, any_bridge, forge_config::user_config_exists()) {
        return Ok(());
    }
    println!("⚒ Welcome to Forge — no providers are configured yet.");
    let yes = prompt_line("Run guided setup now? [Y/n]: ")?;
    if yes.is_empty() || yes.eq_ignore_ascii_case("y") || yes.eq_ignore_ascii_case("yes") {
        setup()?;
    } else {
        let _ = forge_config::write_subscriptions(&std::collections::HashMap::new());
        println!("Skipped. Run `forge setup` anytime, or `forge auth <provider>` to add a key.");
    }
    Ok(())
}

/// Probe Claude's current rate limits and record them into the session store. Best-effort; the
/// caller gates it on staleness.
pub(crate) async fn refresh_claude_quota(session: &std::sync::Arc<tokio::sync::Mutex<Session>>) {
    let limits = tokio::task::spawn_blocking(bridge_stats::probe_claude_limits)
        .await
        .unwrap_or_default();
    if !limits.is_empty() {
        let s = session.lock().await;
        for (w, f) in limits {
            s.seed_subscription_quota("claude-cli", &w, Some(f * 100.0));
        }
    }
}

/// Whether the stored Claude quota is older than `max_age` seconds (or absent).
pub(crate) async fn claude_quota_is_stale(
    session: &std::sync::Arc<tokio::sync::Mutex<Session>>,
    max_age: i64,
) -> bool {
    session
        .lock()
        .await
        .claude_quota_age_secs()
        .is_none_or(|age| age > max_age)
}

/// Sends the turn-complete signal on drop so an aborted turn releases `busy`.
pub(crate) struct DoneGuard(pub(crate) std::sync::mpsc::Sender<u64>, pub(crate) u64);

impl Drop for DoneGuard {
    fn drop(&mut self) {
        let _ = self.0.send(self.1);
    }
}
