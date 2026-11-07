//! The bridged expert's budget meter, kept per session in the store. `forge mcp-serve` is a
//! fresh process for every `codex exec` and again whenever the claude process is respawned, so
//! a meter held in the process took a new baseline (and a new unverified allowance) each time
//! and let every turn spend its own slice. The state lives in the newest `expert/meter:` row of
//! the session instead, and the spend is the session's stored expert rows.

use super::*;
use serde::{Deserialize, Serialize};

const METER_LABEL: &str = "expert/meter:";

#[derive(Debug, Default, Serialize, Deserialize)]
struct BridgeMeter {
    envelope: Option<EnvelopeState>,
    #[serde(default)]
    unverified_consults: u32,
}

impl BridgeMeter {
    /// What a later process must see again: the baseline, not the reading it was compared with.
    fn baseline(&self) -> (Option<String>, u32) {
        let base = self
            .envelope
            .as_ref()
            .map(|e| serde_json::json!([e.spec, e.start, e.carried, e.resets_at]).to_string());
        (base, self.unverified_consults)
    }
}

/// Why the session's expert budget allows no further bridged consult, or `None` to go ahead.
/// An allowed consult made without a fresh reading counts against the session's allowance.
pub(crate) fn bridge_budget_refusal(
    store: &forge_store::Store,
    session_id: &str,
    model: &str,
    spec: &EnvelopeSpec,
    budget: &str,
    now: i64,
) -> Option<String> {
    let mut state: BridgeMeter = store
        .latest_side_call_label(session_id, METER_LABEL)
        .ok()
        .flatten()
        .and_then(|label| serde_json::from_str(label.strip_prefix(METER_LABEL)?).ok())
        .unwrap_or_default();
    let before = state.baseline();
    let windows = store.subscription_windows().unwrap_or_default();
    let spent_usd = store.side_call_cost(session_id, "expert/").unwrap_or(0.0);
    let check = meter(&mut state.envelope, spec, &windows, spent_usd, now);
    let refusal = envelope_refusal(
        check,
        state.envelope.as_ref(),
        &mut state.unverified_consults,
        budget,
    );
    if state.baseline() != before {
        if let Ok(json) = serde_json::to_string(&state) {
            let label = format!("{METER_LABEL}{json}");
            let usage = forge_types::Usage::default();
            if let Err(error) =
                store.record_side_call_usage_for(session_id, &label, Some(model), &usage)
            {
                tracing::warn!(%error, "expert meter was not recorded");
            }
        }
    }
    refusal
}

/// The refusal for a metered check, shared by the direct and the bridged path. Without a fresh
/// reading the slice cannot be verified. A consult usually refreshes the reading itself, so a
/// few are allowed; a meter that never comes back must not let an overnight run draw on the plan
/// unchecked.
pub(crate) fn envelope_refusal(
    check: EnvelopeCheck,
    envelope: Option<&EnvelopeState>,
    unverified_consults: &mut u32,
    budget: &str,
) -> Option<String> {
    let budget = budget.trim();
    match check {
        EnvelopeCheck::Spent => {
            let used = envelope.map_or(0.0, |e| e.used);
            Some(format!(
                "the expert budget ({budget}) is spent at {used:.1}"
            ))
        }
        EnvelopeCheck::Within => None,
        EnvelopeCheck::Unverified if *unverified_consults < UNVERIFIED_CONSULTS_MAX => {
            *unverified_consults += 1;
            None
        }
        EnvelopeCheck::Unverified => Some(format!(
            "the expert budget ({budget}) cannot be checked: no fresh plan-usage reading came back \
             after {UNVERIFIED_CONSULTS_MAX} consults. Run `forge doctor` to check the login, or \
             clear [harness.expert] model to stop using it"
        )),
    }
}
