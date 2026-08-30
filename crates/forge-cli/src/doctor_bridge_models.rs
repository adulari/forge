use crate::doctor::{check, Check, Status};

/// Report whether each installed bridge's inventory came from the CLI, a last-known-good cache,
/// or Forge's release-age fallback. This is separate from turn liveness: a bridge may answer turns
/// while model enumeration fails, and silently treating that fallback as current caused stale
/// aliases to drive routing.
pub(crate) async fn checks() -> Vec<Check> {
    let probes = forge_provider::CliKind::all()
        .into_iter()
        .filter(|kind| kind.available())
        .map(|kind| async move {
            let discovered = kind.bridge_models_detailed().await;
            let unverified = discovered.is_unverified();
            check(
                if unverified { Status::Warn } else { Status::Ok },
                &format!("{} models", kind.prefix()),
                discovered.describe(),
                unverified.then_some(match kind {
                    forge_provider::CliKind::ClaudeCode => {
                        "run `claude` once to log in, then rerun `forge models`"
                    }
                    forge_provider::CliKind::Codex => {
                        "upgrade/login to Codex, then rerun `forge models`"
                    }
                    forge_provider::CliKind::Antigravity => {
                        "run `agy` to sign in, then rerun `forge models`"
                    }
                }),
            )
        });
    futures::future::join_all(probes).await
}
