//! `GET {base}/api/sessions/{id}/spend` — what one session spent, for the Spend dock.
//!
//! Per model, from the same ledger the usage page reads (`/api/usage?session=`), so the totals
//! agree with it: every provider call of the session, side calls included. Each subagent child
//! session is listed beside it, not folded into the total. `estimate_usd` prices every call at
//! the model's list rate, so subscription and free routes (which record $0) still show what the
//! work would cost; it is `null` when a call used a model with no tracked price.

use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use forge_mesh::pricing::Pricing;
use forge_store::{ModelUsageRow, Store};

use crate::serve::{err_response, json_response, DaemonState};

const SUBAGENT_LABEL_CHARS: usize = 80;

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new().route(
        &format!("{base}/api/sessions/{{id}}/spend"),
        get(session_spend),
    )
}

#[derive(Debug, Default, PartialEq, serde::Serialize)]
pub(crate) struct SpendTotals {
    input_tokens: u64,
    cached_input_tokens: Option<u64>,
    output_tokens: u64,
    calls: u64,
    cost_usd: f64,
    estimate_usd: Option<f64>,
}

#[derive(Debug, PartialEq, serde::Serialize)]
pub(crate) struct SpendModel {
    model: String,
    #[serde(flatten)]
    totals: SpendTotals,
}

#[derive(Debug, PartialEq, serde::Serialize)]
pub(crate) struct SpendSubagent {
    session_id: String,
    label: String,
    model: Option<String>,
    #[serde(flatten)]
    totals: SpendTotals,
}

#[derive(Debug, PartialEq, serde::Serialize)]
pub(crate) struct SpendResponse {
    session_id: String,
    total: SpendTotals,
    models: Vec<SpendModel>,
    subagents: Vec<SpendSubagent>,
}

fn add_cached(total: Option<u64>, part: Option<u64>) -> Option<u64> {
    match (total, part) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
    }
}

fn add_row(totals: &mut SpendTotals, row: &ModelUsageRow, pricing: &Pricing) {
    totals.input_tokens += row.usage.input_tokens;
    totals.output_tokens += row.usage.output_tokens;
    totals.cached_input_tokens =
        add_cached(totals.cached_input_tokens, row.usage.cached_input_tokens);
    totals.calls += row.calls;
    totals.cost_usd += row.usage.cost_usd;
    let part = forge_core::row_notional_cost(row, pricing);
    totals.estimate_usd = match (totals.estimate_usd, part) {
        (Some(sum), Some(part)) => Some(sum + part),
        _ => None,
    };
}

pub(crate) fn totals_of(rows: &[ModelUsageRow], pricing: &Pricing) -> SpendTotals {
    let mut totals = SpendTotals {
        estimate_usd: Some(0.0),
        ..SpendTotals::default()
    };
    for row in rows {
        add_row(&mut totals, row, pricing);
    }
    totals
}

pub(crate) fn by_model(rows: &[ModelUsageRow], pricing: &Pricing) -> Vec<SpendModel> {
    let mut models: Vec<SpendModel> = Vec::new();
    for row in rows {
        let name = row.model.clone().unwrap_or_else(|| "unknown".to_string());
        let index = match models.iter().position(|m| m.model == name) {
            Some(index) => index,
            None => {
                models.push(SpendModel {
                    model: name,
                    totals: SpendTotals {
                        estimate_usd: Some(0.0),
                        ..SpendTotals::default()
                    },
                });
                models.len() - 1
            }
        };
        add_row(&mut models[index].totals, row, pricing);
    }
    models.sort_by(|a, b| {
        let tokens = |m: &SpendModel| m.totals.input_tokens + m.totals.output_tokens;
        tokens(b)
            .cmp(&tokens(a))
            .then_with(|| a.model.cmp(&b.model))
    });
    models
}

fn label_of(title: Option<&str>, task: &str) -> String {
    let source = title.filter(|t| !t.trim().is_empty()).unwrap_or(task);
    let line = source.lines().next().unwrap_or("").trim();
    if line.chars().count() <= SUBAGENT_LABEL_CHARS {
        return line.to_string();
    }
    let head: String = line.chars().take(SUBAGENT_LABEL_CHARS - 1).collect();
    format!("{}…", head.trim_end())
}

pub(crate) fn spend_for(
    store: &Store,
    session_id: &str,
    pricing: &Pricing,
) -> anyhow::Result<SpendResponse> {
    let rows = store.session_usage_by_model(session_id)?;
    let mut subagents = Vec::new();
    for child in store.child_index(session_id)? {
        let child_rows = store.session_usage_by_model(&child.id)?;
        if child_rows.is_empty() {
            continue;
        }
        subagents.push(SpendSubagent {
            label: label_of(child.title.as_deref(), &child.task),
            model: child.model,
            totals: totals_of(&child_rows, pricing),
            session_id: child.id,
        });
    }
    Ok(SpendResponse {
        session_id: session_id.to_string(),
        total: totals_of(&rows, pricing),
        models: by_model(&rows, pricing),
        subagents,
    })
}

async fn session_spend(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<SpendResponse>> {
        if !store.session_exists(&id)? {
            return Ok(None);
        }
        let pricing = Pricing::from_config(&forge_config::load().unwrap_or_default());
        spend_for(&store, &id, &pricing).map(Some)
    })
    .await;
    match result {
        Ok(Ok(Some(body))) => json_response(&body),
        Ok(Ok(None)) => err_response(axum::http::StatusCode::NOT_FOUND, "no such session"),
        Ok(Err(error)) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("spend unavailable: {error}"),
        ),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "spend unavailable",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::Usage;

    fn row(
        model: &str,
        zero_cost: bool,
        calls: u64,
        input: u64,
        output: u64,
        cost: f64,
    ) -> ModelUsageRow {
        ModelUsageRow {
            model: Some(model.to_string()),
            zero_cost,
            calls,
            usage: Usage {
                input_tokens: input,
                output_tokens: output,
                cost_usd: cost,
                ..Usage::default()
            },
        }
    }

    fn pricing() -> Pricing {
        Pricing::from_config_with_fetched(
            &forge_config::Config::default(),
            vec![("kimi::k3".to_string(), 0.003, 0.015, None)],
        )
    }

    #[test]
    fn a_model_billed_and_unbilled_is_one_row_with_both_prices() {
        let rows = vec![
            row("kimi::k3", false, 2, 1_000, 100, 0.5),
            row("kimi::k3", true, 3, 1_000, 1_000, 0.0),
            row("groq::small", false, 1, 50, 5, 0.01),
        ];
        let models = by_model(&rows, &pricing());
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].model, "kimi::k3", "busiest model first");
        assert_eq!(models[0].totals.calls, 5);
        assert_eq!(models[0].totals.input_tokens, 2_000);
        assert!((models[0].totals.cost_usd - 0.5).abs() < 1e-9);
        let estimate = models[0]
            .totals
            .estimate_usd
            .expect("both halves are priced");
        assert!((estimate - (0.5 + 0.003 + 0.015)).abs() < 1e-9);
    }

    #[test]
    fn the_total_is_the_sum_of_the_model_rows() {
        let rows = vec![
            row("kimi::k3", false, 2, 1_000, 100, 0.5),
            row("groq::small", false, 1, 50, 5, 0.01),
        ];
        let total = totals_of(&rows, &pricing());
        let models = by_model(&rows, &pricing());
        assert_eq!(
            total.input_tokens,
            models.iter().map(|m| m.totals.input_tokens).sum::<u64>()
        );
        assert_eq!(
            total.output_tokens,
            models.iter().map(|m| m.totals.output_tokens).sum::<u64>()
        );
        assert_eq!(total.calls, 3);
    }

    #[test]
    fn an_unpriced_free_call_leaves_the_estimate_unknown_instead_of_zero() {
        let rows = vec![row("ollama::qwen", true, 1, 10, 10, 0.0)];
        assert_eq!(totals_of(&rows, &pricing()).estimate_usd, None);
        assert_eq!(by_model(&rows, &pricing())[0].totals.estimate_usd, None);
    }

    #[test]
    fn the_totals_match_the_usage_page_for_the_same_session() {
        use forge_types::Role;
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        let usage = |input, output, cost| Usage {
            input_tokens: input,
            output_tokens: output,
            cost_usd: cost,
            ..Usage::default()
        };
        for (seq, (model, input, output, cost)) in [
            ("kimi::k3", 120, 12, 0.4),
            ("groq::small", 80, 8, 0.0),
            ("kimi::k3", 40, 4, 0.1),
        ]
        .into_iter()
        .enumerate()
        {
            let message = store
                .add_message(&sid, seq as i64 + 1, Role::Assistant, "a", Some(model))
                .unwrap();
            store
                .record_usage(&sid, &message, &usage(input, output, cost), None)
                .unwrap();
        }
        let response = spend_for(&store, &sid, &pricing()).unwrap();
        let page = store.usage_by_provider_for_session(&sid).unwrap();
        assert_eq!(
            response.total.input_tokens,
            page.iter().map(|r| r.input_tokens).sum::<u64>()
        );
        assert_eq!(
            response.total.output_tokens,
            page.iter().map(|r| r.output_tokens).sum::<u64>()
        );
        assert!(
            (response.total.cost_usd - page.iter().map(|r| r.cost_usd).sum::<f64>()).abs() < 1e-9
        );
        assert_eq!(response.models.len(), 2);
        assert!(response.subagents.is_empty());
    }

    #[test]
    fn an_empty_session_totals_zero_with_a_known_estimate() {
        let total = totals_of(&[], &pricing());
        assert_eq!((total.input_tokens, total.calls), (0, 0));
        assert_eq!(total.estimate_usd, Some(0.0));
    }

    #[test]
    fn a_subagent_is_labelled_by_its_title_else_the_first_line_of_its_task() {
        assert_eq!(label_of(Some("Fix the build"), "task"), "Fix the build");
        assert_eq!(
            label_of(None, "Read the parser\nthen more"),
            "Read the parser"
        );
        assert_eq!(label_of(Some("  "), "from task"), "from task");
        assert!(label_of(None, &"x".repeat(200)).chars().count() <= SUBAGENT_LABEL_CHARS);
    }
}
