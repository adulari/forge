//! `forge stats cache` — prompt-cache hit rate per model and why the misses happened.
//! docs/features/context-pipeline.md ("Cache instrument"). `forge stats codemode` renders in
//! stats_codemode.rs.

use crate::cli::args::StatsCmd;
use crate::*;
use anyhow::{Context, Result};

pub(crate) fn stats_cmd(cmd: StatsCmd) -> Result<()> {
    let (session, days, codemode) = match cmd {
        StatsCmd::Cache { session, days } => (session, days, false),
        StatsCmd::Codemode { session, days } => (session, days, true),
    };
    let store = open_store()?;
    let session = match session.as_deref() {
        None => None,
        Some("") => Some(
            store
                .most_recent_session_id()
                .context("looking up the most recent session")?
                .context("no sessions yet")?,
        ),
        Some(prefix) => Some(resolve_session(&store, prefix)?),
    };
    let since = if session.is_some() {
        0
    } else {
        chrono::Utc::now().timestamp() - i64::from(days) * 86_400
    };
    if codemode {
        let stats = store
            .codemode_stats(session.as_deref(), since)
            .context("reading the tool-call audit")?;
        print!(
            "{}",
            super::stats_codemode::render(&stats, session.as_deref(), days)
        );
        return Ok(());
    }
    let stats = store
        .cache_stats(session.as_deref(), since)
        .context("reading usage")?;
    let config = crate::cli::commands::load_config()?;
    let pricing = crate::cli::commands::models::pricing_with_fetched_rates(&config);
    print!("{}", render(&stats, &pricing, session.as_deref(), days));
    Ok(())
}

/// What the uncached input would have cost less had it been read from cache.
fn miss_usd(pricing: &forge_mesh::pricing::Pricing, model: &str, uncached: u64) -> f64 {
    let fresh = forge_types::Usage {
        input_tokens: uncached,
        cached_input_tokens: Some(0),
        ..Default::default()
    };
    let cached = forge_types::Usage {
        cached_input_tokens: Some(uncached),
        ..fresh
    };
    pricing.cost_for_usage(model, &fresh) - pricing.cost_for_usage(model, &cached)
}

fn render(
    stats: &[forge_store::CacheModelStats],
    pricing: &forge_mesh::pricing::Pricing,
    session: Option<&str>,
    days: u32,
) -> String {
    let scope = match session {
        Some(id) => format!("session {}", id.chars().take(8).collect::<String>()),
        None => format!("last {days} days"),
    };
    if stats.is_empty() {
        return format!("no main-loop model calls recorded ({scope})\n");
    }
    let mut out = format!("prompt cache — {scope}\n\n");
    out.push_str(&format!(
        "{:<44} {:>6} {:>8} {:>7} {:>10}  causes\n",
        "model", "calls", "input", "hit", "miss $"
    ));
    for row in stats {
        let hit = row
            .hit_ratio()
            .map_or_else(|| "n/a".to_string(), |r| format!("{:.1}%", r * 100.0));
        let miss = if row.cached_input_tokens.is_some() {
            format!(
                "${:.2}",
                miss_usd(pricing, &row.model, row.uncached_input_tokens)
            )
        } else {
            "n/a".to_string()
        };
        let causes = if row.fingerprinted == 0 {
            "n/a (not fingerprinted: CLI bridge or older rows)".to_string()
        } else {
            row.labels
                .iter()
                .map(|(label, n)| format!("{label} {n}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push_str(&format!(
            "{:<44} {:>6} {:>8} {:>7} {:>10}  {causes}\n",
            row.model,
            row.calls,
            human_tokens(row.input_tokens),
            hit,
            miss
        ));
    }
    out.push_str(
        "\nharness_rewrite = Forge rewrote context it had already sent (should be ~0); \
         floor_advance/age_rounds/compaction/history_rewrite = deliberate; provider_eviction = prefix intact \
         but the provider served it uncached.\n",
    );
    out
}

fn human_tokens(n: u64) -> String {
    match n {
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 1_000 => format!("{:.1}K", n as f64 / 1e3),
        n => n.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_prices_misses_and_marks_bridges_not_applicable() {
        let mut rates = std::collections::HashMap::new();
        rates.insert(
            "bedrock::kimi".to_string(),
            forge_mesh::pricing::ModelRate {
                input_per_1k: 0.003,
                output_per_1k: 0.015,
                cache_read_per_1k: Some(0.0003),
            },
        );
        let pricing = forge_mesh::pricing::Pricing::from_rates(rates);
        let stats = vec![
            forge_store::CacheModelStats {
                model: "bedrock::kimi".into(),
                calls: 10,
                fingerprinted: 10,
                input_tokens: 2_000_000,
                cached_input_tokens: Some(1_000_000),
                uncached_input_tokens: 1_000_000,
                labels: vec![("append".into(), 8), ("harness_rewrite".into(), 2)],
            },
            forge_store::CacheModelStats {
                model: "claude-cli::sonnet".into(),
                calls: 3,
                fingerprinted: 0,
                input_tokens: 9_000,
                cached_input_tokens: None,
                uncached_input_tokens: 0,
                labels: vec![],
            },
        ];
        let table = render(&stats, &pricing, None, 30);
        assert!(table.contains("50.0%"), "{table}");
        // 1M uncached tokens at ($3 - $0.30)/M.
        assert!(table.contains("$2.70"), "{table}");
        assert!(table.contains("harness_rewrite 2"), "{table}");
        assert!(table.contains("n/a (not fingerprinted"), "{table}");
    }
}
