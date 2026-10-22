//! What a machine-format `result` needs that the presenter's events do not carry: the tokens
//! written to the prompt cache and the per-model split. The CLI fills a [`UsageSlot`] once the
//! session exists, with a reader over the session's store; the presenters read it at the end of
//! every model call and at the end of the turn.

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

/// One model's share of a session's usage, summed over every provider call it served.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelUsage {
    pub model: String,
    pub calls: u64,
    /// Everything sent to the model, cached or not (a superset of the two cache counts).
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

/// Reads the session's cumulative per-model usage.
pub type UsageSource = Arc<dyn Fn() -> Vec<ModelUsage> + Send + Sync>;

/// Hand-off for the usage reader: empty until the session is built.
#[derive(Clone, Default)]
pub struct UsageSlot(Arc<Mutex<Option<UsageSource>>>);

impl UsageSlot {
    pub fn fill(&self, source: UsageSource) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(source);
        }
    }

    /// The session's cumulative usage per model, `None` before the slot is filled.
    pub(crate) fn snapshot(&self) -> Option<Vec<ModelUsage>> {
        let source = self.0.lock().ok()?.clone()?;
        Some(merge(source()))
    }
}

/// One entry per model: the store keeps free and billed calls of one model in separate rows.
fn merge(rows: Vec<ModelUsage>) -> Vec<ModelUsage> {
    let mut merged: Vec<ModelUsage> = Vec::new();
    for row in rows {
        match merged.iter_mut().find(|m| m.model == row.model) {
            Some(m) => {
                m.calls += row.calls;
                m.input_tokens += row.input_tokens;
                m.cache_read_tokens += row.cache_read_tokens;
                m.cache_write_tokens += row.cache_write_tokens;
                m.output_tokens += row.output_tokens;
                m.cost_usd += row.cost_usd;
            }
            None => merged.push(row),
        }
    }
    merged
}

pub(crate) fn cache_writes(usage: &[ModelUsage]) -> u64 {
    usage.iter().map(|m| m.cache_write_tokens).sum()
}

/// What `now` adds on top of `base`, without the models that did nothing in between.
pub(crate) fn since(now: &[ModelUsage], base: &[ModelUsage]) -> Vec<ModelUsage> {
    now.iter()
        .filter_map(|n| {
            let b = base.iter().find(|b| b.model == n.model);
            let sub =
                |now: u64, base: fn(&ModelUsage) -> u64| now.saturating_sub(b.map_or(0, base));
            let diff = ModelUsage {
                model: n.model.clone(),
                calls: sub(n.calls, |m| m.calls),
                input_tokens: sub(n.input_tokens, |m| m.input_tokens),
                cache_read_tokens: sub(n.cache_read_tokens, |m| m.cache_read_tokens),
                cache_write_tokens: sub(n.cache_write_tokens, |m| m.cache_write_tokens),
                output_tokens: sub(n.output_tokens, |m| m.output_tokens),
                cost_usd: (n.cost_usd - b.map_or(0.0, |m| m.cost_usd)).max(0.0),
            };
            (diff.calls > 0 || diff.input_tokens > 0 || diff.output_tokens > 0).then_some(diff)
        })
        .collect()
}

/// Claude Code's `modelUsage`: one object per model, `inputTokens` excluding both cache counts.
pub(crate) fn model_usage_json(usage: &[ModelUsage]) -> Value {
    let mut map = serde_json::Map::new();
    for m in usage {
        let uncached = m
            .input_tokens
            .saturating_sub(m.cache_read_tokens)
            .saturating_sub(m.cache_write_tokens);
        map.insert(
            m.model.clone(),
            json!({
                "inputTokens": uncached,
                "outputTokens": m.output_tokens,
                "cacheReadInputTokens": m.cache_read_tokens,
                "cacheCreationInputTokens": m.cache_write_tokens,
                "webSearchRequests": 0,
                "costUSD": m.cost_usd,
            }),
        );
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(model: &str, calls: u64, input: u64, read: u64, write: u64, out: u64) -> ModelUsage {
        ModelUsage {
            model: model.into(),
            calls,
            input_tokens: input,
            cache_read_tokens: read,
            cache_write_tokens: write,
            output_tokens: out,
            cost_usd: 0.5,
        }
    }

    #[test]
    fn free_and_billed_rows_of_one_model_become_one_entry() {
        let slot = UsageSlot::default();
        assert!(slot.snapshot().is_none());
        slot.fill(Arc::new(|| {
            vec![usage("a", 1, 10, 0, 0, 1), usage("a", 2, 20, 5, 0, 2)]
        }));
        let merged = slot.snapshot().unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!((merged[0].calls, merged[0].input_tokens), (3, 30));
    }

    #[test]
    fn since_subtracts_the_baseline_and_drops_idle_models() {
        let base = vec![usage("a", 1, 100, 40, 10, 5), usage("idle", 1, 9, 0, 0, 1)];
        let now = vec![
            usage("a", 2, 160, 90, 10, 9),
            usage("idle", 1, 9, 0, 0, 1),
            usage("new", 1, 7, 0, 0, 2),
        ];
        let diff = since(&now, &base);
        assert_eq!(diff.len(), 2);
        assert_eq!(
            (
                diff[0].model.as_str(),
                diff[0].input_tokens,
                diff[0].cache_read_tokens
            ),
            ("a", 60, 50)
        );
        assert_eq!(diff[1].model, "new");
    }

    #[test]
    fn model_usage_reports_uncached_input_and_both_cache_counts() {
        let v = model_usage_json(&[usage("claude-cli::haiku", 1, 19_424, 11_227, 8_195, 4)]);
        let m = &v["claude-cli::haiku"];
        assert_eq!(m["inputTokens"], 2);
        assert_eq!(m["cacheReadInputTokens"], 11_227);
        assert_eq!(m["cacheCreationInputTokens"], 8_195);
        assert_eq!(m["outputTokens"], 4);
        assert_eq!(m["costUSD"], 0.5);
    }
}
