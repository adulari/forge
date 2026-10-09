//! What discovery does when providers are unreachable: which failures are worth retrying, how long
//! to keep retrying, and what the catalog keeps meanwhile.
//!
//! The daemon starts at login, a second or two before DNS works, so the first listing of every
//! keyed provider fails. That is expected and transient; it must neither be retried forever nor
//! cost the catalog the models those providers listed an hour ago.

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use forge_mesh::ModelCatalog;

use super::{DiscoveryStatusKind, ProviderDiscoveryStatus};

/// Delays before retrying a discovery that left a keyed provider unreachable; after these,
/// [`DISCOVERY_RETRY_STEADY`] repeats at most [`DISCOVERY_RETRY_STEADY_ROUNDS`] times.
pub(super) const DISCOVERY_RETRY_BACKOFF: [Duration; 3] = [
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(60),
];
pub(super) const DISCOVERY_RETRY_STEADY: Duration = Duration::from_secs(600);
/// A provider still failing ~35 minutes after startup is not a boot race (key revoked, endpoint
/// retired, listing unsupported); every further round would re-run the whole discovery sweep for
/// nothing, so the loop stops and says so once.
pub(super) const DISCOVERY_RETRY_STEADY_ROUNDS: usize = 3;

/// How long a provider's models are carried over from the previous catalog while its listing keeps
/// failing. Past this they are dropped: a retired key must not advertise models for ever.
const CARRY_MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60;

fn is_cli_bridge(provider: &str) -> bool {
    forge_provider::CliKind::all()
        .into_iter()
        .any(|kind| kind.prefix() == provider)
}

/// Keyed API providers whose listing failed or timed out. Keyless `ollama` is excluded (not
/// running is normal) and so are the CLI bridges: they have no API key, but `has_api_key` answers
/// `true` for any provider it has no key variable for, which made one slow `agy models` look like
/// an unreachable keyed provider and re-ran the whole discovery every ten minutes for the life of
/// the daemon.
pub(super) fn keyed_discovery_failures(
    statuses: &[ProviderDiscoveryStatus],
    has_key: impl Fn(&str) -> bool,
) -> Vec<&ProviderDiscoveryStatus> {
    statuses
        .iter()
        .filter(|s| {
            matches!(
                s.kind,
                DiscoveryStatusKind::Failed | DiscoveryStatusKind::TimedOut
            )
        })
        .filter(|s| s.provider != "ollama" && !is_cli_bridge(&s.provider) && has_key(&s.provider))
        .collect()
}

fn describe_failures(failing: &[&ProviderDiscoveryStatus]) -> String {
    let names: Vec<&str> = failing.iter().map(|s| s.provider.as_str()).collect();
    let cause = failing
        .iter()
        .find_map(|s| s.detail.as_deref())
        .map(|d| d.chars().take(160).collect::<String>())
        .unwrap_or_default();
    format!("{} ({cause})", names.join(", "))
}

/// One concise warning for callers that run a single discovery and have no retry of their own.
pub(super) fn warn_unreachable(
    statuses: &[ProviderDiscoveryStatus],
    has_key: impl Fn(&str) -> bool,
) {
    let failing = keyed_discovery_failures(statuses, has_key);
    if !failing.is_empty() {
        tracing::warn!(
            "model discovery: keyed provider(s) unreachable: {}",
            describe_failures(&failing)
        );
    }
}

/// Run `discover` until no keyed provider is left failing, sleeping `backoff` then up to
/// `steady_rounds` times `steady` between attempts. `publish(catalog, late)` receives every
/// result; `late` is true for retries. Logs one warning when the first attempt leaves providers
/// unreachable and one when it gives up; recovery and in-between retries are `info`.
pub(super) async fn discover_until_keyed_providers_answer<D, Fut, P>(
    backoff: &[Duration],
    steady: Duration,
    steady_rounds: usize,
    has_key: impl Fn(&str) -> bool,
    mut discover: D,
    mut publish: P,
) where
    D: FnMut() -> Fut,
    Fut: std::future::Future<Output = (ModelCatalog, Vec<ProviderDiscoveryStatus>)>,
    P: FnMut(ModelCatalog, bool),
{
    let mut attempt = 0usize;
    loop {
        let (catalog, statuses) = discover().await;
        let failing = keyed_discovery_failures(&statuses, &has_key);
        let summary = describe_failures(&failing);
        let all_answered = failing.is_empty();
        publish(catalog, attempt > 0);
        if all_answered {
            if attempt > 0 {
                tracing::info!("model discovery recovered after {attempt} retry round(s)");
            }
            return;
        }
        let delay = match backoff.get(attempt) {
            Some(delay) => *delay,
            None if attempt - backoff.len() < steady_rounds => steady,
            None => {
                tracing::warn!(
                    "model discovery gave up after {attempt} retry round(s); still unreachable: \
                     {summary} — models last listed for them stay in the catalog for a while; \
                     `forge models` retries on demand"
                );
                return;
            }
        };
        if attempt == 0 {
            tracing::warn!(
                "model discovery: keyed provider(s) unreachable at startup: {summary} — \
                 retrying in {}s, routing from the cached catalog meanwhile",
                delay.as_secs()
            );
        } else {
            tracing::info!(
                "model discovery retry {attempt}: still unreachable: {summary} — next in {}s",
                delay.as_secs()
            );
        }
        attempt += 1;
        tokio::time::sleep(delay).await;
    }
}

/// Models to re-add for providers that failed this round, and the updated "first carried at"
/// clock per provider.
///
/// A catalog built while a provider is down would otherwise lack all of that provider's models, and
/// every consumer (router, `/api/models`, the next startup's cache) would lose them until a later
/// refresh happened to succeed. Successful providers keep their fresh answer, so a model a healthy
/// provider retired still disappears; only the providers we could not ask are carried, and only for
/// [`CARRY_MAX_AGE_SECS`] from the first round they failed.
pub(super) fn plan_carry(
    fresh: &[String],
    cached: &[String],
    failed: &[&str],
    since: &BTreeMap<String, u64>,
    now: u64,
) -> (Vec<String>, BTreeMap<String, u64>) {
    let mut clock = BTreeMap::new();
    let mut carried: HashSet<&str> = HashSet::new();
    for provider in failed {
        let first = since.get(*provider).copied().unwrap_or(now);
        clock.insert((*provider).to_string(), first);
        if now.saturating_sub(first) <= CARRY_MAX_AGE_SECS {
            carried.insert(provider);
        }
    }
    let mut seen: HashSet<&str> = fresh.iter().map(String::as_str).collect();
    let added = cached
        .iter()
        .filter(|m| carried.contains(forge_config::provider_of(m)))
        .filter(|m| seen.insert(m.as_str()))
        .cloned()
        .collect();
    (added, clock)
}

fn carry_clock_path() -> Option<std::path::PathBuf> {
    forge_config::data_dir().map(|d| d.join("catalog-carry.json"))
}

/// Re-add the previous catalog's models for keyed providers that could not be listed this round,
/// and note it on their status so `forge models` shows why they look cached.
pub(super) fn carry_forward_unreachable(
    models: &mut Vec<String>,
    statuses: &mut [ProviderDiscoveryStatus],
    disabled: &[String],
) {
    let failed: Vec<String> = keyed_discovery_failures(statuses, forge_config::has_api_key)
        .into_iter()
        .map(|s| s.provider.clone())
        .collect();
    let clock_path = carry_clock_path();
    let since: BTreeMap<String, u64> = clock_path
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let cached = super::load_cached_catalog_aged()
        .map(|(catalog, _, _)| catalog.models().to_vec())
        .unwrap_or_default();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let failed_refs: Vec<&str> = failed.iter().map(String::as_str).collect();
    let (mut added, clock) = plan_carry(models, &cached, &failed_refs, &since, now);
    added.retain(|m| {
        !forge_config::is_non_chat_model(m) && !forge_config::is_model_disabled(m, disabled)
    });
    if let Some(path) = clock_path {
        if clock != since {
            if let Ok(json) = serde_json::to_vec(&clock) {
                let _ = std::fs::write(path, json);
            }
        }
    }
    for status in statuses.iter_mut() {
        let kept = added
            .iter()
            .filter(|m| forge_config::provider_of(m) == status.provider)
            .count();
        if kept > 0 {
            let note = format!("keeping {kept} model(s) from the last successful listing");
            status.detail = Some(match status.detail.take() {
                Some(detail) => format!("{detail}; {note}"),
                None => note,
            });
        }
    }
    models.extend(added);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn status(provider: &str, kind: DiscoveryStatusKind) -> ProviderDiscoveryStatus {
        ProviderDiscoveryStatus {
            provider: provider.into(),
            kind,
            models: 0,
            detail: Some("dns error".into()),
        }
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn cli_bridges_and_ollama_are_never_keyed_failures() {
        let statuses = [
            status("agy-cli", DiscoveryStatusKind::Failed),
            status("claude-cli", DiscoveryStatusKind::TimedOut),
            status("ollama", DiscoveryStatusKind::Failed),
            status("groq", DiscoveryStatusKind::Failed),
            status("gemini", DiscoveryStatusKind::Discovered),
        ];
        let failing = keyed_discovery_failures(&statuses, |_| true);
        let names: Vec<&str> = failing.iter().map(|s| s.provider.as_str()).collect();
        assert_eq!(names, ["groq"]);
    }

    #[test]
    fn a_provider_without_a_key_is_not_a_failure() {
        let statuses = [status("groq", DiscoveryStatusKind::Failed)];
        assert!(keyed_discovery_failures(&statuses, |_| false).is_empty());
    }

    fn answer(failing: bool) -> (ModelCatalog, Vec<ProviderDiscoveryStatus>) {
        let kind = if failing {
            DiscoveryStatusKind::Failed
        } else {
            DiscoveryStatusKind::Discovered
        };
        (ModelCatalog::default(), vec![status("groq", kind)])
    }

    #[tokio::test(start_paused = true)]
    async fn a_permanently_failing_provider_stops_the_loop() {
        let calls = AtomicUsize::new(0);
        discover_until_keyed_providers_answer(
            &DISCOVERY_RETRY_BACKOFF,
            DISCOVERY_RETRY_STEADY,
            DISCOVERY_RETRY_STEADY_ROUNDS,
            |_| true,
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(answer(true))
            },
            |_, _| {},
        )
        .await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1 + DISCOVERY_RETRY_BACKOFF.len() + DISCOVERY_RETRY_STEADY_ROUNDS
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_bridge_alone_never_starts_a_retry_loop() {
        let calls = AtomicUsize::new(0);
        discover_until_keyed_providers_answer(
            &DISCOVERY_RETRY_BACKOFF,
            DISCOVERY_RETRY_STEADY,
            DISCOVERY_RETRY_STEADY_ROUNDS,
            |_| true,
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready((
                    ModelCatalog::default(),
                    vec![status("agy-cli", DiscoveryStatusKind::Failed)],
                ))
            },
            |_, _| {},
        )
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failed_providers_keep_their_previous_models_and_healthy_ones_do_not() {
        let fresh = ids(&["openai::gpt-new", "groq::kept-live"]);
        let cached = ids(&[
            "openai::gpt-old-retired",
            "groq::kept-live",
            "groq::only-cached",
            "gemini::flash",
        ]);
        let (added, clock) = plan_carry(&fresh, &cached, &["groq"], &BTreeMap::new(), 1_000);
        assert_eq!(added, ["groq::only-cached"]);
        assert_eq!(clock.get("groq"), Some(&1_000));
        assert!(!clock.contains_key("openai"));
    }

    #[test]
    fn carrying_stops_after_the_age_cap_and_stays_stopped() {
        let cached = ids(&["groq::old"]);
        let since = BTreeMap::from([("groq".to_string(), 1_000u64)]);
        let (added, clock) =
            plan_carry(&[], &cached, &["groq"], &since, 1_000 + CARRY_MAX_AGE_SECS);
        assert_eq!(added, ["groq::old"]);
        let later = 1_001 + CARRY_MAX_AGE_SECS;
        let (added, clock) = plan_carry(&[], &cached, &["groq"], &clock, later);
        assert!(added.is_empty());
        assert_eq!(
            clock.get("groq"),
            Some(&1_000),
            "the clock must not restart"
        );
    }

    #[test]
    fn a_recovered_provider_clears_its_clock() {
        let since = BTreeMap::from([("groq".to_string(), 1_000u64)]);
        let (added, clock) = plan_carry(&ids(&["groq::a"]), &ids(&["groq::b"]), &[], &since, 2_000);
        assert!(added.is_empty());
        assert!(clock.is_empty());
    }
}
