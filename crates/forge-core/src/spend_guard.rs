//! Per-turn and per-session spend guards, in USD.
//!
//! The day/week/month caps (`mesh.daily_budget_usd` and friends) are off by default and are only
//! read when a turn STARTS, so one runaway turn — hundreds of model calls re-sending a 300k-token
//! prompt — was invisible until the bill. On the measured store, 19 turns over $5 were 68% of all
//! recorded spend, and one resumed session reached $186 without any guard ever speaking.
//!
//! These guards read the cost of every call as it is recorded: a warning once a threshold is
//! crossed, then a stop. A stopped turn is not lost work — the next prompt, or `continue`, starts a
//! fresh turn allowance, which is the confirmation. The session cap is the backstop that survives
//! `continue`: it refuses new turns until it is raised or `FORGE_BUDGET_OVERRIDE=1` is set.

use super::*;

/// Where one number sits against its warn threshold and cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    Ok,
    Warn,
    Stop,
}

/// `0` disables a threshold. `hard_stop = false` keeps the cap but only ever warns.
pub(crate) fn level(warn: f64, cap: f64, hard_stop: bool, spent: f64) -> Level {
    if cap > 0.0 && spent >= cap {
        return if hard_stop { Level::Stop } else { Level::Warn };
    }
    if warn > 0.0 && spent >= warn {
        return Level::Warn;
    }
    Level::Ok
}

#[derive(Debug, Default)]
pub(crate) struct SpendState {
    turn_usd: f64,
    turn_warned: bool,
    session_warned: bool,
}

impl SpendState {
    pub(crate) fn begin_turn(&mut self) {
        self.turn_usd = 0.0;
        self.turn_warned = false;
    }
}

/// A side call's usage with its cost filled in. Providers return `cost_usd: 0.0` — the mesh prices
/// a call from its token counts — and the side-call sites recorded that zero, so a compaction
/// that read 220k tokens through a metered model cost nothing on paper: session cost, the
/// day/week/month caps and the guards above never saw it.
pub(crate) fn priced(
    pricing: &forge_mesh::pricing::Pricing,
    model: &str,
    usage: &forge_types::Usage,
) -> forge_types::Usage {
    forge_types::Usage {
        cost_usd: pricing.cost_for_usage(model, usage),
        ..*usage
    }
}

impl Session {
    /// The first hard guard a just-recorded model call trips: the dollar guards, then the
    /// per-turn input-token ceiling. `Some` is the final text of the turn that must end now.
    pub(crate) fn hard_guard_after_call(&mut self, call_usd: f64) -> Option<String> {
        self.spend_guard_after_call(call_usd).or_else(|| {
            self.turn_input_ceiling_hit()
                .then(|| self.abort_for_token_ceiling())
        })
    }

    /// Charge one recorded model call to the turn and check both guards. Returns the final text
    /// when the turn must end; a warning is emitted at most once per turn / per process.
    pub(crate) fn spend_guard_after_call(&mut self, call_usd: f64) -> Option<String> {
        self.spend.turn_usd += call_usd.max(0.0);
        let b = self.config.mesh.budget;
        let turn = level(
            b.turn_warn_usd,
            b.turn_cap_usd,
            b.hard_stop,
            self.spend.turn_usd,
        );
        let session_usd = self.store.session_cost(&self.id).unwrap_or(0.0);
        let session = level(
            b.session_warn_usd,
            b.session_cap_usd,
            b.hard_stop,
            session_usd,
        );
        let overridden = budget_override_active();

        if turn == Level::Stop && !overridden {
            return Some(self.abort_for_spend_cap(format!(
                "turn spend cap reached (${:.2} this turn, cap ${:.2} `mesh.budget.turn_cap_usd`) \
                 — send a new prompt or `continue` for a fresh turn allowance",
                self.spend.turn_usd, b.turn_cap_usd
            )));
        }
        if session == Level::Stop && !overridden {
            return Some(self.abort_for_spend_cap(format!(
                "session spend cap reached (${session_usd:.2}, cap ${:.2} \
                 `mesh.budget.session_cap_usd`) — raise the cap or set FORGE_BUDGET_OVERRIDE=1 \
                 to go on",
                b.session_cap_usd
            )));
        }
        if turn != Level::Ok && !self.spend.turn_warned {
            self.spend.turn_warned = true;
            self.presenter.emit(PresenterEvent::Warning(format!(
                "this turn has spent ${:.2} (warn ${:.2}, stop ${:.2}); every step re-sends the whole \
                 context — consider /compact",
                self.spend.turn_usd, b.turn_warn_usd, b.turn_cap_usd
            )));
        }
        if session != Level::Ok && !self.spend.session_warned {
            self.spend.session_warned = true;
            self.presenter.emit(PresenterEvent::Warning(format!(
                "this session has spent ${session_usd:.2} (warn ${:.2}, stop ${:.2})",
                b.session_warn_usd, b.session_cap_usd
            )));
        }
        None
    }

    /// Why this session may not start another turn, if its lifetime spend is already at the cap.
    pub(crate) fn session_spend_refusal(&self) -> Option<String> {
        let b = self.config.mesh.budget;
        let spent = self.store.session_cost(&self.id).unwrap_or(0.0);
        if level(b.session_warn_usd, b.session_cap_usd, b.hard_stop, spent) != Level::Stop
            || budget_override_active()
        {
            return None;
        }
        Some(format!(
            "session spend cap reached (${spent:.2}, cap ${:.2} `mesh.budget.session_cap_usd`). \
             Refusing further model calls. Raise the cap or set FORGE_BUDGET_OVERRIDE=1 to proceed.",
            b.session_cap_usd
        ))
    }

    fn abort_for_spend_cap(&mut self, reason: String) -> String {
        let work = self.preserve_uncommitted_work();
        self.presenter.emit(PresenterEvent::Error(format!(
            "ERROR: {reason}; work is uncommitted: {work}"
        )));
        self.turn_hard_guard_abort = true;
        format!("ERROR: {reason}; work is uncommitted: {work}")
    }
}
