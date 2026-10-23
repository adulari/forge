//! What one Codex turn consumed, out of the totals `codex exec` reports.
//!
//! `turn.completed.usage` is the running total of the whole thread, not of the turn: `codex exec
//! resume <id>` reloads the thread's rollout and reports its token totals again from the first
//! turn on. Recording that figure per turn counted every earlier model call once more for each
//! later resume, so a bridged task with a completeness review reported about twice the input it
//! really sent (a two-turn task: 266 085 on the first turn, 328 308 on the second, 62 223 of
//! them new). The first turn of a thread has nothing to subtract.

use forge_types::Usage;

/// `total` less what the same thread had already reported in `prior`.
///
/// A field that went DOWN is not a running total (a fresh thread, or a future Codex that reports
/// per turn), so the whole report stands rather than a clamped zero.
pub(super) fn turn_delta(total: &Usage, prior: Option<&Usage>) -> Usage {
    let Some(prior) = prior else {
        return *total;
    };
    if total.input_tokens < prior.input_tokens || total.output_tokens < prior.output_tokens {
        return *total;
    }
    let part = |now: Option<u64>, before: Option<u64>| match (now, before) {
        (Some(now), Some(before)) => Some(now.saturating_sub(before)),
        _ => None,
    };
    Usage {
        input_tokens: total.input_tokens - prior.input_tokens,
        output_tokens: total.output_tokens - prior.output_tokens,
        cached_input_tokens: part(total.cached_input_tokens, prior.cached_input_tokens),
        cache_write_tokens: part(total.cache_write_tokens, prior.cache_write_tokens),
        reasoning_tokens: part(total.reasoning_tokens, prior.reasoning_tokens),
        ..*total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u64, cached: u64, output: u64, reasoning: u64) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: output,
            cached_input_tokens: Some(cached),
            cache_write_tokens: Some(0),
            reasoning_tokens: Some(reasoning),
            ..Usage::default()
        }
    }

    /// The two `turn.completed` events of one real bridged task: the first `codex exec`, then the
    /// review's `codex exec resume` on the same thread.
    #[test]
    fn a_resumed_turn_reports_only_its_own_calls() {
        let first = usage(266_085, 220_544, 3_979, 715);
        let second_total = usage(328_308, 279_552, 4_430, 976);
        assert_eq!(turn_delta(&first, None), first);
        let second = turn_delta(&second_total, Some(&first));
        assert_eq!(second.input_tokens, 62_223);
        assert_eq!(second.cached_input_tokens, Some(59_008));
        assert_eq!(second.output_tokens, 451);
        assert_eq!(second.reasoning_tokens, Some(261));
        assert_eq!(first.input_tokens + second.input_tokens, 328_308);
    }

    #[test]
    fn a_total_that_went_down_is_a_fresh_count() {
        let prior = usage(300_000, 200_000, 4_000, 0);
        let fresh = usage(15_000, 11_000, 100, 0);
        assert_eq!(turn_delta(&fresh, Some(&prior)), fresh);
    }

    #[test]
    fn an_unreported_split_stays_unreported() {
        let prior = Usage {
            input_tokens: 10,
            output_tokens: 1,
            ..Usage::default()
        };
        let total = Usage {
            input_tokens: 25,
            output_tokens: 3,
            cached_input_tokens: Some(20),
            ..Usage::default()
        };
        let delta = turn_delta(&total, Some(&prior));
        assert_eq!(delta.input_tokens, 15);
        assert_eq!(
            delta.cached_input_tokens, None,
            "a split that was unknown before cannot be split now"
        );
    }
}
