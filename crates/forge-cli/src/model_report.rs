//! The model report card: what each model did for this user over a trailing window, next to its
//! leaderboard score. One builder feeds `forge models --report`, the daemon's
//! `/api/models/report` route and (through the daemon) the app and TUI Models views.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use forge_store::{ModelReportRow, Store};

use crate::serve::{err_response, json_response, DaemonState};

pub(crate) const DEFAULT_DAYS: u32 = 30;
pub(crate) const MAX_DAYS: u32 = 365;

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new().route(&format!("{base}/api/models/report"), get(model_report))
}

#[derive(serde::Deserialize, Default)]
struct ReportParams {
    days: Option<u32>,
    /// Limit to one repository (a session `cwd` or worktree path).
    repo: Option<String>,
}

async fn model_report(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<ReportParams>,
) -> Response {
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || {
        let catalog =
            crate::cli::commands::models::load_cached_catalog_aged().map(|(catalog, _, _)| catalog);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        build(
            &store,
            &|model| catalog.as_ref().and_then(|c| c.benchmark_for(model)),
            params.days.unwrap_or(DEFAULT_DAYS),
            params.repo.as_deref().filter(|r| !r.is_empty()),
            now,
        )
    })
    .await;
    match result {
        Ok(Ok(report)) => json_response(&report),
        _ => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not build the model report",
        ),
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ReportEntry {
    #[serde(flatten)]
    pub(crate) stats: ModelReportRow,
    pub(crate) completion_rate: Option<f64>,
    pub(crate) tool_error_rate: Option<f64>,
    pub(crate) benchmark_intelligence: Option<f64>,
    pub(crate) benchmark_coding: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ModelReport {
    pub(crate) days: u32,
    pub(crate) repo: Option<String>,
    pub(crate) generated_at: i64,
    pub(crate) models: Vec<ReportEntry>,
}

pub(crate) fn build(
    store: &Store,
    benchmark: &dyn Fn(&str) -> Option<(f64, f64)>,
    days: u32,
    repo: Option<&str>,
    now: i64,
) -> anyhow::Result<ModelReport> {
    let days = days.clamp(1, MAX_DAYS);
    let since = now - i64::from(days) * 86_400;
    let models = store
        .model_report(since, repo)?
        .into_iter()
        .map(|stats| {
            let bench = benchmark(&stats.model);
            ReportEntry {
                completion_rate: stats.completion_rate(),
                tool_error_rate: stats.tool_error_rate(),
                benchmark_intelligence: bench.map(|b| b.0),
                benchmark_coding: bench.map(|b| b.1),
                stats,
            }
        })
        .collect();
    Ok(ModelReport {
        days,
        repo: repo.map(str::to_string),
        generated_at: now,
        models,
    })
}

pub(crate) fn pct(rate: Option<f64>) -> String {
    rate.map_or("—".to_string(), |r| format!("{:.0}%", r * 100.0))
}

pub(crate) fn latency(ms: Option<i64>) -> String {
    match ms {
        None => "—".to_string(),
        Some(ms) if ms < 1000 => format!("{ms}ms"),
        Some(ms) => format!("{:.1}s", ms as f64 / 1000.0),
    }
}

pub(crate) fn cost(usd: Option<f64>) -> String {
    match usd {
        None => "—".to_string(),
        Some(c) if c < 0.00005 => "$0".to_string(),
        Some(c) if c < 0.01 => format!("${c:.4}"),
        Some(c) if c < 0.1 => format!("${c:.3}"),
        Some(c) => format!("${c:.2}"),
    }
}

/// Fixed-width columns shared by the CLI and the TUI overlay.
pub(crate) fn render(report: &ModelReport) -> String {
    let scope = report
        .repo
        .as_deref()
        .map_or("all repos".to_string(), |r| format!("repo {r}"));
    let mut out = format!("model report card — last {} days, {scope}\n", report.days);
    if report.models.is_empty() {
        out.push_str("no model activity recorded in this window\n");
        return out;
    }
    out.push_str(&format!(
        "{:<44} {:>6} {:>6} {:>6} {:>6} {:>8} {:>8} {:>7} {:>6}\n",
        "model", "calls", "done", "failed", "f/over", "latency", "cost", "tool-err", "bench"
    ));
    for e in &report.models {
        out.push_str(&row_line(e));
        out.push('\n');
    }
    out.push_str(
        "done = answered; f/over = failed, then the mesh moved to another hop; cost and latency \
         are medians per call; bench = intelligence score\n",
    );
    out
}

pub(crate) fn row_line(e: &ReportEntry) -> String {
    let s = &e.stats;
    format!(
        "{:<44} {:>6} {:>6} {:>6} {:>6} {:>8} {:>8} {:>8} {:>6}",
        truncate(&s.model, 44),
        s.calls,
        pct(e.completion_rate),
        s.failed,
        s.failed_over,
        latency(s.median_latency_ms),
        cost(s.median_cost_usd),
        pct(e.tool_error_rate),
        e.benchmark_intelligence
            .map_or("—".to_string(), |b| format!("{b:.0}")),
    )
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max - 1).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_rates_latency_and_cost() {
        assert_eq!(pct(Some(0.934)), "93%");
        assert_eq!(pct(None), "—");
        assert_eq!(latency(Some(850)), "850ms");
        assert_eq!(latency(Some(2400)), "2.4s");
        assert_eq!(cost(Some(0.0)), "$0");
        assert_eq!(cost(Some(0.0042)), "$0.0042");
        assert_eq!(cost(Some(0.015)), "$0.015");
        assert_eq!(cost(Some(0.25)), "$0.25");
    }

    #[test]
    fn empty_window_says_so_and_rows_pair_with_benchmarks() {
        let store = Store::open_in_memory().unwrap();
        let report = build(&store, &|_| None, 30, None, 1_000_000).unwrap();
        assert!(render(&report).contains("no model activity"));

        let sid = store.create_session("/r", "auto").unwrap();
        for ok in [true, true, false] {
            store
                .record_mesh_outcome(&forge_store::MeshOutcome {
                    session_id: sid.clone(),
                    model: "p::good".into(),
                    tier: forge_types::TaskTier::Standard,
                    started_at: 999_000,
                    completed_at: 999_000,
                    latency_ms: 1500,
                    outcome: if ok { "success" } else { "failure" }.into(),
                    error_kind: None,
                    failover_hop: 0,
                    tool_calls: 0,
                    verified_completion: ok,
                })
                .unwrap();
        }
        let report = build(
            &store,
            &|m| (m == "p::good").then_some((61.0, 55.0)),
            30,
            None,
            1_000_000,
        )
        .unwrap();
        let text = render(&report);
        assert!(text.contains("p::good") && text.contains("67%") && text.contains("1.5s"));
        assert_eq!(report.models[0].benchmark_intelligence, Some(61.0));
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["models"][0]["calls"], 3);
        assert_eq!(
            json["models"][0]["completion_rate"]
                .as_f64()
                .unwrap()
                .round(),
            1.0
        );
    }
}
