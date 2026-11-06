//! Spend envelopes for `/goal --budget` (the pure accounting is `forge_types::goal_envelope`).
//!
//! This is the part that touches the outside: reading the subscription window the envelope is
//! measured against (the store's quota rows, refreshed through the same claude/codex probes the
//! usage page uses), baselining a goal before its first turn, the pause message, and the record
//! that lets a paused goal survive a daemon restart.

use std::sync::atomic::{AtomicBool, Ordering};

use forge_types::goal_envelope::{
    EnvelopeCheck, EnvelopeKind, EnvelopeSpec, EnvelopeState, QuotaReading,
};

use super::*;

/// Set for `--mock` sessions: the offline provider must never trigger a real `claude` probe.
pub(crate) static MOCK_SESSION: AtomicBool = AtomicBool::new(false);

/// A reading older than this is refreshed before the envelope is checked.
const REFRESH_AFTER_SECS: i64 = 120;

/// Everything one envelope check reads, so a test can hand it a fake instead of a store.
pub(crate) struct EnvelopeSources {
    pub(crate) windows: Vec<forge_store::SubscriptionWindow>,
    pub(crate) session_usd: f64,
    pub(crate) now: i64,
}

impl EnvelopeSources {
    pub(crate) fn read(s: &Session) -> Self {
        Self::from_store(&s.store, s.store.session_cost(s.id()).unwrap_or(0.0))
    }

    /// The same reading without a session, for work that spans several (an issue drain).
    pub(crate) fn from_store(store: &forge_store::Store, session_usd: f64) -> Self {
        let now = chrono::Utc::now().timestamp();
        let mut windows = store.subscription_windows().unwrap_or_default();
        windows.extend(test_override(now));
        Self {
            windows,
            session_usd,
            now,
        }
    }

    /// The latest reading for the envelope's window, if the store holds one.
    pub(crate) fn quota(&self, spec: &EnvelopeSpec) -> Option<QuotaReading> {
        let EnvelopeKind::Quota { provider, window } = &spec.kind else {
            return None;
        };
        self.windows
            .iter()
            .filter(|w| family_matches(provider, &w.provider) && &w.window_kind == window)
            .filter_map(|w| {
                Some(QuotaReading {
                    pct: w.fraction? * 100.0,
                    observed_at: w.updated_at,
                    resets_at: w.resets_at,
                })
            })
            .max_by_key(|r| r.observed_at)
    }

    pub(crate) fn check(&self, envelope: &mut EnvelopeState) -> EnvelopeCheck {
        if envelope.is_usd() {
            envelope.observe_usd(self.session_usd)
        } else {
            let reading = self.quota(&envelope.spec);
            envelope.observe_quota(reading, self.now)
        }
    }
}

fn family_matches(family: &str, provider: &str) -> bool {
    match family {
        "claude" => provider == "claude-cli",
        "codex" => matches!(provider, "codex-cli" | "codex-oauth"),
        _ => false,
    }
}

/// TEST-ONLY (debug builds): `FORGE_TEST_GOAL_ENVELOPE_QUOTA=<file>` holds one line,
/// `<claude|codex> <weekly|five_hour> <percent> [resets_at_unix]`, read at every check, standing
/// in for a live subscription reading so the envelope can be driven without a real plan. Release
/// builds ignore it.
fn test_override(now: i64) -> Option<forge_store::SubscriptionWindow> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let path = std::env::var("FORGE_TEST_GOAL_ENVELOPE_QUOTA").ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    let mut words = text.split_whitespace();
    let provider = match words.next()? {
        "claude" => "claude-cli",
        "codex" => "codex-cli",
        _ => return None,
    };
    let window = words.next()?.to_string();
    let pct: f64 = words.next()?.parse().ok()?;
    let resets_at = words.next().and_then(|w| w.parse().ok());
    Some(forge_store::SubscriptionWindow {
        provider: provider.to_string(),
        window_kind: window,
        status: "ok".to_string(),
        resets_at,
        fraction: Some(pct / 100.0),
        updated_at: now + 1,
    })
}

fn test_override_active() -> bool {
    cfg!(debug_assertions) && std::env::var_os("FORGE_TEST_GOAL_ENVELOPE_QUOTA").is_some()
}

/// Ask the claude or codex probe for a current reading when the stored one is old. Never runs for
/// a mock session or under the test override.
pub(crate) async fn refresh_quota(session: &Arc<tokio::sync::Mutex<Session>>, spec: &EnvelopeSpec) {
    let EnvelopeKind::Quota { .. } = &spec.kind else {
        return;
    };
    if MOCK_SESSION.load(Ordering::Relaxed) || test_override_active() {
        return;
    }
    let store = session.lock().await.store.clone();
    refresh_quota_store(&store, spec).await;
}

/// [`refresh_quota`] for a caller that holds a store rather than a session.
pub(crate) async fn refresh_quota_store(store: &forge_store::Store, spec: &EnvelopeSpec) {
    let EnvelopeKind::Quota { provider, .. } = &spec.kind else {
        return;
    };
    if MOCK_SESSION.load(Ordering::Relaxed) || test_override_active() {
        return;
    }
    let sources = EnvelopeSources::from_store(store, 0.0);
    if sources
        .quota(spec)
        .is_some_and(|r| sources.now - r.observed_at <= REFRESH_AFTER_SECS)
    {
        return;
    }
    match provider.as_str() {
        "claude" => {
            crate::cli::commands::models::refresh_claude_quota_if_stale(store, REFRESH_AFTER_SECS)
                .await
        }
        _ => crate::cli::commands::models::refresh_codex_quota(store).await,
    }
}

/// The envelope for a goal about to start, baselined on the reading (or session cost) right now.
/// `Err` says why the envelope cannot be verified, in which case the goal does not start.
pub(crate) async fn baseline(
    session: &Arc<tokio::sync::Mutex<Session>>,
    spec: EnvelopeSpec,
) -> Result<EnvelopeState, String> {
    refresh_quota(session, &spec).await;
    let s = session.lock().await;
    baseline_from(&EnvelopeSources::read(&s), spec)
}

/// [`baseline`] for a drain: the envelope starts counting at the window's reading right now, and
/// a `$` envelope at zero (the drain sums its own sessions' cost).
pub(crate) async fn baseline_store(
    store: &forge_store::Store,
    spec: EnvelopeSpec,
    refresh: bool,
) -> Result<EnvelopeState, String> {
    if refresh {
        refresh_quota_store(store, &spec).await;
    }
    baseline_from(&EnvelopeSources::from_store(store, 0.0), spec)
}

fn baseline_from(sources: &EnvelopeSources, spec: EnvelopeSpec) -> Result<EnvelopeState, String> {
    if spec.kind == EnvelopeKind::Usd {
        return Ok(EnvelopeState::new(spec, sources.session_usd, None));
    }
    match sources.quota(&spec) {
        Some(r)
            if sources.now - r.observed_at
                <= forge_types::goal_envelope::ENVELOPE_MAX_READING_AGE_SECS =>
        {
            Ok(EnvelopeState::new(spec, r.pct, r.resets_at))
        }
        _ => Err(format!(
            "no fresh {} usage reading to measure the envelope against",
            spec.label()
        )),
    }
}

/// `Thu 09:00` in local time.
pub(crate) fn reset_label(resets_at: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(resets_at, 0)
        .single()
        .map_or_else(String::new, |t| t.format("%a %H:%M").to_string())
}

/// When a goal waiting on a window resumes, as the pause message and the surfaces say it: the
/// clock time alone within the next day, with the weekday beyond that.
pub(crate) fn resume_clock(resets_at: i64, now: i64) -> String {
    use chrono::TimeZone;
    let Some(t) = chrono::Local.timestamp_opt(resets_at, 0).single() else {
        return String::new();
    };
    let fmt = if (resets_at - now).abs() < 20 * 3600 {
        "%H:%M"
    } else {
        "%a %H:%M"
    };
    t.format(fmt).to_string()
}

/// What the goal says when it pauses at the cap, or cannot verify the envelope.
pub(crate) fn pause_message(envelope: &EnvelopeState, check: EnvelopeCheck) -> String {
    pause_message_at(envelope, check, chrono::Utc::now().timestamp())
}

pub(crate) fn pause_message_at(envelope: &EnvelopeState, check: EnvelopeCheck, now: i64) -> String {
    let raise = format!("/goal budget {}", envelope.raise_hint());
    if check == EnvelopeCheck::Unverified {
        return format!(
            "Envelope can't be verified: no fresh {} reading. Paused instead of spending blind — \
             /goal resume retries, or raise it with {raise}",
            envelope.spec.label()
        );
    }
    let phrase = envelope.slice_phrase();
    match (envelope.is_usd(), envelope.auto_resume_at()) {
        (false, Some(at)) => format!(
            "Envelope spent ({phrase}). The goal resumes at {} when the window resets — or raise \
             it with {raise} to continue now",
            resume_clock(at, now)
        ),
        _ => format!("Envelope spent ({phrase}). Raise it with {raise}, then /goal resume"),
    }
}

/// The note a goal leaves, in history and as a push, when it resumes at the window reset.
pub(crate) const AUTO_RESUME_NOTE: &str = "Window reset — goal resumed";

/// Record how a goal waiting for its window says when it resumes, so every surface can.
pub(crate) fn mark_waiting(envelope: &mut EnvelopeState, now: i64) {
    envelope.held = false;
    envelope.resume_label = envelope
        .auto_resume_at()
        .map(|at| resume_clock(at, now))
        .filter(|label| !label.is_empty());
}

/// True for a goal paused because its window envelope is spent, whose window has now reset.
/// A goal the user paused, a `$` envelope, an unknown reset time or a held goal is never due.
pub(crate) fn auto_resume_due(app: &forge_tui::App, now: i64) -> bool {
    use forge_tui::goal_status::GoalPhase;
    app.goal.as_ref().is_some_and(|g| {
        g.phase == GoalPhase::Paused
            && g.envelope
                .as_ref()
                .and_then(EnvelopeState::auto_resume_at)
                .is_some_and(|at| at <= now)
    })
}

/// Resume a goal whose window has reset, through the same path as `/goal resume`. `None` when no
/// goal is due; otherwise the outcome that starts it, after the note in history.
pub(crate) async fn auto_resume(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    now: i64,
) -> Option<super::dispatch::DispatchOutcome> {
    use super::dispatch::DispatchOutcome;
    if !auto_resume_due(app, now) {
        return None;
    }
    app.note(&format!("▶ {AUTO_RESUME_NOTE}"));
    match super::goal_control::goal_control_outcome(session, app, forge_tui::GoalControl::Resume)
        .await
    {
        Ok(outcome @ DispatchOutcome::StartGoal { .. }) => Some(outcome),
        _ => None,
    }
}

/// A paused envelope goal that is still spent, or can resume. A reset window starts the next run
/// from a fresh baseline.
pub(crate) fn resume_gate(envelope: &mut EnvelopeState, now: i64) -> Result<(), String> {
    if !envelope.spent {
        return Ok(());
    }
    if !envelope.is_usd() && envelope.resets_at.is_some_and(|at| at <= now) {
        envelope.rebaseline();
        return Ok(());
    }
    Err(format!(
        "envelope still spent ({}) — raise it with /goal budget {}{}",
        envelope.meter(),
        envelope.raise_hint(),
        envelope
            .resets_at
            .filter(|_| !envelope.is_usd())
            .map(|at| format!(", or wait for the reset at {}", reset_label(at)))
            .unwrap_or_default()
    ))
}

/// The record a restart restores a goal from.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct GoalRecord {
    pub(crate) objective: String,
    pub(crate) gates: Vec<String>,
    pub(crate) max_tokens: Option<u64>,
    pub(crate) max_minutes: Option<u64>,
    pub(crate) iteration: usize,
    pub(crate) tokens_used: u64,
    pub(crate) elapsed_secs: u64,
    pub(crate) reason: Option<String>,
    pub(crate) envelope: EnvelopeState,
}

impl GoalRecord {
    /// `None` unless the goal carries an envelope and is running or paused.
    pub(crate) fn of(app: &forge_tui::App) -> Option<String> {
        use forge_tui::goal_status::GoalPhase;
        let goal = app.goal.as_ref()?;
        if !matches!(goal.phase, GoalPhase::Active | GoalPhase::Paused) {
            return None;
        }
        let record = Self {
            objective: goal.objective.clone(),
            gates: goal.gates.clone(),
            max_tokens: goal.max_tokens,
            max_minutes: goal.max_minutes,
            iteration: goal.iteration,
            tokens_used: goal.tokens_used,
            elapsed_secs: goal.elapsed_secs,
            reason: goal.reason.clone(),
            envelope: goal.envelope.clone()?,
        };
        serde_json::to_string(&record).ok()
    }
}

/// Bring back the goal a restart interrupted, paused, with its envelope and counters.
pub(crate) fn restore(app: &mut forge_tui::App, store: &forge_store::Store, session_id: &str) {
    if app.goal.is_some() {
        return;
    }
    let Ok(Some(json)) = store.session_goal_state(session_id) else {
        return;
    };
    let Ok(record) = serde_json::from_str::<GoalRecord>(&json) else {
        return;
    };
    let reason = record
        .reason
        .or_else(|| Some("interrupted by a restart".to_string()));
    app.goal_restore(
        &record.objective,
        &record.gates,
        record.max_tokens,
        record.max_minutes,
        forge_tui::goal_status::GoalCarry {
            iteration: record.iteration,
            tokens_used: record.tokens_used,
            elapsed_secs: record.elapsed_secs,
        },
        reason,
        record.envelope,
    );
    let waiting = app
        .goal
        .as_ref()
        .and_then(|g| g.envelope.as_ref())
        .and_then(|e| e.auto_resume_at().and(e.resume_label.clone()));
    app.note(&match waiting {
        Some(at) => format!(
            "⏸ goal restored after a restart — {} — resumes at {at} when the window resets",
            record.objective
        ),
        None => format!(
            "⏸ goal restored after a restart — {} — /goal resume continues it",
            record.objective
        ),
    });
}

#[cfg(test)]
#[path = "goal_envelope_tests.rs"]
mod tests;
