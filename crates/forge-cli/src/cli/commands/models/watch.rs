//! Keeping a long-running process's model catalog current without a restart.
//!
//! Sessions load their catalog and provider set once. A TUI left open (or a daemon-hosted session)
//! therefore never saw a `[[providers.custom]]` block added to config.toml, a key stored with
//! `forge auth`, or a model a provider started listing. One background task per process fixes
//! that:
//!
//! * every [`CHECK_EVERY`] it `stat`s four files (user config, project config, the secret-change
//!   stamp, the catalog cache) and compares them with the previous look. That is the whole steady
//!   state cost: no network, no keyring, no parsing.
//! * config or key change: reload the provider registries, re-inject keys, rediscover in the
//!   background (single-flight with the startup refresh) and publish the result.
//! * the catalog cache moved without us (another process rediscovered): adopt it, no network.
//! * nothing moved but the cache is an hour old: rediscover, so models a provider started listing
//!   show up on the same cadence the context-window sweep uses.
//!
//! Publishing goes to [`live_catalog`], which every router already follows; sessions attach with
//! [`follow_catalog`] and swap their catalog snapshot at the next turn boundary (the swap takes the
//! session lock, which a running turn holds), never touching the pin.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use forge_core::Session;
use forge_mesh::ModelCatalog;

use super::discovery::{
    catalog_cache_path, discover_catalog_with_status, end_catalog_refresh,
    load_cached_catalog_aged, publish_live, refresh_loop, save_catalog, startup_refresh_due,
    try_begin_catalog_refresh, write_catalog,
};

/// Interval between change checks.
const CHECK_EVERY: Duration = Duration::from_secs(30);

/// Minimum spacing between periodic (nothing-changed) full rediscoveries; mirrors the startup
/// refresh window and the context-window sweep gate.
const PERIODIC_EVERY: Duration = Duration::from_secs(60 * 60);

/// Everything the watcher looks at, as modification times. `None` = file absent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Signals {
    pub config: Option<SystemTime>,
    pub project_config: Option<SystemTime>,
    pub secrets: Option<SystemTime>,
    pub catalog: Option<SystemTime>,
}

impl Signals {
    /// Four `stat` calls.
    pub(crate) fn read() -> Self {
        let mtime = |path: Option<std::path::PathBuf>| {
            path.and_then(|p| std::fs::metadata(p).ok())
                .and_then(|m| m.modified().ok())
        };
        Self {
            config: mtime(forge_config::config_dir().map(|d| d.join("config.toml"))),
            project_config: mtime(Some(std::path::PathBuf::from("./.forge/config.toml"))),
            secrets: forge_config::secret_store::change_stamp(),
            catalog: mtime(catalog_cache_path()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reason {
    Config,
    Keys,
    Periodic,
    Requested,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Idle,
    AdoptCache,
    Rediscover(Reason),
}

/// The decision logic, free of IO and clocks so it can be tested exhaustively.
pub(crate) struct Planner {
    last: Signals,
    last_rediscovery: Instant,
}

impl Planner {
    pub(crate) fn new(now: Signals, at: Instant) -> Self {
        Self {
            last: now,
            last_rediscovery: at,
        }
    }

    /// What to do given the current `now` signals. Config and keys outrank an adoptable cache (the
    /// cache may predate the change); an adoptable cache outranks the hourly sweep.
    pub(crate) fn plan(&self, now: &Signals, at: Instant, cache_age_secs: Option<u64>) -> Step {
        if now.config != self.last.config || now.project_config != self.last.project_config {
            return Step::Rediscover(Reason::Config);
        }
        if now.secrets != self.last.secrets {
            return Step::Rediscover(Reason::Keys);
        }
        if now.catalog != self.last.catalog {
            return Step::AdoptCache;
        }
        if at.duration_since(self.last_rediscovery) >= PERIODIC_EVERY
            && startup_refresh_due(cache_age_secs)
        {
            return Step::Rediscover(Reason::Periodic);
        }
        Step::Idle
    }

    /// A rediscovery was started for `now`: its inputs are consumed. The catalog signal is
    /// re-read after the refresh writes, so our own write is not mistaken for someone else's.
    pub(crate) fn rediscovery_started(&mut self, now: Signals, at: Instant) {
        self.last = now;
        self.last_rediscovery = at;
    }

    pub(crate) fn cache_adopted(&mut self, catalog: Option<SystemTime>) {
        self.last.catalog = catalog;
    }
}

fn publish_generation() -> &'static tokio::sync::watch::Sender<u64> {
    static GEN: OnceLock<tokio::sync::watch::Sender<u64>> = OnceLock::new();
    GEN.get_or_init(|| tokio::sync::watch::channel(0).0)
}

/// Wake every [`follow_catalog`] task: a new catalog was just published.
pub(super) fn announce_publish() {
    publish_generation().send_modify(|g| *g += 1);
}

/// Keep `session` on the published catalog for as long as it lives. The swap waits for the session
/// lock, so it can never land inside a running turn.
pub(crate) fn follow_catalog(session: &Arc<tokio::sync::Mutex<Session>>) {
    let weak = Arc::downgrade(session);
    let mut rx = publish_generation().subscribe();
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let Some(session) = weak.upgrade() else {
                return;
            };
            session.lock().await.sync_catalog_feed();
        }
    });
}

/// Start this process's watcher (idempotent).
pub(crate) fn spawn_catalog_watch() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        let mut planner = Planner::new(Signals::read(), Instant::now());
        let mut tick = tokio::time::interval(CHECK_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await;
        loop {
            tick.tick().await;
            let now = Signals::read();
            let cache_age = catalog_age_secs(&now);
            match planner.plan(&now, Instant::now(), cache_age) {
                Step::Idle => {}
                Step::AdoptCache => {
                    let ours = *OWN_CACHE_WRITE.lock().unwrap_or_else(|e| e.into_inner());
                    if ours == now.catalog || adopt_cache() {
                        planner.cache_adopted(now.catalog);
                    }
                }
                Step::Rediscover(reason) => {
                    // The startup refresh (or an earlier request) may still be running; look again
                    // next tick rather than queueing a second sweep behind it.
                    if start_refresh(reason) {
                        planner.rediscovery_started(now, Instant::now());
                    }
                }
            }
        }
    });
}

fn catalog_age_secs(signals: &Signals) -> Option<u64> {
    signals
        .catalog
        .map(|t| t.elapsed().unwrap_or_default().as_secs())
}

/// Publish the on-disk catalog another process wrote. No network.
fn adopt_cache() -> bool {
    match load_cached_catalog_aged() {
        Some((catalog, _, _)) => {
            publish_live(catalog);
            true
        }
        None => false,
    }
}

static LAST_PROVIDER_SET: Mutex<Option<String>> = Mutex::new(None);

/// Modification time of the catalog cache as WE last wrote it, so the watcher does not adopt back
/// what it just persisted.
static OWN_CACHE_WRITE: Mutex<Option<SystemTime>> = Mutex::new(None);

/// Begin a background refresh: reload provider config and keys, rediscover, persist and publish.
/// Supersedes a refresh still retrying a down provider. Always starts one. Returns immediately; discovery runs on its own task under the
/// per-provider budgets of [`discover_catalog_with_status`], and a keyed provider that is down is
/// retried with backoff, publishing each round, exactly like the startup refresh.
fn start_refresh(reason: Reason) -> bool {
    // A refresh that is still retrying a down provider (up to ~30 min) must not block adoption of
    // a newer config: take over its slot instead of declining. The superseded task stops without
    // releasing the slot, which the new task now owns.
    let _ = try_begin_catalog_refresh();
    let mut generation = supersede_running_refresh();
    tokio::spawn(async move {
        let provider_set_changed = tokio::task::spawn_blocking(reload_providers)
            .await
            .unwrap_or(false);
        let config = forge_config::load().unwrap_or_default();
        tracing::debug!(
            ?reason,
            provider_set_changed,
            "refreshing the model catalog"
        );
        let refresh = refresh_with(
            provider_set_changed,
            || discover_catalog_with_status(&config),
            |catalog, changed| {
                if changed {
                    write_catalog(&catalog);
                } else {
                    save_catalog(&catalog);
                }
                *OWN_CACHE_WRITE.lock().unwrap_or_else(|e| e.into_inner()) =
                    Signals::read().catalog;
                publish_live(catalog);
            },
        );
        tokio::select! {
            () = refresh => end_catalog_refresh(),
            _ = generation.changed() => {}
        }
    });
    true
}

/// Stop any running refresh and return a receiver that fires when THIS one is superseded in turn.
pub(super) fn supersede_running_refresh() -> tokio::sync::watch::Receiver<u64> {
    refresh_generation().send_modify(|g| *g += 1);
    refresh_generation().subscribe()
}

/// Bumped by every refresh start; a running refresh whose snapshot goes stale stops.
fn refresh_generation() -> &'static tokio::sync::watch::Sender<u64> {
    static GEN: std::sync::OnceLock<tokio::sync::watch::Sender<u64>> = std::sync::OnceLock::new();
    GEN.get_or_init(|| tokio::sync::watch::channel(0).0)
}

/// Ask for a refresh now (the Ctrl-R reload). Idempotent while one is running.
pub(crate) fn refresh_catalog_now() -> bool {
    start_refresh(Reason::Requested)
}

/// Re-read provider definitions and keys; whether the set of reachable providers differs from the
/// previous refresh. Blocking: the secret store may talk to the keyring.
pub(crate) fn reload_providers() -> bool {
    forge_config::reload_provider_registries();
    forge_config::inject_provider_keys();
    let fingerprint = forge_config::provider_set_fingerprint();
    let mut last = LAST_PROVIDER_SET.lock().unwrap_or_else(|e| e.into_inner());
    let changed = last.as_deref().is_some_and(|prev| prev != fingerprint);
    *last = Some(fingerprint);
    changed
}

/// The discover-and-publish loop, with discovery and publishing injected so it runs offline in
/// tests.
async fn refresh_with<D, Fut, P>(provider_set_changed: bool, discover: D, mut publish: P)
where
    D: FnMut() -> Fut,
    Fut: std::future::Future<
        Output = (ModelCatalog, Vec<super::discovery::ProviderDiscoveryStatus>),
    >,
    P: FnMut(ModelCatalog, bool),
{
    refresh_loop(forge_config::has_api_key, discover, |catalog, _late| {
        publish(catalog, provider_set_changed)
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(secs: u64) -> Option<SystemTime> {
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
    }

    fn signals() -> Signals {
        Signals {
            config: t(10),
            project_config: None,
            secrets: t(20),
            catalog: t(30),
        }
    }

    #[test]
    fn steady_state_is_idle() {
        let start = Instant::now();
        let planner = Planner::new(signals(), start);
        assert_eq!(planner.plan(&signals(), start, Some(5)), Step::Idle);
    }

    #[test]
    fn a_config_edit_or_new_project_config_rediscovers() {
        let start = Instant::now();
        let planner = Planner::new(signals(), start);
        let mut edited = signals();
        edited.config = t(99);
        assert_eq!(
            planner.plan(&edited, start, Some(5)),
            Step::Rediscover(Reason::Config)
        );
        let mut project = signals();
        project.project_config = t(1);
        assert_eq!(
            planner.plan(&project, start, Some(5)),
            Step::Rediscover(Reason::Config)
        );
    }

    #[test]
    fn a_stored_or_deleted_key_rediscovers() {
        let start = Instant::now();
        let planner = Planner::new(signals(), start);
        let mut keyed = signals();
        keyed.secrets = t(77);
        assert_eq!(
            planner.plan(&keyed, start, Some(5)),
            Step::Rediscover(Reason::Keys)
        );
    }

    #[test]
    fn a_catalog_written_by_another_process_is_adopted_not_rediscovered() {
        let start = Instant::now();
        let planner = Planner::new(signals(), start);
        let mut other = signals();
        other.catalog = t(500);
        assert_eq!(planner.plan(&other, start, Some(1)), Step::AdoptCache);
    }

    #[test]
    fn config_change_outranks_an_adoptable_cache() {
        let start = Instant::now();
        let planner = Planner::new(signals(), start);
        let mut both = signals();
        both.catalog = t(500);
        both.config = t(600);
        assert_eq!(
            planner.plan(&both, start, Some(1)),
            Step::Rediscover(Reason::Config)
        );
    }

    #[test]
    fn an_hour_old_cache_triggers_one_periodic_rediscovery_per_hour() {
        let start = Instant::now();
        let mut planner = Planner::new(signals(), start);
        let hour = start + PERIODIC_EVERY;
        assert_eq!(
            planner.plan(&signals(), hour, Some(3600)),
            Step::Rediscover(Reason::Periodic)
        );
        planner.rediscovery_started(signals(), hour);
        // The refresh may have been refused by the clobber guard, leaving the cache old: the
        // spacing, not the cache age alone, bounds the sweep rate.
        assert_eq!(
            planner.plan(&signals(), hour + CHECK_EVERY, Some(3700)),
            Step::Idle
        );
    }

    #[test]
    fn a_fresh_cache_is_not_rediscovered_even_after_an_hour_of_watching() {
        let start = Instant::now();
        let planner = Planner::new(signals(), start);
        assert_eq!(
            planner.plan(&signals(), start + PERIODIC_EVERY, Some(120)),
            Step::Idle
        );
    }

    #[test]
    fn our_own_cache_write_is_not_adopted_back() {
        let start = Instant::now();
        let mut planner = Planner::new(signals(), start);
        let mut after_refresh = signals();
        after_refresh.catalog = t(900);
        planner.rediscovery_started(signals(), start);
        planner.cache_adopted(after_refresh.catalog);
        assert_eq!(planner.plan(&after_refresh, start, Some(0)), Step::Idle);
    }

    #[tokio::test]
    async fn refresh_publishes_each_round_and_flags_a_provider_set_change() {
        let published = Arc::new(Mutex::new(Vec::new()));
        let sink = published.clone();
        refresh_with(
            true,
            || async {
                (
                    ModelCatalog::new(vec!["modelgateway::glm".into()]),
                    Vec::new(),
                )
            },
            move |catalog, changed| {
                sink.lock().unwrap().push((catalog.models().len(), changed));
            },
        )
        .await;
        assert_eq!(*published.lock().unwrap(), vec![(1, true)]);
    }

    /// The periodic check must stay negligible: four `stat`s.
    #[test]
    fn reading_the_signals_is_microseconds() {
        let start = Instant::now();
        for _ in 0..1000 {
            let _ = Signals::read();
        }
        let per_call = start.elapsed() / 1000;
        eprintln!("Signals::read: {per_call:?} per call");
        assert!(per_call < Duration::from_millis(1));
    }
}
