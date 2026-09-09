//! What Forge says when the model the user pinned is the thing that is broken.
//!
//! A pin deliberately overrides routing: no classification, no failover, no health filter. That is
//! what a pin is for. The cost is that every health signal Forge already has about that model stops
//! reaching the user.
//!
//! Live failure (2026-09-09, session `07ca114e`): the session was pinned to
//! `meta::muse-spark-1.3-contributor`, which had started returning HTTP-success completions with no
//! text, no tool call and zero tokens billed. Twelve consecutive turns died that way. `forge models`
//! had been printing that exact id as `benched` the whole time, and the turn's own error said only
//! "model returned an empty response … stopping the turn" — true, and useless: it never mentioned
//! the pin, the bench, or `/model`. The user reasonably read it as their session being corrupted.
//!
//! Both messages here exist to close that gap; the reasoning lives with the words rather than
//! inline in the model loop, so it can be tested without a session.

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Roughly how long the bench has left, for a message rather than a table.
fn remaining(until: i64) -> String {
    let secs = until.saturating_sub(now());
    if secs <= 0 {
        return "expiring now".to_string();
    }
    let mins = secs / 60;
    match mins {
        0 => "under a minute left".to_string(),
        1 => "about a minute left".to_string(),
        m if m < 60 => format!("about {m} minutes left"),
        m => format!("about {} hours left", m / 60),
    }
}

/// Said once at the top of a pinned turn whose model Forge has already benched.
///
/// The pin still wins — Forge does not silently re-route a model the user chose by hand — but the
/// user gets the health verdict that routing would have acted on, and the two commands that resolve
/// it either way.
pub(crate) fn pinned_but_benched(model: &str, until: i64, reason: &str) -> String {
    format!(
        "you are pinned to {model}, which Forge has benched ({reason}; {}). The pin overrides \
         routing, so this turn will still call it. If it keeps failing, clear the pin with \
         `/model` or pin a working model; `forge models --probe` re-checks it.",
        remaining(until)
    )
}

/// [`pinned_but_benched`] for `model` when the health table currently benches it, else `None`.
pub(crate) fn benched_pin_warning(store: &forge_store::Store, model: &str) -> Option<String> {
    store
        .current_benched_report()
        .unwrap_or_default()
        .into_iter()
        .find(|(m, _, _)| m == model)
        .map(|(_, until, reason)| pinned_but_benched(model, until, &reason))
}

/// Said when a model answered nothing until the retries ran out and nothing could take over.
pub(crate) fn empty_response_stop(model: &str, pinned: bool) -> String {
    if pinned {
        format!(
            "{model} returned empty responses (no text, no tool call) and it is PINNED, so there \
             was no failover — stopping the turn. The model has been benched. This is the pinned \
             model failing, not your session: clear the pin with `/model`, or pin another model, \
             and the conversation continues from here."
        )
    } else {
        format!(
            "{model} returned an empty response (no text, no tool call) and no fallback model was \
             available — stopping the turn. It has been benched."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_minutes(m: i64) -> i64 {
        now() + m * 60
    }

    #[test]
    fn the_pin_warning_names_the_model_the_reason_and_the_way_out() {
        let text = pinned_but_benched(
            "meta::muse-spark-1.3-contributor",
            in_minutes(20),
            "empty response (no text, no tool call)",
        );
        assert!(text.contains("meta::muse-spark-1.3-contributor"));
        assert!(text.contains("empty response"));
        assert!(text.contains("about 20 minutes left"));
        assert!(text.contains("/model"));
        // The pin is still honoured; the warning must not claim Forge re-routed.
        assert!(text.contains("will still call it"));
    }

    #[test]
    fn an_expired_or_expiring_bench_still_reads_sensibly() {
        assert!(pinned_but_benched("m", 0, "r").contains("expiring now"));
        assert!(pinned_but_benched("m", in_minutes(1), "r").contains("about a minute"));
        assert!(pinned_but_benched("m", in_minutes(600), "r").contains("about 10 hours"));
    }

    #[test]
    fn the_stop_message_blames_the_pin_only_when_there_was_one() {
        let pinned = empty_response_stop("x::y", true);
        assert!(pinned.contains("PINNED"));
        assert!(pinned.contains("/model"));
        assert!(pinned.contains("not your session"));

        let routed = empty_response_stop("x::y", false);
        assert!(!routed.contains("/model"));
        assert!(routed.contains("no fallback model was available"));
    }
}
