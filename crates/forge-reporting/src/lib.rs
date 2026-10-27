//! Automatic, scrubbed, content-free issue reports for Forge's own bugs.
//!
//! Capture (panic hook, classified internal errors) -> classify (bug vs expected condition) ->
//! scrub (no prompts, replies, code, paths, secrets, hosts) -> fingerprint -> queue on disk ->
//! send in the background to the relay, which opens one GitHub issue per distinct problem.
//!
//! Hosts call [`init`] once at startup and [`install_panic_hook`]; everything else is optional.
//! The onboarding step and Settings toggle use [`mode`] / [`set_mode`] with [`ReportingMode`].
//! See `docs/features/issue-reporting.md`.

mod classify;
mod context;
mod frames;
mod panic;
mod report;
mod ring;
mod scrub;
mod send;
mod store;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub use classify::{classify, provider_request_is_bug, Class, Expected};
pub use context::{add_flag, set_session_context};
pub use forge_config::reporting::ReportingMode;
pub use panic::install_panic_hook;
pub use report::{
    fingerprint, Context, ErrorKind, Report, Source, Surface, MANUAL_FINGERPRINT, WIRE_VERSION,
};
pub use ring::{event, EventEntry};
pub use scrub::{scrub, scrub_message, set_redactor, strip_content, truncate};
pub use send::{endpoint, FlushStats, DEFAULT_ENDPOINT};
pub use store::{SentEntry, Store};

const MODE_TTL: Duration = Duration::from_secs(30);
const MAX_PER_PROCESS: u64 = 5;
const SEEN_WINDOW: Duration = Duration::from_secs(600);

/// What the host knows about itself.
#[derive(Debug, Clone)]
pub struct HostInfo {
    pub surface: Surface,
    pub version: String,
    pub git_sha: Option<String>,
    pub install_method: Option<String>,
}

pub(crate) static HOST: OnceLock<HostInfo> = OnceLock::new();
static MODE: Mutex<Option<(Instant, ReportingMode)>> = Mutex::new(None);
static GUARDS: Guards = Guards::new();
static REPORTED_SEQ: AtomicU64 = AtomicU64::new(0);
static PENDING_SEQ: AtomicU64 = AtomicU64::new(0);

pub(crate) fn truthy(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| {
        !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no" | "off"
        )
    })
}

/// `FORGE_NO_REPORTS=1` forces reporting off, whatever the config says.
pub fn forced_off() -> bool {
    truthy("FORGE_NO_REPORTS")
}

/// Debug builds (cargo test, local dev) never reach the public relay unless an endpoint is named
/// explicitly, so development noise cannot become issues.
fn build_allows_sending() -> bool {
    !cfg!(debug_assertions)
        || truthy("FORGE_REPORTING_FORCE")
        || std::env::var("FORGE_REPORTING_URL").is_ok_and(|u| !u.trim().is_empty())
}

fn load_mode() -> ReportingMode {
    forge_config::load()
        .map(|c| c.reporting.mode)
        .unwrap_or_default()
}

/// The effective mode: `Off` when `FORGE_NO_REPORTS` is set, else `[reporting] mode` (default
/// `auto`). Re-read from config at most every 30 seconds.
pub fn mode() -> ReportingMode {
    if forced_off() {
        return ReportingMode::Off;
    }
    if let Ok(mut slot) = MODE.lock() {
        if let Some((at, mode)) = *slot {
            if at.elapsed() < MODE_TTL {
                return mode;
            }
        }
        let fresh = load_mode();
        *slot = Some((Instant::now(), fresh));
        return fresh;
    }
    ReportingMode::Auto
}

/// The mode without touching disk: the last value [`mode`] saw. For the panic hook.
pub(crate) fn mode_cached() -> ReportingMode {
    if forced_off() {
        return ReportingMode::Off;
    }
    MODE.try_lock()
        .ok()
        .and_then(|slot| slot.map(|(_, m)| m))
        .unwrap_or_default()
}

/// Persist the mode in the user config. Used by Settings, `/config` and the onboarding step.
pub fn set_mode(mode: ReportingMode) -> Result<(), forge_config::ConfigError> {
    forge_config::reporting::write_reporting_mode(mode)?;
    if let Ok(mut slot) = MODE.lock() {
        *slot = Some((Instant::now(), mode));
    }
    Ok(())
}

/// Whether anything would be collected right now.
pub fn is_active() -> bool {
    mode() != ReportingMode::Off
}

pub fn store() -> Option<Store> {
    Store::default_location()
}

/// Call once at startup. Records who is running, warms the mode cache and, in `auto`, delivers
/// anything left in the queue by an offline or crashed earlier run.
pub fn init(host: HostInfo) {
    let _ = HOST.set(host);
    let mode = mode();
    if mode == ReportingMode::Off {
        // Off means nothing lingers: drop whatever an earlier run left waiting.
        if let Some(store) = store() {
            store.clear_waiting();
        }
        return;
    }
    if mode == ReportingMode::Auto && build_allows_sending() {
        if let Some(store) = store() {
            if !store.queued().is_empty() && !store.in_backoff() {
                send::spawn_flush(store, endpoint());
            }
        }
    }
}

/// A problem Forge noticed in itself.
#[derive(Debug, Clone)]
pub struct Incident {
    pub kind: ErrorKind,
    /// Stable logical site, e.g. `forge_core::turn` (not a line number).
    pub site: String,
    pub message: String,
    pub frames: Vec<String>,
    pub location: Option<String>,
    /// Run the text rules in [`classify`] before reporting. False for errors whose type already
    /// says they are bugs.
    pub check_text: bool,
}

impl Incident {
    /// An error whose type already established it is a bug.
    pub fn bug(kind: ErrorKind, site: &str, message: impl std::fmt::Display) -> Self {
        Incident {
            kind,
            site: site.to_string(),
            message: message.to_string(),
            frames: Vec::new(),
            location: None,
            check_text: false,
        }
    }

    /// An error that might be an expected condition: classify its text first.
    pub fn gray(kind: ErrorKind, site: &str, message: impl std::fmt::Display) -> Self {
        Incident {
            check_text: true,
            ..Self::bug(kind, site, message)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Mode is off (or `FORGE_NO_REPORTS`).
    Off,
    /// An expected condition, not a bug.
    NotABug(Expected),
    /// Already reported recently, or the per-run cap was reached.
    Duplicate,
    /// Queued for background delivery (auto mode).
    Queued,
    /// Waiting for the user's decision (ask mode).
    Held,
}

pub fn build_report(incident: &Incident) -> Report {
    let site = report::slug(&incident.site, 120, ":_.<>/-");
    let message = scrub::scrub_message(&incident.message, report::MAX_MESSAGE);
    let location = incident
        .location
        .as_deref()
        .map(|l| scrub::truncate(&scrub::scrub(&scrub::normalize_location(l)), 160))
        .unwrap_or_default();
    let frames: Vec<String> = incident.frames.iter().take(12).cloned().collect();
    let print = fingerprint(incident.kind, &site, &frames, &incident.message);
    Report {
        v: WIRE_VERSION,
        source: Source::Auto,
        install_id: store()
            .map(|s| s.install_id())
            .unwrap_or_else(|| "0".repeat(32)),
        fingerprint: print,
        error_kind: incident.kind,
        site,
        location,
        message,
        frames,
        context: context::context(),
        events: ring::snapshot(),
        user_note: None,
        last_error: None,
        occurred_at: store::now_secs(),
    }
}

/// Per-process flood protection: each problem is reported once per 10 minutes and at most
/// [`MAX_PER_PROCESS`] reports leave one run.
struct Guards {
    seen: Mutex<Vec<(String, Instant)>>,
    sent: AtomicU64,
}

impl Guards {
    const fn new() -> Self {
        Guards {
            seen: Mutex::new(Vec::new()),
            sent: AtomicU64::new(0),
        }
    }

    fn seen_recently(&self, fingerprint: &str) -> bool {
        let Ok(mut seen) = self.seen.lock() else {
            return false;
        };
        seen.retain(|(_, at)| at.elapsed() < SEEN_WINDOW);
        if seen.iter().any(|(f, _)| f == fingerprint) {
            return true;
        }
        seen.push((fingerprint.to_string(), Instant::now()));
        false
    }
}

/// Report an incident according to the mode. Never blocks on the network and never panics.
pub fn report(incident: Incident) -> Disposition {
    let mode = mode();
    report_in(
        incident,
        mode,
        store().as_ref(),
        build_allows_sending(),
        &GUARDS,
    )
}

fn report_in(
    incident: Incident,
    mode: ReportingMode,
    store: Option<&Store>,
    may_send: bool,
    guards: &Guards,
) -> Disposition {
    if mode == ReportingMode::Off || !may_send {
        return Disposition::Off;
    }
    let class = if incident.check_text || incident.kind == ErrorKind::Panic {
        classify(incident.kind, &incident.message)
    } else {
        Class::Bug
    };
    if let Class::Expected(why) = class {
        return Disposition::NotABug(why);
    }
    let Some(store) = store else {
        return Disposition::Off;
    };
    if guards.sent.load(Ordering::Relaxed) >= MAX_PER_PROCESS {
        return Disposition::Duplicate;
    }
    let report = build_report(&incident);
    store.write_last(&report);
    if guards.seen_recently(&report.fingerprint) || store.recently_sent(&report.fingerprint) {
        return Disposition::Duplicate;
    }
    guards.sent.fetch_add(1, Ordering::Relaxed);
    match mode {
        ReportingMode::Ask => {
            if store.hold_pending(&report).is_ok() {
                PENDING_SEQ.fetch_add(1, Ordering::Relaxed);
            }
            Disposition::Held
        }
        _ => {
            if store.enqueue(&report).is_ok() {
                REPORTED_SEQ.fetch_add(1, Ordering::Relaxed);
                send::spawn_flush(store.clone(), endpoint());
            }
            Disposition::Queued
        }
    }
}

/// Bumped each time an auto-mode report is queued; a UI shows its subtle "reported" mark when the
/// value changes.
pub fn reported_seq() -> u64 {
    REPORTED_SEQ.load(Ordering::Relaxed)
}

/// Bumped each time an ask-mode report is held; a UI re-reads [`pending`] when it changes.
pub fn pending_seq() -> u64 {
    PENDING_SEQ.load(Ordering::Relaxed)
}

#[derive(Debug, Clone)]
pub struct Pending {
    pub id: String,
    pub report: Report,
}

/// Reports waiting for an ask-mode decision, oldest first (includes ones held by a crashed run).
pub fn pending() -> Vec<Pending> {
    store()
        .map(|s| {
            s.pending()
                .into_iter()
                .map(|(id, report)| Pending { id, report })
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Send,
    Skip,
    /// Send this one and switch the mode to `auto` for the future.
    AlwaysSend,
}

/// Apply the user's choice on an ask-mode card.
pub fn resolve(id: &str, decision: Decision) {
    let Some(store) = store() else { return };
    let Some(report) = store.take_pending(id) else {
        return;
    };
    if decision == Decision::Skip || forced_off() {
        return;
    }
    if decision == Decision::AlwaysSend {
        let _ = set_mode(ReportingMode::Auto);
    }
    if store.enqueue(&report).is_ok() && build_allows_sending() {
        REPORTED_SEQ.fetch_add(1, Ordering::Relaxed);
        send::spawn_flush(store, endpoint());
    }
}

/// The most recently built report (sent or not), for `forge report --show-last`.
pub fn last_report() -> Option<Report> {
    store().and_then(|s| s.last())
}

/// What has been sent, oldest first.
pub fn sent_log() -> Vec<SentEntry> {
    store().map(|s| s.sent()).unwrap_or_default()
}

/// A manual `/bug` report: the same scrubbed context plus the user's own words. `last_error` is
/// only attached when the user has seen the preview and agreed.
pub fn build_manual(note: &str, last_error: Option<&str>) -> Report {
    let mut report = build_report(&Incident::bug(ErrorKind::Manual, "manual", ""));
    report.source = Source::Manual;
    report.fingerprint = MANUAL_FINGERPRINT.to_string();
    report.location = String::new();
    report.message = String::new();
    report.frames = Vec::new();
    report.user_note = Some(scrub::truncate(&scrub::scrub(note), report::MAX_NOTE));
    report.last_error = last_error
        .filter(|e| !e.trim().is_empty())
        .map(|e| scrub::truncate(&scrub::scrub(e), report::MAX_LAST_ERROR));
    report
}

/// Park a manual report until the user confirms it on a preview card. Returns the id to pass to
/// [`resolve`].
pub fn hold_manual(report: &Report) -> Option<String> {
    let path = store()?.hold_pending(report).ok()?;
    Some(path.file_stem()?.to_string_lossy().to_string())
}

/// Queue a user-confirmed manual report. Works in any mode (the user just asked for it) but not
/// when `FORGE_NO_REPORTS` is set.
pub fn submit_manual(report: Report) -> Result<(), String> {
    if forced_off() {
        return Err("reporting is disabled by FORGE_NO_REPORTS".to_string());
    }
    let store = store().ok_or("no data directory")?;
    store.write_last(&report);
    store.enqueue(&report).map_err(|e| e.to_string())?;
    if build_allows_sending() {
        send::spawn_flush(store, endpoint());
    }
    Ok(())
}

/// Deliver the queue now (blocking, bounded). For a CLI that is about to exit.
pub fn flush_blocking(budget: Duration) -> FlushStats {
    match (store(), build_allows_sending(), forced_off()) {
        (Some(store), true, false) => send::flush_on_thread(&store, &endpoint(), budget),
        _ => FlushStats::default(),
    }
}

#[cfg(test)]
mod tests;
