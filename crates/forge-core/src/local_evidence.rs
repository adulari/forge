//! Local routing evidence read from the store: the outcome calibration and the measured cost per
//! solve from recorded bench runs (docs/features/mesh-routing.md §4.7a). Every composition root
//! (sessions, `forge mesh`, the API server) attaches both through here so they score alike.

use std::collections::HashMap;

use forge_mesh::ModelCatalog;

/// Attach both kinds of local evidence to a freshly discovered catalog. Store errors yield no
/// evidence: both signals are bounded tie-breakers, so missing data must not fail a session.
pub fn apply_local_evidence(catalog: ModelCatalog, store: &forge_store::Store) -> ModelCatalog {
    let calibration = store
        .model_outcome_calibration()
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            (
                row.model,
                forge_mesh::RuntimeCalibration {
                    samples: row.samples,
                    success_rate: row.success_rate,
                    mean_latency_ms: row.mean_latency_ms,
                },
            )
        })
        .collect();
    catalog
        .with_runtime_calibration(calibration)
        .with_measured_solve_cost(measured_solve_cost(store))
}

/// Every model with recorded bench runs, pooled.
fn measured_solve_cost(
    store: &forge_store::Store,
) -> HashMap<String, forge_mesh::MeasuredSolveCost> {
    store
        .bench_solve_totals()
        .unwrap_or_default()
        .into_iter()
        .map(|t| {
            (
                t.model,
                forge_mesh::MeasuredSolveCost {
                    attempts: t.attempts,
                    solved: t.solved,
                    cost_usd: t.cost_usd,
                },
            )
        })
        .collect()
}
