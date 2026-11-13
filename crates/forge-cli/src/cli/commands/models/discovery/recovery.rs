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

/// A model the previous catalog held is dropped only after it has been absent from this many
/// consecutive successful listings of its provider...
pub(super) const MISS_LISTINGS_BEFORE_REMOVAL: u32 = 5;
/// ...and for at least this long since its first absence. Rediscovery runs hourly in steady state
/// but also in bursts (config edits, retry rounds), so the count alone could be spent in minutes.
pub(super) const MISS_WINDOW_SECS: u64 = 6 * 60 * 60;

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

/// [`keyed_discovery_failures`] minus providers that REFUSED their key. Those are settled facts
/// with a persisted verdict, not boot races: retrying them every few seconds and warning each
/// round is what produced 11,138 identical warnings in two weeks.
fn keyed_retryable_failures(
    statuses: &[ProviderDiscoveryStatus],
    has_key: impl Fn(&str) -> bool,
) -> Vec<&ProviderDiscoveryStatus> {
    keyed_discovery_failures(statuses, has_key)
        .into_iter()
        .filter(|s| super::verdict::rejection(s).is_none())
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
    let failing = keyed_retryable_failures(statuses, has_key);
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
        let failing = keyed_retryable_failures(&statuses, &has_key);
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

/// Per-model absence record: consecutive listings that omitted it, and when the first one was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct Miss {
    pub listings: u32,
    pub first_epoch: u64,
}

/// Hysteresis for providers that DID answer. A listing endpoint intermittently omitting a few
/// models (load-balanced backends, partial pages, rotating aliases) used to publish a "new"
/// catalog each time, so the picker, routing and every session's transcript flapped. A model in
/// the previous catalog that this listing omitted is kept until it has been missing for
/// [`MISS_LISTINGS_BEFORE_REMOVAL`] consecutive listings spanning [`MISS_WINDOW_SECS`]. A model
/// that reappears resets its record. Providers that returned nothing, or that failed (handled by
/// [`plan_carry`]), are never judged here: only a provider that listed something can say a
/// sibling model is gone. A model that is really dead is benched by the health path on its first
/// failed call, independent of whether the catalog still lists it.
pub(super) fn plan_hysteresis(
    fresh: &[String],
    cached: &[String],
    failed: &[&str],
    misses: &BTreeMap<String, Miss>,
    now: u64,
) -> (Vec<String>, BTreeMap<String, Miss>) {
    let answered: HashSet<&str> = fresh
        .iter()
        .map(|m| forge_config::provider_of(m))
        .filter(|p| !failed.contains(p))
        .collect();
    let present: HashSet<&str> = fresh.iter().map(String::as_str).collect();
    let mut next = BTreeMap::new();
    let mut kept = Vec::new();
    for model in cached {
        if present.contains(model.as_str()) || !answered.contains(forge_config::provider_of(model))
        {
            continue;
        }
        let prev = misses.get(model).copied().unwrap_or(Miss {
            listings: 0,
            first_epoch: now,
        });
        let miss = Miss {
            listings: prev.listings.saturating_add(1),
            first_epoch: prev.first_epoch,
        };
        let gone = miss.listings >= MISS_LISTINGS_BEFORE_REMOVAL
            && now.saturating_sub(miss.first_epoch) >= MISS_WINDOW_SECS;
        if !gone {
            next.insert(model.clone(), miss);
            kept.push(model.clone());
        }
    }
    // Records for providers we could not judge this round carry over untouched.
    for (model, miss) in misses {
        if !answered.contains(forge_config::provider_of(model))
            && cached.contains(model)
            && !present.contains(model.as_str())
        {
            next.insert(model.clone(), *miss);
        }
    }
    (kept, next)
}

fn miss_path() -> Option<std::path::PathBuf> {
    forge_config::data_dir().map(|d| d.join("catalog-misses.json"))
}

/// Keep models a healthy provider's listing merely omitted this round (see [`plan_hysteresis`]).
pub(super) fn retain_flapping_models(
    models: &mut Vec<String>,
    statuses: &[ProviderDiscoveryStatus],
    disabled: &[String],
) {
    let failed: Vec<&str> = statuses
        .iter()
        .filter(|s| !matches!(s.kind, DiscoveryStatusKind::Discovered))
        .map(|s| s.provider.as_str())
        .collect();
    let path = miss_path();
    let misses: BTreeMap<String, Miss> = path
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
    let (mut kept, next) = plan_hysteresis(models, &cached, &failed, &misses, now);
    kept.retain(|m| {
        !forge_config::is_non_chat_model(m) && !forge_config::is_model_disabled(m, disabled)
    });
    if let Some(path) = path {
        if next != misses {
            if let Ok(json) = serde_json::to_vec(&next) {
                let _ = std::fs::write(path, json);
            }
        }
    }
    models.extend(kept);
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

    /// One discovery round as `discover_catalog_with_status` sees it: fresh ids in, the catalog that
    /// would be published (and the miss records to persist) out.
    fn round(
        fresh: &[&str],
        previous: &[String],
        misses: &BTreeMap<String, Miss>,
        now: u64,
    ) -> (Vec<String>, BTreeMap<String, Miss>) {
        let mut models = ids(fresh);
        let (kept, next) = plan_hysteresis(&models, previous, &[], misses, now);
        models.extend(kept);
        (models, next)
    }

    fn sorted(mut v: Vec<String>) -> Vec<String> {
        v.sort();
        v
    }

    #[test]
    fn a_listing_that_flaps_never_changes_the_published_catalog() {
        let full = ["sambanova::a", "sambanova::b", "inferhub::c", "inferhub::d"];
        let mut published = ids(&full);
        let mut misses = BTreeMap::new();
        let mut now = 1_000_000;
        let mut changes = 0;
        for i in 0..100 {
            now += 60;
            // Every other round the provider omits two of its models.
            let fresh: &[&str] = if i % 2 == 0 {
                &["sambanova::a", "inferhub::c"]
            } else {
                &full
            };
            let (next, m) = round(fresh, &published, &misses, now);
            if sorted(next.clone()) != sorted(published.clone()) {
                changes += 1;
            }
            published = next;
            misses = m;
        }
        assert_eq!(changes, 0);
        assert_eq!(sorted(published), sorted(ids(&full)));
    }

    #[test]
    fn a_model_really_gone_disappears_after_the_listings_and_the_window() {
        let mut published = ids(&["groq::a", "groq::b"]);
        let mut misses = BTreeMap::new();
        let start = 1_000_000;
        // Enough listings, but too fast: still kept.
        for i in 0..(MISS_LISTINGS_BEFORE_REMOVAL * 3) {
            let (next, m) = round(&["groq::a"], &published, &misses, start + u64::from(i));
            published = next;
            misses = m;
        }
        assert!(published.contains(&"groq::b".to_string()));
        // Past the window, the next listing removes it and clears its record.
        let (next, m) = round(&["groq::a"], &published, &misses, start + MISS_WINDOW_SECS);
        assert_eq!(next, ids(&["groq::a"]));
        assert!(m.is_empty());
    }

    #[test]
    fn a_model_that_reappears_restarts_its_count() {
        let published = ids(&["groq::a", "groq::b"]);
        let (_, m) = round(&["groq::a"], &published, &BTreeMap::new(), 100);
        assert_eq!(m["groq::b"].listings, 1);
        let (_, m) = round(&["groq::a", "groq::b"], &published, &m, 200);
        assert!(m.is_empty());
    }

    #[test]
    fn failed_or_empty_providers_are_never_judged() {
        let cached = ids(&["groq::a", "groq::b", "mistral::x"]);
        // groq failed (the carry logic owns it); mistral listed nothing at all.
        let (kept, m) = plan_hysteresis(
            &ids(&["gemini::g"]),
            &cached,
            &["groq"],
            &BTreeMap::new(),
            5,
        );
        assert!(kept.is_empty());
        assert!(m.is_empty());
    }

    #[test]
    fn records_of_unjudged_providers_survive_a_round() {
        let cached = ids(&["groq::a", "mistral::x"]);
        let mut misses = BTreeMap::new();
        misses.insert(
            "mistral::x".to_string(),
            Miss {
                listings: 3,
                first_epoch: 7,
            },
        );
        let (_, m) = plan_hysteresis(&ids(&["groq::a"]), &cached, &[], &misses, 99);
        assert_eq!(m.get("mistral::x").map(|x| x.listings), Some(3));
    }

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

    #[test]
    fn a_provider_that_refused_its_key_is_neither_retried_nor_warned_about() {
        let statuses = vec![
            ProviderDiscoveryStatus {
                detail: Some(
                    r#"provider auth failed: 401 {"error":"This API key has expired"}"#.into(),
                ),
                ..status("opencode", DiscoveryStatusKind::Failed)
            },
            status("groq", DiscoveryStatusKind::TimedOut),
        ];
        let retry = keyed_retryable_failures(&statuses, |_| true);
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].provider, "groq");
        assert_eq!(keyed_discovery_failures(&statuses, |_| true).len(), 2);
    }
}
