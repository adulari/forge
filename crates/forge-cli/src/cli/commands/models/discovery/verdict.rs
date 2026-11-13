//! Turning a discovery round into persisted key verdicts, and into log lines only when something
//! changed. The owner's expired OpenCode key logged the same warning every round for two weeks
//! (11,138 times) while no screen said a word.

use forge_provider::{classify_key_failure, KeyFailure};
use forge_store::{Store, VerdictChange};

use super::{DiscoveryStatusKind, ProviderDiscoveryStatus};

fn has_no_key_to_renew(provider: &str) -> bool {
    provider == "ollama"
        || forge_provider::CliKind::all()
            .into_iter()
            .any(|kind| kind.prefix() == provider)
}

/// The key verdict a discovery status carries, if its listing failed because the provider
/// refused the credential.
pub(crate) fn rejection(status: &ProviderDiscoveryStatus) -> Option<(KeyFailure, String)> {
    if status.kind != DiscoveryStatusKind::Failed || has_no_key_to_renew(&status.provider) {
        return None;
    }
    classify_key_failure(status.detail.as_deref()?)
}

/// Record this round's verdicts: a refused key is set (or refreshed), a provider that listed
/// models clears its verdict, anything else (a timeout, a blip) leaves the verdict as it was.
/// Returns the lines worth logging, one per CHANGE.
pub(super) fn observe(
    store: &Store,
    statuses: &[ProviderDiscoveryStatus],
    now: i64,
) -> Vec<String> {
    let mut lines = Vec::new();
    for status in statuses {
        if let Some((kind, reason)) = rejection(status) {
            let change = store.set_provider_verdict(&status.provider, kind.as_str(), &reason, now);
            if let Ok(VerdictChange::New | VerdictChange::Changed) = change {
                lines.push(format!(
                    "{}: {} ({reason}) — `forge auth {}` to replace the key; not warning again \
                     until this changes",
                    status.provider,
                    kind.headline(),
                    status.provider
                ));
            }
        } else if status.kind == DiscoveryStatusKind::Discovered
            && store
                .clear_provider_verdict(&status.provider)
                .unwrap_or(false)
        {
            lines.push(format!(
                "{}: key works again, verdict cleared",
                status.provider
            ));
        }
    }
    lines
}

/// [`observe`] against the real store, logging the changes.
pub(super) fn record(statuses: &[ProviderDiscoveryStatus]) {
    let Ok(store) = crate::open_store() else {
        return;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    for line in observe(&store, statuses, now) {
        tracing::warn!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(
        provider: &str,
        kind: DiscoveryStatusKind,
        detail: Option<&str>,
    ) -> ProviderDiscoveryStatus {
        ProviderDiscoveryStatus {
            provider: provider.into(),
            kind,
            models: 0,
            detail: detail.map(str::to_string),
        }
    }

    const EXPIRED: &str = r#"provider auth failed: HttpError status: 401, body: {"error":"This API key has expired"}"#;

    #[test]
    fn expired_then_same_again_then_success() {
        let store = Store::open_in_memory().unwrap();
        let failed = [status(
            "opencode",
            DiscoveryStatusKind::Failed,
            Some(EXPIRED),
        )];

        let first = observe(&store, &failed, 100);
        assert_eq!(first.len(), 1, "a new verdict is logged once: {first:?}");
        assert!(first[0].contains("Key expired"));
        let v = &store.provider_verdicts().unwrap()[0];
        assert_eq!(
            (v.status.as_str(), v.reason.as_str()),
            ("expired", "This API key has expired")
        );

        assert!(
            observe(&store, &failed, 700).is_empty(),
            "unchanged verdict stays quiet"
        );
        let v = &store.provider_verdicts().unwrap()[0];
        assert_eq!((v.first_seen, v.last_seen), (100, 700));

        let ok = [status("opencode", DiscoveryStatusKind::Discovered, None)];
        let cleared = observe(&store, &ok, 800);
        assert_eq!(cleared.len(), 1);
        assert!(store.provider_verdicts().unwrap().is_empty());
        assert!(observe(&store, &ok, 900).is_empty());
    }

    #[test]
    fn a_timeout_or_blip_keeps_the_verdict_and_bridges_never_get_one() {
        let store = Store::open_in_memory().unwrap();
        observe(
            &store,
            &[status("groq", DiscoveryStatusKind::Failed, Some(EXPIRED))],
            1,
        );
        observe(
            &store,
            &[status(
                "groq",
                DiscoveryStatusKind::TimedOut,
                Some("after 8s"),
            )],
            2,
        );
        observe(
            &store,
            &[status(
                "groq",
                DiscoveryStatusKind::Failed,
                Some("provider unavailable: 503"),
            )],
            3,
        );
        assert_eq!(store.provider_verdicts().unwrap().len(), 1);

        let bridge = [status(
            "claude-cli",
            DiscoveryStatusKind::Failed,
            Some("401 Not logged in"),
        )];
        assert!(observe(&store, &bridge, 4).is_empty());
        assert_eq!(store.provider_verdicts().unwrap().len(), 1);
    }
}
