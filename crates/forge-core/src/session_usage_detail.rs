//! Per-session usage accounting for benchmarks: every token split the store holds, the call and
//! tool counts, and a notional price for calls that recorded no spend.

use super::*;

/// What one session consumed, summed over every provider call (side calls and deactivated turns
/// included). Optional totals are `None` when not one call reported that split.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionUsageDetail {
    pub input_tokens: u64,
    pub cached_input_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub output_tokens: u64,
    /// Spend the store recorded (zero for subscription and free routes).
    pub cost_usd: f64,
    /// `cost_usd` plus every zero-cost call priced at its model's list rate. NOT spend: it puts
    /// subscription/free arms on the same axis as metered ones. `None` when some zero-cost call
    /// that used tokens has no tracked price, so the figure would understate.
    pub notional_cost_usd: Option<f64>,
    /// Provider calls (one `usage` row each).
    pub model_calls: u64,
    pub tool_calls: u64,
}

impl Session {
    /// [`SessionUsageDetail`] for this session, read from the store. Store errors yield the
    /// default (all zero) rather than failing a benchmark attempt.
    pub fn session_usage_detail(&self) -> SessionUsageDetail {
        let rows = self
            .store
            .session_usage_by_model(self.session_id())
            .unwrap_or_default();
        let tool_calls = self
            .store
            .session_tool_call_count(self.session_id())
            .unwrap_or(0);
        usage_detail_from_rows(&rows, tool_calls, &self.pricing)
    }
}

fn add_opt(total: Option<u64>, part: Option<u64>) -> Option<u64> {
    match (total, part) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
    }
}

pub(crate) fn usage_detail_from_rows(
    rows: &[forge_store::ModelUsageRow],
    tool_calls: u64,
    pricing: &Pricing,
) -> SessionUsageDetail {
    let mut detail = SessionUsageDetail {
        tool_calls,
        notional_cost_usd: Some(0.0),
        ..SessionUsageDetail::default()
    };
    for row in rows {
        let usage = &row.usage;
        detail.input_tokens += usage.input_tokens;
        detail.output_tokens += usage.output_tokens;
        detail.cached_input_tokens = add_opt(detail.cached_input_tokens, usage.cached_input_tokens);
        detail.cache_write_tokens = add_opt(detail.cache_write_tokens, usage.cache_write_tokens);
        detail.reasoning_tokens = add_opt(detail.reasoning_tokens, usage.reasoning_tokens);
        detail.cost_usd += usage.cost_usd;
        detail.model_calls += row.calls;
        let row_notional = if !row.zero_cost {
            Some(usage.cost_usd)
        } else if usage.total_tokens() == 0 {
            Some(0.0)
        } else {
            row.model
                .as_deref()
                .and_then(|model| pricing.reference_cost_for_usage(model, usage))
        };
        detail.notional_cost_usd = match (detail.notional_cost_usd, row_notional) {
            (Some(total), Some(part)) => Some(total + part),
            _ => None,
        };
    }
    detail
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_store::ModelUsageRow;
    use forge_types::Usage;

    fn row(model: &str, zero_cost: bool, calls: u64, usage: Usage) -> ModelUsageRow {
        ModelUsageRow {
            model: Some(model.to_string()),
            zero_cost,
            calls,
            usage,
        }
    }

    fn pricing() -> Pricing {
        Pricing::from_config_with_fetched(
            &forge_config::Config::default(),
            vec![(
                "openrouter::moonshotai/kimi-k3".to_string(),
                0.003,
                0.015,
                Some(0.0003),
            )],
        )
    }

    #[test]
    fn zero_cost_calls_get_a_notional_price_and_billed_calls_keep_their_cost() {
        let rows = vec![
            row(
                "bedrock::kimi-k3",
                false,
                2,
                Usage {
                    reported_cost_usd: None,
                    input_tokens: 1_000,
                    output_tokens: 100,
                    cached_input_tokens: Some(500),
                    cache_write_tokens: Some(200),
                    cost_usd: 0.5,
                    reasoning_tokens: None,
                },
            ),
            row(
                "kimi::kimi-k3",
                true,
                3,
                Usage {
                    input_tokens: 1_000,
                    output_tokens: 1_000,
                    cached_input_tokens: None,
                    reasoning_tokens: Some(40),
                    ..Default::default()
                },
            ),
        ];
        let detail = usage_detail_from_rows(&rows, 7, &pricing());
        assert_eq!(detail.model_calls, 5);
        assert_eq!(detail.tool_calls, 7);
        assert_eq!(detail.input_tokens, 2_000);
        assert_eq!(detail.cached_input_tokens, Some(500));
        assert_eq!(detail.cache_write_tokens, Some(200));
        assert_eq!(detail.reasoning_tokens, Some(40));
        assert!((detail.cost_usd - 0.5).abs() < 1e-9);
        // 0.5 real + (1000 fresh * 0.003 + 1000 out * 0.015) / 1k notional
        let notional = detail.notional_cost_usd.expect("every call is priced");
        assert!((notional - (0.5 + 0.003 + 0.015)).abs() < 1e-9);
    }

    #[test]
    fn an_unpriced_zero_cost_call_makes_the_notional_total_unknown() {
        let rows = vec![row(
            "ollama::qwen3",
            true,
            1,
            Usage {
                input_tokens: 10,
                output_tokens: 10,
                ..Default::default()
            },
        )];
        let detail = usage_detail_from_rows(&rows, 0, &pricing());
        assert_eq!(detail.notional_cost_usd, None);
        assert_eq!(
            detail.cached_input_tokens, None,
            "nothing reported → unknown"
        );
    }
}
