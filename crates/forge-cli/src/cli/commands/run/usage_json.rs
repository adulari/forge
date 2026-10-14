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

pub(super) fn write_usage_json(session: &Session, path: &Path, wall_secs: f64) -> Result<()> {
    let value = usage_json_value(
        session.session_id(),
        &session.session_usage_detail(),
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
}
