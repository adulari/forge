//! `forge run --usage-json <path>`: what the run's session consumed, as one JSON object, for a
//! harness (a benchmark runner or a container agent shim) that drives Forge as a subprocess.

use std::path::Path;

use anyhow::{Context, Result};
use forge_core::{Session, SessionUsageDetail};

pub(crate) fn usage_json_value(
    session_id: &str,
    detail: &SessionUsageDetail,
    wall_secs: f64,
) -> serde_json::Value {
    serde_json::json!({
        "session_id": session_id,
        "model_calls": detail.model_calls,
        "tool_calls": detail.tool_calls,
        "input_tokens": detail.input_tokens,
        "cached_input_tokens": detail.cached_input_tokens,
        "cache_write_tokens": detail.cache_write_tokens,
        "reasoning_tokens": detail.reasoning_tokens,
        "output_tokens": detail.output_tokens,
        "cost_usd": detail.cost_usd,
        "notional_cost_usd": detail.notional_cost_usd,
        "wall_secs": wall_secs,
    })
}

/// What this invocation added on top of `before`. A `--resume`d session's store rows include
/// every earlier turn, so reporting the raw totals would bill this run for all of them.
pub(crate) fn usage_since(
    before: &SessionUsageDetail,
    after: &SessionUsageDetail,
) -> SessionUsageDetail {
    let opt = |b: Option<u64>, a: Option<u64>| match (b, a) {
        (Some(b), Some(a)) => Some(a.saturating_sub(b)),
        (_, a) => a,
    };
    SessionUsageDetail {
        input_tokens: after.input_tokens.saturating_sub(before.input_tokens),
        cached_input_tokens: opt(before.cached_input_tokens, after.cached_input_tokens),
        cache_write_tokens: opt(before.cache_write_tokens, after.cache_write_tokens),
        reasoning_tokens: opt(before.reasoning_tokens, after.reasoning_tokens),
        output_tokens: after.output_tokens.saturating_sub(before.output_tokens),
        cost_usd: (after.cost_usd - before.cost_usd).max(0.0),
        notional_cost_usd: match (before.notional_cost_usd, after.notional_cost_usd) {
            (Some(b), Some(a)) => Some((a - b).max(0.0)),
            // The resumed history had an unpriced call: the delta cannot be trusted either.
            (None, Some(_)) if before.model_calls > 0 => None,
            (_, a) => a,
        },
        model_calls: after.model_calls.saturating_sub(before.model_calls),
        tool_calls: after.tool_calls.saturating_sub(before.tool_calls),
    }
}

pub(super) fn write_usage_json(
    session: &Session,
    before: &SessionUsageDetail,
    path: &Path,
    wall_secs: f64,
) -> Result<()> {
    let value = usage_json_value(
        session.session_id(),
        &usage_since(before, &session.session_usage_detail()),
        wall_secs,
    );
    let body = serde_json::to_string_pretty(&value).expect("usage JSON serializes") + "\n";
    std::fs::write(path, body).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreported_splits_serialize_as_null_not_zero() {
        let detail = SessionUsageDetail {
            input_tokens: 120,
            output_tokens: 30,
            model_calls: 2,
            tool_calls: 1,
            ..SessionUsageDetail::default()
        };
        let v = usage_json_value("s-1", &detail, 1.5);
        assert_eq!(v["input_tokens"], 120);
        assert_eq!(v["model_calls"], 2);
        assert!(v["cached_input_tokens"].is_null());
        assert!(v["notional_cost_usd"].is_null());
        assert_eq!(v["session_id"], "s-1");
    }

    #[test]
    fn a_resumed_session_reports_only_this_runs_usage() {
        let before = SessionUsageDetail {
            input_tokens: 1000,
            output_tokens: 100,
            cached_input_tokens: Some(800),
            cost_usd: 0.5,
            notional_cost_usd: Some(0.5),
            model_calls: 4,
            tool_calls: 3,
            ..SessionUsageDetail::default()
        };
        let after = SessionUsageDetail {
            input_tokens: 1500,
            output_tokens: 130,
            cached_input_tokens: Some(1200),
            reasoning_tokens: Some(20),
            cost_usd: 0.75,
            notional_cost_usd: Some(0.75),
            model_calls: 6,
            tool_calls: 4,
            ..SessionUsageDetail::default()
        };
        let d = usage_since(&before, &after);
        assert_eq!(d.input_tokens, 500);
        assert_eq!(d.output_tokens, 30);
        assert_eq!(d.cached_input_tokens, Some(400));
        assert_eq!(d.reasoning_tokens, Some(20));
        assert_eq!(d.cache_write_tokens, None);
        assert_eq!(d.model_calls, 2);
        assert_eq!(d.tool_calls, 1);
        assert!((d.cost_usd - 0.25).abs() < 1e-9);
        assert!((d.notional_cost_usd.unwrap() - 0.25).abs() < 1e-9);
        let fresh = usage_since(&SessionUsageDetail::default(), &after);
        assert_eq!(fresh.input_tokens, 1500);
        assert_eq!(fresh.notional_cost_usd, Some(0.75));
    }
}
