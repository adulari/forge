//! The order in which compaction tries summarizers.

use forge_config::CompactModel;

/// The summarizer chain for `[mesh] compact_model`.
///
/// `Auto` is the order Forge used before the setting existed (free shortlist, routed chain, the
/// session's own model as the can't-exhaust backstop). `Session` puts the session's own model
/// first because a summary replaces everything it folds and that model knows the task, then its
/// fallbacks, then the free shortlist.
///
/// A subscription model is admitted only when it is the session's own, in both orders: a summary is
/// optional work and must not spend a plan the user did not point this session at.
pub(crate) fn chain_for(
    mode: CompactModel,
    shortlist: Vec<String>,
    session_fallbacks: Vec<String>,
    routed: Vec<String>,
    session_model: &str,
    is_benched: impl Fn(&str) -> bool,
    is_subscription: impl Fn(&str) -> bool,
) -> Vec<String> {
    match mode {
        CompactModel::Auto => crate::compact_candidate_chain(
            shortlist,
            routed,
            session_model,
            is_benched,
            is_subscription,
        ),
        CompactModel::Session => session_first_chain(
            session_model,
            session_fallbacks,
            shortlist,
            is_benched,
            is_subscription,
        ),
    }
}

fn session_first_chain(
    session_model: &str,
    fallbacks: Vec<String>,
    shortlist: Vec<String>,
    is_benched: impl Fn(&str) -> bool,
    is_subscription: impl Fn(&str) -> bool,
) -> Vec<String> {
    let admitted = |m: &str| {
        let benched = is_benched(m);
        let foreign_subscription = is_subscription(m) && m != session_model;
        if benched || foreign_subscription {
            tracing::debug!(
                model = m,
                benched,
                foreign_subscription,
                "compaction chain: candidate skipped"
            );
        }
        !m.is_empty() && !benched && !foreign_subscription
    };
    let mut out: Vec<String> = Vec::new();
    let add = |m: String, out: &mut Vec<String>| {
        if !out.contains(&m) {
            out.push(m);
        }
    };
    // A benched session model (rate limited a moment ago) would only burn a call: it goes to the
    // back as the backstop instead of the front.
    if admitted(session_model) {
        add(session_model.to_string(), &mut out);
    } else {
        tracing::info!(
            session_model,
            "compaction chain: the session model is not first (benched or empty); it is kept as \
             the last resort"
        );
    }
    for m in fallbacks.into_iter().filter(|m| admitted(m)) {
        add(m, &mut out);
    }
    for m in shortlist.into_iter().filter(|m| admitted(m)).take(3) {
        add(m, &mut out);
    }
    add(session_model.to_string(), &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(models: &[&str]) -> Vec<String> {
        models.iter().map(|m| m.to_string()).collect()
    }

    fn chain(
        mode: CompactModel,
        shortlist: &[&str],
        fallbacks: &[&str],
        session: &str,
    ) -> Vec<String> {
        chain_for(
            mode,
            list(shortlist),
            list(fallbacks),
            list(fallbacks),
            session,
            |_| false,
            |_| false,
        )
    }

    #[test]
    fn session_mode_leads_with_the_session_model_then_its_fallbacks_then_the_free_shortlist() {
        let out = chain(
            CompactModel::Session,
            &["free::a", "free::b"],
            &["fallback::x"],
            "session::pin",
        );
        assert_eq!(out, ["session::pin", "fallback::x", "free::a", "free::b"]);
    }

    #[test]
    fn session_mode_without_fallbacks_goes_straight_to_the_shortlist() {
        let out = chain(CompactModel::Session, &["free::a"], &[], "session::routed");
        assert_eq!(out, ["session::routed", "free::a"]);
    }

    #[test]
    fn auto_mode_keeps_the_free_shortlist_first_and_the_session_model_last() {
        let out = chain(
            CompactModel::Auto,
            &["free::a", "free::b"],
            &["fallback::x"],
            "session::pin",
        );
        assert_eq!(out, ["free::a", "free::b", "fallback::x", "session::pin"]);
    }

    #[test]
    fn session_mode_caps_the_shortlist_at_three_and_never_repeats_a_model() {
        let out = chain(
            CompactModel::Session,
            &["free::a", "free::b", "free::c", "free::d"],
            &["free::a", "session::pin"],
            "session::pin",
        );
        assert_eq!(out, ["session::pin", "free::a", "free::b", "free::c"]);
    }

    #[test]
    fn session_mode_never_admits_a_subscription_model_the_session_is_not_on() {
        let is_sub = |m: &str| m.starts_with("codex-oauth::") || m.starts_with("claude-cli::");
        let out = chain_for(
            CompactModel::Session,
            list(&["claude-cli::haiku", "groq::fast"]),
            list(&["codex-oauth::gpt-6-astra", "openrouter::x"]),
            Vec::new(),
            "opencode::muse-free",
            |_| false,
            is_sub,
        );
        assert_eq!(out, ["opencode::muse-free", "openrouter::x", "groq::fast"]);
    }

    #[test]
    fn session_mode_may_summarize_on_the_sessions_own_subscription_model() {
        let is_sub = |m: &str| m.starts_with("codex-oauth::");
        let out = chain_for(
            CompactModel::Session,
            list(&["groq::fast"]),
            Vec::new(),
            Vec::new(),
            "codex-oauth::gpt-5.6-luna",
            |_| false,
            is_sub,
        );
        assert_eq!(out, ["codex-oauth::gpt-5.6-luna", "groq::fast"]);
    }

    #[test]
    fn a_benched_session_model_moves_to_the_back_but_is_never_dropped() {
        let out = chain_for(
            CompactModel::Session,
            list(&["free::a"]),
            list(&["fallback::x"]),
            Vec::new(),
            "session::pin",
            |m| m == "session::pin" || m == "free::a",
            |_| false,
        );
        assert_eq!(out, ["fallback::x", "session::pin"]);
    }
}
