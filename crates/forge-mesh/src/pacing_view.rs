//! Reporting subscription pacing to a human.
//!
//! Pacing (#1201) silently removes a subscription's expensive siblings from routing when its
//! window is ahead of schedule. Every surface that shows a quota window renders the decision
//! through these helpers, so a `forge mesh` line, the `/mesh` overlay, and the routing rationale
//! all quote the SAME `SubscriptionPacing` the router acted on rather than recomputing a pace of
//! their own.

use forge_types::SubscriptionPacing;

/// One over-pace subscription provider: the pacing decision plus which of its models routing
/// holds back and which it keeps.
#[derive(Debug, Clone, PartialEq)]
pub struct PacingHold {
    pub provider: String,
    pub pacing: SubscriptionPacing,
    /// Models pacing withholds from routing (the expensive siblings).
    pub held: Vec<String>,
    /// Models pacing leaves routable — empty when the whole provider is held.
    pub kept: Vec<String>,
}

/// `gpt-5.6-sol` for `codex-oauth::gpt-5.6-sol` — provider-qualified names make these lines
/// unreadable when three siblings are listed.
fn short_model(model: &str) -> &str {
    model.rsplit("::").next().unwrap_or(model)
}

/// Past a handful of models the names stop informing and start flooding (an over-pace
/// OpenCode Go holds two dozen), so long lists collapse to a count.
fn join_short(models: &[String]) -> String {
    const MAX_NAMES: usize = 3;
    if models.len() > MAX_NAMES {
        return format!("{} models", models.len());
    }
    models
        .iter()
        .map(|model| short_model(model))
        .collect::<Vec<_>>()
        .join("/")
}

/// The provider-line pace marker: `weekly 32% used · 21% allowed · OVER PACE → sol/terra held,
/// luna`, `… · on pace`, or `pace unknown` when the window has no authoritative reset.
///
/// A window paced from the nominal length instead of a provider reset is reported as unknown, not
/// quietly presented as fact (§6): the start instant is a guess, so the allowance derived from it
/// is not a number to act on.
pub fn pacing_summary(pacing: Option<&SubscriptionPacing>, hold: Option<&PacingHold>) -> String {
    let Some(pacing) = pacing else {
        return "pace unknown (no observed window)".to_string();
    };
    let used = format!(
        "{} {:.0}% used",
        pacing.window,
        pacing.fraction_used * 100.0
    );
    if pacing.used_nominal_fallback {
        return format!(
            "{used} · pace unknown (no reset time — used_nominal_fallback from the nominal {} window)",
            pacing.window,
        );
    }
    let allowed = format!("{:.0}% allowed", pacing.allowed_fraction * 100.0);
    if !pacing.is_over_pace() {
        return format!("{used} · {allowed} · on pace");
    }
    let mut line = format!("{used} · {allowed} · OVER PACE");
    if let Some(hold) = hold.filter(|hold| !hold.held.is_empty()) {
        line.push_str(&format!(" → {} held", join_short(&hold.held)));
        if !hold.kept.is_empty() {
            line.push_str(&format!(", {}", join_short(&hold.kept)));
        }
    }
    line
}

/// The routing-rationale form, which names the held models in full so the claim can be checked:
/// `codex-oauth::gpt-5.6-sol held: weekly 32% > 21% allowed at 29% elapsed`. Past a handful of
/// names the full list stops being readable, so long holds collapse to a count — the JSON view
/// carries the complete list for anyone who needs it.
pub fn pacing_hold_note(hold: &PacingHold) -> String {
    const MAX_NAMES: usize = 4;
    let pacing = &hold.pacing;
    let held = if hold.held.len() > MAX_NAMES {
        format!("{} models", hold.held.len())
    } else {
        hold.held.join(", ")
    };
    let basis = if pacing.used_nominal_fallback {
        " (pace unknown: no reset time, used_nominal_fallback)"
    } else {
        ""
    };
    let kept = if hold.kept.is_empty() {
        String::new()
    } else if hold.kept.len() > MAX_NAMES {
        format!(" → {} models kept", hold.kept.len())
    } else {
        format!(" → {} kept", hold.kept.join(", "))
    };
    format!(
        " — pacing: {held} held: {} {:.0}% > {:.0}% allowed at {:.0}% elapsed{basis}{kept}",
        pacing.window,
        pacing.fraction_used * 100.0,
        pacing.allowed_fraction * 100.0,
        pacing.elapsed_fraction() * 100.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pacing(fraction_used: f64, allowed: f64, fallback: bool) -> SubscriptionPacing {
        SubscriptionPacing {
            window: "weekly".into(),
            fraction_used,
            allowed_fraction: allowed,
            elapsed_secs: 172_800,
            total_secs: 604_800,
            resets_at: (!fallback).then_some(1_000_000),
            used_nominal_fallback: fallback,
        }
    }

    #[test]
    fn an_over_pace_window_names_the_models_it_holds() {
        let hold = PacingHold {
            provider: "codex-oauth".into(),
            pacing: pacing(0.32, 0.21, false),
            held: vec![
                "codex-oauth::gpt-5.6-sol".into(),
                "codex-oauth::gpt-5.6-terra".into(),
            ],
            kept: vec!["codex-oauth::gpt-5.6-luna".into()],
        };
        assert_eq!(
            pacing_summary(Some(&hold.pacing), Some(&hold)),
            "weekly 32% used · 21% allowed · OVER PACE → gpt-5.6-sol/gpt-5.6-terra held, gpt-5.6-luna"
        );
        let note = pacing_hold_note(&hold);
        assert!(
            note.contains(
                "codex-oauth::gpt-5.6-sol, codex-oauth::gpt-5.6-terra held: \
                 weekly 32% > 21% allowed at 29% elapsed"
            ),
            "{note}"
        );
        assert!(note.contains("codex-oauth::gpt-5.6-luna kept"), "{note}");
    }

    #[test]
    fn a_window_within_its_allowance_reads_as_on_pace() {
        assert_eq!(
            pacing_summary(Some(&pacing(0.05, 0.21, false)), None),
            "weekly 5% used · 21% allowed · on pace"
        );
    }

    #[test]
    fn a_long_hold_list_collapses_to_a_count() {
        let hold = PacingHold {
            provider: "opencode_go".into(),
            pacing: pacing(0.67, 0.24, false),
            held: (0..25).map(|i| format!("opencode_go::m{i}")).collect(),
            kept: (0..6).map(|i| format!("opencode_go::k{i}")).collect(),
        };
        assert_eq!(
            pacing_summary(Some(&hold.pacing), Some(&hold)),
            "weekly 67% used · 24% allowed · OVER PACE → 25 models held, 6 models"
        );
        let note = pacing_hold_note(&hold);
        assert!(
            note.contains("25 models held: weekly 67% > 24% allowed"),
            "{note}"
        );
        assert!(note.contains("6 models kept"), "{note}");
    }

    #[test]
    fn a_window_without_a_reset_is_reported_unknown_not_estimated() {
        let summary = pacing_summary(Some(&pacing(0.32, 0.21, true)), None);
        assert!(summary.contains("pace unknown"), "{summary}");
        assert!(summary.contains("used_nominal_fallback"), "{summary}");
        assert!(
            !summary.contains("allowed"),
            "a guessed allowance must never be presented as one: {summary}"
        );
        assert_eq!(
            pacing_summary(None, None),
            "pace unknown (no observed window)"
        );
    }
}
