//! The Model Mesh (ADR-0006): classify a task, then route it to the cheapest configured
//! model that can handle it — adjusting for the remaining budget. Routing is deterministic
//! and adds no model calls. The [`Router`] trait keeps a smarter (e.g. LLM-based)
//! classifier pluggable later without changing callers.

use async_trait::async_trait;
use forge_config::Config;
#[cfg(test)]
use forge_types::Message;
use forge_types::{EffortLevel, ModelHealth, ProjectContext, SubscriptionQuota, TaskTier};

use classification::score_prompt;
#[cfg(test)]
use classification::{is_code_heavy, is_multistep};

pub mod bench;
pub mod capability;
pub mod catalog;
mod classification;
mod context;
#[cfg(test)]
mod doc_sync;
pub mod explain;
pub mod pricing;

pub use bench::{BenchScore, BenchmarkScores};
pub use catalog::{
    CatalogStats, ConserveDecision, ModelCatalog, ModelInfo, ProviderGroup, RuntimeCalibration,
    ScoreRow,
};
pub use classification::{max_tier, RouteHints};
#[cfg(test)]
use context::COMPACTION_SUMMARY_PREFIX;
pub use context::{RoutingContext, SessionAffinity};
pub use explain::{CandidateRow, ProviderQuotaView, RoutingExplanation};

/// Live budget context the router considers when choosing a tier. Carries daily, weekly, and
/// monthly axes (FR-5); the stricter of all configured axes governs.
#[derive(Debug, Clone, Copy)]
pub struct BudgetState {
    pub spent_today_usd: f64,
    pub daily_cap_usd: Option<f64>,
    pub spent_week_usd: f64,
    pub weekly_cap_usd: Option<f64>,
    pub spent_month_usd: f64,
    pub monthly_cap_usd: Option<f64>,
    /// Fraction of a cap at which to warn (e.g. 0.8 = 80%).
    pub warn_fraction: f64,
    /// Minimum context window (in tokens) required for the selected model. When set, models whose
    /// known window is smaller than this value are skipped during routing.
    pub min_context_tokens: Option<u32>,
}

impl Default for BudgetState {
    fn default() -> Self {
        Self {
            spent_today_usd: 0.0,
            daily_cap_usd: None,
            spent_week_usd: 0.0,
            weekly_cap_usd: None,
            spent_month_usd: 0.0,
            monthly_cap_usd: None,
            warn_fraction: DEFAULT_WARN_FRACTION,
            min_context_tokens: None,
        }
    }
}

#[cfg(test)]
mod session_affinity_tests {
    use super::*;
    use forge_types::QuotaStatus;
    use std::collections::{HashMap, HashSet};

    const BEST: &str = "codex-oauth::affinity-best";
    const WARM: &str = "anthropic::affinity-warm";
    const WEAK: &str = "openrouter::affinity-weak";

    fn affinity_router(warm_coding: f64) -> HeuristicRouter {
        let mut bench = BenchmarkScores::new();
        bench.insert("affinity best", 60.0, 77.4);
        bench.insert("affinity warm", 59.0, warm_coding);
        bench.insert("affinity weak", 50.0, 68.0);
        let catalog = ModelCatalog::new(vec![BEST.into(), WARM.into(), WEAK.into()])
            .with_benchmarks(Some(bench));
        let mut config = Config::default();
        config.mesh.auto_discover = true;
        HeuristicRouter::new(config)
            .with_catalog(catalog)
            .with_availability(|_| true)
    }

    fn dependent_context(model: &str, tier: TaskTier, prefix_tokens: u64) -> RoutingContext {
        RoutingContext::from_messages(&[
            Message::user(
                "Diagnose and fix the concurrent reservation algorithm, prove correctness, and \
                 run the complete stress suite.",
            ),
            Message::assistant("The implementation is in progress; continue with verification."),
        ])
        .with_session_affinity(
            Some(SessionAffinity {
                model: model.into(),
                tier,
                code_heavy: true,
            }),
            prefix_tokens,
        )
    }

    async fn contextual_route(
        router: &HeuristicRouter,
        context: &RoutingContext,
        health: &ModelHealth,
        quota: &SubscriptionQuota,
        tier: TaskTier,
        min_context_tokens: Option<u32>,
    ) -> RoutingDecision {
        router
            .route_contextual(
                "Continue the current implementation and verification.",
                false,
                BudgetState {
                    min_context_tokens,
                    ..BudgetState::default()
                },
                health,
                quota,
                Some(tier),
                None,
                &ProjectContext::default(),
                context,
            )
            .await
    }

    #[tokio::test]
    async fn first_complex_task_still_uses_strongest_quality_anchor() {
        let decision = affinity_router(76.8)
            .route_contextual(
                "Diagnose and fix the concurrent reservation algorithm, prove correctness, and \
                 run the complete stress suite.",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                Some(TaskTier::Complex),
                None,
                &ProjectContext::default(),
                &RoutingContext::default(),
            )
            .await;

        assert_eq!(decision.model, BEST, "{}", decision.rationale);
        assert!(!decision.rationale.contains("session affinity"));
    }

    #[tokio::test]
    async fn first_complex_task_uses_faster_calibrated_member_of_top_quality_band() {
        let mut bench = BenchmarkScores::new();
        bench.insert("affinity best", 60.0, 77.4);
        bench.insert("affinity warm", 59.0, 76.8);
        let catalog = ModelCatalog::new(vec![BEST.into(), WARM.into()])
            .with_benchmarks(Some(bench))
            .with_runtime_calibration(HashMap::from([
                (
                    BEST.to_string(),
                    RuntimeCalibration {
                        samples: 40,
                        success_rate: 0.99,
                        mean_latency_ms: 20_000.0,
                    },
                ),
                (
                    WARM.to_string(),
                    RuntimeCalibration {
                        samples: 40,
                        success_rate: 0.99,
                        mean_latency_ms: 10_000.0,
                    },
                ),
            ]));
        let mut config = Config::default();
        config.mesh.auto_discover = true;
        let router = HeuristicRouter::new(config)
            .with_catalog(catalog)
            .with_availability(|_| true);
        let hints = RouteHints::from_context(
            "Implement and fix the concurrent reservation algorithm, prove correctness, \
             and run the complete stress suite.",
            &RoutingContext::default(),
        );
        assert!(hints.code_heavy);
        assert!(!hints.continuation);
        assert_eq!(
            router
                .catalog
                .as_ref()
                .and_then(|catalog| dependable_calibrated_latency(catalog, WARM)),
            Some(10_000.0)
        );

        let decision = router
            .route_contextual(
                "Implement and fix the concurrent reservation algorithm, prove correctness, \
                 and run the complete stress suite.",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                Some(TaskTier::Complex),
                None,
                &ProjectContext::default(),
                &RoutingContext::default(),
            )
            .await;

        assert_eq!(decision.model, WARM, "{}", decision.rationale);
        assert!(decision.rationale.contains(
            "calibrated low-latency quality anchor: anthropic::affinity-warm is 0.60 points"
        ));
        assert!(decision.rationale.contains("10000ms faster per model call"));
    }

    #[tokio::test]
    async fn dependent_continuation_prefers_warm_model_inside_quality_band() {
        let context = dependent_context(WARM, TaskTier::Complex, 48_000);
        let decision = contextual_route(
            &affinity_router(76.8),
            &context,
            &ModelHealth::default(),
            &SubscriptionQuota::default(),
            TaskTier::Complex,
            None,
        )
        .await;

        assert_eq!(decision.model, WARM, "{}", decision.rationale);
        assert_eq!(decision.fallbacks.first().map(String::as_str), Some(BEST));
        assert!(decision.rationale.contains("quality gap 0.60"));
        assert!(decision.rationale.contains("48000-token cold prefix"));
    }

    #[test]
    fn affinity_rationale_reports_warm_quality_advantage_without_clamping_it() {
        assert_eq!(
            describe_affinity_quality(Some(-6.0), 1.0),
            "warm model quality advantage 6.00"
        );
    }

    #[tokio::test]
    async fn long_adversarial_continuation_uses_tighter_quality_band() {
        let context = dependent_context(WARM, TaskTier::Complex, 48_000);
        let decision = affinity_router(76.8)
            .route_contextual(
                "Continue from the current implementation. A small green suite is not enough: \
                 add adversarial coverage for 100 concurrent requests against one unit of stock \
                 and for many concurrent duplicate request IDs. Diagnose and fix any race or \
                 idempotency weakness you expose, preserve the public API, and rerun the complete \
                 suite.",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                Some(TaskTier::Complex),
                None,
                &ProjectContext::default(),
                &context,
            )
            .await;

        assert_eq!(decision.model, BEST, "{}", decision.rationale);
        assert!(decision
            .rationale
            .contains("0.50-point quality-critical band"));
        assert!(decision.rationale.contains("48000 tokens"));
    }

    #[tokio::test]
    async fn meaningful_quality_gap_overrides_affinity() {
        let context = dependent_context(WARM, TaskTier::Complex, 48_000);
        let decision = contextual_route(
            &affinity_router(70.0),
            &context,
            &ModelHealth::default(),
            &SubscriptionQuota::default(),
            TaskTier::Complex,
            None,
        )
        .await;

        assert_eq!(decision.model, BEST, "{}", decision.rationale);
        assert!(decision.rationale.contains("quality advantage"));
    }

    #[test]
    fn significant_quality_gap_restores_anchor_after_transient_failover() {
        let router = affinity_router(71.4);
        let context = dependent_context(WARM, TaskTier::Complex, 51_061);
        let decision = router.apply_session_affinity(
            RoutingDecision {
                tier: TaskTier::Complex,
                model: WARM.into(),
                rationale: "normal continuation order".into(),
                fallbacks: vec![BEST.into(), WEAK.into()],
                pinned: false,
            },
            &context,
            RouteHints {
                code_heavy: true,
                seed: 0,
                continuation: true,
                quality_critical: true,
            },
            &ModelHealth::default(),
            &SubscriptionQuota::default(),
            None,
            None,
            false,
        );

        assert_eq!(decision.model, BEST, "{}", decision.rationale);
        assert_eq!(decision.fallbacks.first().map(String::as_str), Some(WARM));
        assert!(decision.rationale.contains(
            "affinity-best offers measured quality advantage 6.00, exceeding 0.50-point \
             quality-critical band"
        ));
        assert!(decision.rationale.contains("51061 tokens"));
    }

    #[test]
    fn missing_quality_evidence_never_creates_speculative_stickiness() {
        let router = HeuristicRouter::new(Config::default()).with_availability(|_| true);
        let context = dependent_context(WARM, TaskTier::Complex, 48_000);
        let decision = router.apply_session_affinity(
            RoutingDecision {
                tier: TaskTier::Complex,
                model: BEST.into(),
                rationale: "normal mesh order".into(),
                fallbacks: vec![WARM.into()],
                pinned: false,
            },
            &context,
            RouteHints {
                code_heavy: true,
                seed: 0,
                continuation: true,
                quality_critical: false,
            },
            &ModelHealth::default(),
            &SubscriptionQuota::default(),
            None,
            None,
            false,
        );

        assert_eq!(decision.model, BEST);
        assert!(decision
            .rationale
            .contains("lack comparable measured quality evidence"));
    }

    #[tokio::test]
    async fn health_quota_context_and_task_class_override_affinity() {
        let router = affinity_router(76.8);
        let context = dependent_context(WARM, TaskTier::Complex, 48_000);

        let unhealthy = contextual_route(
            &router,
            &context,
            &ModelHealth::new(HashSet::from([WARM.to_string()])),
            &SubscriptionQuota::default(),
            TaskTier::Complex,
            None,
        )
        .await;
        assert_eq!(unhealthy.model, BEST);
        assert!(unhealthy.rationale.contains("unhealthy or degraded"));

        let pressured_quota = SubscriptionQuota::new(HashMap::from([(
            "anthropic".to_string(),
            QuotaStatus::Warning,
        )]));
        let pressured = contextual_route(
            &router,
            &context,
            &ModelHealth::default(),
            &pressured_quota,
            TaskTier::Complex,
            None,
        )
        .await;
        assert_eq!(pressured.model, BEST);
        assert!(pressured.rationale.contains("quota under pressure"));

        let context_limited = affinity_router(76.8).with_context_windows(HashMap::from([
            (BEST.to_string(), 200_000),
            (WARM.to_string(), 16_000),
            (WEAK.to_string(), 200_000),
        ]));
        let exhausted = contextual_route(
            &context_limited,
            &context,
            &ModelHealth::default(),
            &SubscriptionQuota::default(),
            TaskTier::Complex,
            Some(32_000),
        )
        .await;
        assert_eq!(exhausted.model, BEST);
        assert!(exhausted.rationale.contains("context window exhausted"));

        let changed_context = dependent_context(WARM, TaskTier::Standard, 48_000);
        let changed = contextual_route(
            &router,
            &changed_context,
            &ModelHealth::default(),
            &SubscriptionQuota::default(),
            TaskTier::Complex,
            None,
        )
        .await;
        assert_eq!(changed.model, BEST);
        assert!(changed.rationale.contains("task-class change"));

        let code_class_changed = RoutingContext::from_messages(&[
            Message::user(
                "Diagnose and fix the concurrent reservation algorithm, prove correctness.",
            ),
            Message::assistant("The implementation is in progress."),
        ])
        .with_session_affinity(
            Some(SessionAffinity {
                model: WARM.into(),
                tier: TaskTier::Complex,
                code_heavy: false,
            }),
            48_000,
        );
        let code_changed = contextual_route(
            &router,
            &code_class_changed,
            &ModelHealth::default(),
            &SubscriptionQuota::default(),
            TaskTier::Complex,
            None,
        )
        .await;
        assert_eq!(code_changed.model, BEST);
        assert!(code_changed.rationale.contains("task-class change"));
    }

    #[tokio::test]
    async fn unrelated_task_and_new_session_do_not_inherit_affinity() {
        let router = affinity_router(76.8);
        let context = dependent_context(WARM, TaskTier::Complex, 48_000);
        let unrelated = router
            .route_contextual(
                "Design a new unrelated distributed scheduler from scratch.",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                Some(TaskTier::Complex),
                None,
                &ProjectContext::default(),
                &context,
            )
            .await;
        assert_eq!(unrelated.model, BEST, "{}", unrelated.rationale);
        assert!(!unrelated.rationale.contains("session affinity retained"));

        let new_session = contextual_route(
            &router,
            &RoutingContext::from_messages(&[
                Message::user("Diagnose the existing concurrency bug."),
                Message::assistant("Initial diagnosis complete."),
            ]),
            &ModelHealth::default(),
            &SubscriptionQuota::default(),
            TaskTier::Complex,
            None,
        )
        .await;
        assert_eq!(new_session.model, BEST, "{}", new_session.rationale);
        assert!(!new_session.rationale.contains("session affinity"));
    }

    #[test]
    fn detailed_continuations_use_general_references_not_benchmark_sentences() {
        let context = RoutingContext::from_messages(&[
            Message::user(
                "Implement the asynchronous inventory reservation service with atomic concurrency \
                 and verify every repository test.",
            ),
            Message::assistant("The initial implementation is complete."),
        ]);

        for prompt in [
            "Audit the current implementation for concurrency defects and correct what remains.",
            "Review the whole solution for state drift after repeated operations.",
            "Inspect the actual diff and finish the remaining work with fresh verification.",
            "Complete the task list and summarize the final state.",
        ] {
            assert!(context.is_dependent_turn(prompt), "{prompt:?}");
        }
        for prompt in [
            "Design a new unrelated distributed scheduler from scratch.",
            "New task: review the whole solution in the sibling payments repository.",
            "Create a task list for a new deployment project.",
        ] {
            assert!(!context.is_dependent_turn(prompt), "{prompt:?}");
        }
    }

    #[tokio::test]
    async fn calibrated_latency_gain_can_pay_for_cold_start() {
        let mut bench = BenchmarkScores::new();
        bench.insert("affinity best", 60.0, 77.4);
        bench.insert("affinity warm", 59.0, 76.8);
        let catalog = ModelCatalog::new(vec![BEST.into(), WARM.into()])
            .with_benchmarks(Some(bench))
            .with_runtime_calibration(HashMap::from([
                (
                    BEST.to_string(),
                    RuntimeCalibration {
                        samples: 40,
                        success_rate: 0.99,
                        mean_latency_ms: 5_000.0,
                    },
                ),
                (
                    WARM.to_string(),
                    RuntimeCalibration {
                        samples: 40,
                        success_rate: 0.99,
                        mean_latency_ms: 60_000.0,
                    },
                ),
            ]));
        let mut config = Config::default();
        config.mesh.auto_discover = true;
        let router = HeuristicRouter::new(config)
            .with_catalog(catalog)
            .with_availability(|_| true);
        let decision = contextual_route(
            &router,
            &dependent_context(WARM, TaskTier::Complex, 40_000),
            &ModelHealth::default(),
            &SubscriptionQuota::default(),
            TaskTier::Complex,
            None,
        )
        .await;

        assert_eq!(decision.model, BEST, "{}", decision.rationale);
        assert!(decision
            .rationale
            .contains("calibrated latency advantage 55000ms"));
        assert!(decision.rationale.contains("10000ms cold-prefix estimate"));
    }

    #[tokio::test]
    async fn measured_runtime_degradation_overrides_affinity() {
        let mut bench = BenchmarkScores::new();
        bench.insert("affinity best", 60.0, 77.4);
        bench.insert("affinity warm", 59.0, 76.8);
        let catalog = ModelCatalog::new(vec![BEST.into(), WARM.into()])
            .with_benchmarks(Some(bench))
            .with_runtime_calibration(HashMap::from([(
                WARM.to_string(),
                RuntimeCalibration {
                    samples: 40,
                    success_rate: 0.75,
                    mean_latency_ms: 10_000.0,
                },
            )]));
        let mut config = Config::default();
        config.mesh.auto_discover = true;
        let router = HeuristicRouter::new(config)
            .with_catalog(catalog)
            .with_availability(|_| true);
        let decision = contextual_route(
            &router,
            &dependent_context(WARM, TaskTier::Complex, 40_000),
            &ModelHealth::default(),
            &SubscriptionQuota::default(),
            TaskTier::Complex,
            None,
        )
        .await;

        assert_eq!(decision.model, BEST, "{}", decision.rationale);
        assert!(decision.rationale.contains("runtime reliability degraded"));
    }

    #[tokio::test]
    async fn contextual_route_inspection_matches_execution_order() {
        let router = affinity_router(76.8);
        let context = dependent_context(WARM, TaskTier::Complex, 48_000);
        let decision = contextual_route(
            &router,
            &context,
            &ModelHealth::default(),
            &SubscriptionQuota::default(),
            TaskTier::Complex,
            None,
        )
        .await;
        let explanation = router.explain_contextual_classified(
            "Continue the current implementation and verification.",
            TaskTier::Complex,
            vec!["test classifier".into()],
            BudgetState::default(),
            &ModelHealth::default(),
            &SubscriptionQuota::default(),
            None,
            &context,
        );

        assert_eq!(explanation.pick, decision.model);
        assert_eq!(explanation.fallbacks, decision.fallbacks);
        assert_eq!(
            explanation
                .candidates
                .first()
                .map(|row| row.row.model.as_str()),
            Some(decision.model.as_str())
        );
        assert!(explanation
            .candidates
            .first()
            .is_some_and(|row| row.selected));
        let affinity_reason = "session affinity retained anthropic::affinity-warm: dependent \
                               continuation, quality gap 0.60 within 1.00-point band; avoiding \
                               estimated 48000-token cold prefix";
        assert!(decision.rationale.contains(affinity_reason));
        assert!(explanation.rationale.contains(affinity_reason));
    }

    #[tokio::test]
    async fn retained_six_turn_replay_recognizes_long_continuations_and_avoids_luna() {
        const TERRA: &str = "codex-oauth::gpt-5.6-terra";
        const SOL: &str = "codex-oauth::gpt-5.6-sol";
        const LUNA: &str = "codex-oauth::gpt-5.6-luna";

        let mut bench = BenchmarkScores::new();
        bench.insert("gpt-5.6-sol", 58.9, 77.4);
        bench.insert("gpt-5.6-terra", 55.0, 76.7);
        bench.insert("gpt-5.6-luna", 51.2, 71.4);
        let mut config = Config::default();
        config.mesh.auto_discover = true;
        let router = HeuristicRouter::new(config)
            .with_catalog(
                ModelCatalog::new(vec![SOL.into(), TERRA.into(), LUNA.into()])
                    .with_benchmarks(Some(bench))
                    .with_runtime_calibration(HashMap::from([
                        (
                            SOL.to_string(),
                            RuntimeCalibration {
                                samples: 120,
                                success_rate: 0.958,
                                mean_latency_ms: 13_459.8,
                            },
                        ),
                        (
                            TERRA.to_string(),
                            RuntimeCalibration {
                                samples: 54,
                                success_rate: 0.963,
                                mean_latency_ms: 10_416.4,
                            },
                        ),
                        (
                            LUNA.to_string(),
                            RuntimeCalibration {
                                samples: 120,
                                success_rate: 1.0,
                                mean_latency_ms: 7_296.2,
                            },
                        ),
                    ])),
            )
            .with_availability(|_| true);
        let base_routes = [SOL, SOL, LUNA, SOL, SOL, SOL];
        let old_completed_routes = [TERRA, TERRA, LUNA, SOL, SOL, SOL];
        let prefix_tokens = [0, 12_000, 30_000, 55_000, 75_000, 95_000];
        let prompts = [
            "Benchmark integrity: solve only from this checked-out repository and these prompts. \
             Fix this repository's asynchronous inventory reservation service completely. Read \
             README.md and the full test suite, diagnose the cross-file bugs, implement the \
             contract without changing public method signatures, and run all tests.",
            "Continue from the current implementation. A small green suite is not enough: add \
             adversarial coverage for 100 concurrent requests against one unit of stock and for \
             many concurrent duplicate request IDs. Diagnose and fix any race or idempotency \
             weakness you expose, preserve the public API, and rerun the complete suite.",
            "Now scrutinize rollback and cancellation across the work already done. Exercise an \
             injected storage failure, repeated cancellation, and concurrent reserve/cancel \
             interleavings. Strengthen tests where useful, fix real defects rather than papering \
             over them, and verify inventory is restored exactly once.",
            "Review the whole multi-file solution for long-running-session mistakes: stale \
             assumptions, ordering drift, lock-scope errors, exception mismatches, or state that \
             can become inconsistent after many operations. Make the smallest robust corrections, \
             then run the full tests repeatedly enough to catch scheduling-sensitive failures.",
            "Do a skeptical final code review against every README invariant and the task list. \
             Inspect the actual diff, run the suite from a fresh interpreter, and fix anything \
             incomplete, overcomplicated, flaky, or unverified. Do not stop at a prose review when \
             a code or test correction is needed.",
            "Finish the goal end to end. Run one final complete verification, confirm no tests \
             were weakened and no public signatures changed, ensure the task list is fully done, \
             then give a concise evidence-based summary of the final state.",
        ];
        let first = router
            .route_contextual(
                prompts[0],
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                Some(TaskTier::Complex),
                None,
                &ProjectContext::default(),
                &RoutingContext::default(),
            )
            .await;
        assert_eq!(first.model, TERRA, "{}", first.rationale);
        assert!(first
            .rationale
            .contains("calibrated low-latency quality anchor"));

        let mut affinity = SessionAffinity {
            model: first.model.clone(),
            tier: TaskTier::Complex,
            code_heavy: true,
        };
        let mut replayed = vec![first.model];
        let mut transcript = vec![
            Message::user(prompts[0]),
            Message::assistant("Initial implementation completed; continue with the same task."),
        ];

        for (turn, base) in base_routes.iter().enumerate().skip(1) {
            let fallbacks = [SOL, TERRA, LUNA]
                .into_iter()
                .filter(|candidate| candidate != base)
                .map(str::to_string)
                .collect();
            let context = RoutingContext::from_messages(&transcript)
                .with_session_affinity(Some(affinity.clone()), prefix_tokens[turn]);
            let hints = RouteHints::from_context(prompts[turn], &context);
            assert!(
                hints.continuation,
                "turn {} must be recognized as a dependent continuation",
                turn + 1
            );
            let decision = router.apply_session_affinity(
                RoutingDecision {
                    tier: TaskTier::Complex,
                    model: (*base).into(),
                    rationale: "retained route replay".into(),
                    fallbacks,
                    pinned: false,
                },
                &context,
                hints,
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                None,
                false,
            );
            replayed.push(decision.model.clone());
            affinity.model = decision.model;
            transcript.push(Message::user(prompts[turn]));
            transcript.push(Message::assistant("Continuation completed."));
        }

        let switch_count =
            |models: &[String]| models.windows(2).filter(|pair| pair[0] != pair[1]).count();
        let old_routes: Vec<String> = old_completed_routes
            .into_iter()
            .map(str::to_string)
            .collect();
        assert_eq!(switch_count(&old_routes), 2);
        assert_eq!(switch_count(&replayed), 1);
        assert_eq!(
            replayed,
            [TERRA, SOL, SOL, SOL, SOL, SOL].map(str::to_string)
        );

        let measured_first_use_uncached = [55_953_u64, 65_082, 149_400];
        let avoided_cold_prefix_cost = measured_first_use_uncached[2];
        assert_eq!(avoided_cold_prefix_cost, 149_400);
        assert_eq!(451_483_u64 - avoided_cold_prefix_cost, 302_083);

        // Confirmation v3 completed turn 3 on Luna only after Sol failed
        // transiently. Replaying its next retained decision must restore Sol:
        // Luna is warm, but six measured coding points behind the now-healthy
        // quality anchor. The full 30,804-token cold estimate remains visible
        // and does not conceal that quality override.
        let post_failover_context = RoutingContext::from_messages(&transcript)
            .with_session_affinity(
                Some(SessionAffinity {
                    model: LUNA.into(),
                    tier: TaskTier::Complex,
                    code_heavy: true,
                }),
                30_804,
            );
        let restored = router.apply_session_affinity(
            RoutingDecision {
                tier: TaskTier::Complex,
                model: LUNA.into(),
                rationale: "confirmation-v3 turn-4 replay".into(),
                fallbacks: vec![SOL.into(), TERRA.into()],
                pinned: false,
            },
            &post_failover_context,
            RouteHints::from_context(prompts[3], &post_failover_context),
            &ModelHealth::default(),
            &SubscriptionQuota::default(),
            None,
            None,
            false,
        );
        assert_eq!(restored.model, SOL, "{}", restored.rationale);
        assert_eq!(restored.fallbacks.first().map(String::as_str), Some(LUNA));
        assert!(restored
            .rationale
            .contains("measured quality advantage 6.00"));
        assert!(restored.rationale.contains("30804 tokens"));
    }
}

/// Where spending sits relative to a cap. Ordered `Ok < Warning < Exhausted` so the stricter
/// of two axes can be taken with `.max()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BudgetStatus {
    /// No cap, or comfortably under it.
    Ok,
    /// At or past the warn threshold (default 80% of the cap), not yet over.
    Warning,
    /// At or over the cap — the router downshifts to the cheapest tier.
    Exhausted,
}

/// Default fraction of the cap at which to warn the user.
pub const DEFAULT_WARN_FRACTION: f64 = 0.8;

impl BudgetState {
    fn axis(spent: f64, cap: Option<f64>, warn: f64) -> BudgetStatus {
        match cap {
            Some(c) if spent >= c => BudgetStatus::Exhausted,
            Some(c) if spent >= c * warn => BudgetStatus::Warning,
            _ => BudgetStatus::Ok,
        }
    }

    /// Classify current spending: the stricter of all configured axes wins.
    /// Documented in docs/features/mesh-routing.md.
    pub fn status(&self) -> BudgetStatus {
        Self::axis(self.spent_today_usd, self.daily_cap_usd, self.warn_fraction)
            .max(Self::axis(
                self.spent_week_usd,
                self.weekly_cap_usd,
                self.warn_fraction,
            ))
            .max(Self::axis(
                self.spent_month_usd,
                self.monthly_cap_usd,
                self.warn_fraction,
            ))
    }
}

/// The Mesh's decision for one task, including *why* (recorded + shown to the user).
#[derive(Debug, Clone)]
pub struct RoutingDecision {
    pub tier: TaskTier,
    pub model: String,
    pub rationale: String,
    /// Ordered, already-filtered (available + healthy) alternatives to try if `model` fails
    /// mid-turn — most-preferred first, the routed tier's runners-up then cross-tier picks.
    /// Empty when nothing else is usable.
    pub fallbacks: Vec<String>,
    /// Whether `model` is an EXPLICIT user pin (`--model` / a hard duel pin) rather than a mesh
    /// pick. Carried to the failover decision point: a pinned model is rate-limit-retried with
    /// backoff on the SAME model and never silently switched (unless `mesh.pin_failover = true`) —
    /// a pin must pin (harness-robustness wave 2).
    pub pinned: bool,
}

/// A routing strategy. `async` so an implementation may consult a model (e.g. the opt-in
/// LLM classifier); the default [`HeuristicRouter`] resolves instantly with no I/O. `health`
/// is the set of currently-benched models to route around (failover).
#[async_trait]
pub trait Router: Send + Sync {
    /// `has_images` is whether the CURRENT turn has pending image (vision) attachments — when
    /// true, implementations should prefer a vision-capable model (see
    /// [`catalog::supports_vision`]) and only fail open to a non-vision model if no vision-capable
    /// candidate is usable. Without this signal a turn with an image attached can silently route
    /// to a text-only model and get an immediate provider 404 ("No endpoints found that support
    /// image input").
    #[allow(clippy::too_many_arguments)]
    async fn route(
        &self,
        prompt: &str,
        has_images: bool,
        budget: BudgetState,
        health: &ModelHealth,
        quota: &SubscriptionQuota,
        effort: Option<EffortLevel>,
        project: &ProjectContext,
    ) -> RoutingDecision;

    /// Route with an optional tier hint from an invoked command/skill (`tier:` frontmatter).
    /// The default ignores the hint and delegates to [`Router::route`]; classifying routers
    /// override this to pin the tier (an explicit user `--model` pin still wins, handled in
    /// `decide`). A `None` hint is exactly today's behaviour.
    #[allow(clippy::too_many_arguments)]
    async fn route_hinted(
        &self,
        prompt: &str,
        has_images: bool,
        budget: BudgetState,
        health: &ModelHealth,
        quota: &SubscriptionQuota,
        _tier_override: Option<TaskTier>,
        effort: Option<EffortLevel>,
        project: &ProjectContext,
    ) -> RoutingDecision {
        self.route(prompt, has_images, budget, health, quota, effort, project)
            .await
    }

    /// Route with bounded prior-turn context. Implementations that do not classify contextually
    /// remain source-compatible through the default delegation to [`Router::route_hinted`].
    #[allow(clippy::too_many_arguments)]
    async fn route_contextual(
        &self,
        prompt: &str,
        has_images: bool,
        budget: BudgetState,
        health: &ModelHealth,
        quota: &SubscriptionQuota,
        tier_override: Option<TaskTier>,
        effort: Option<EffortLevel>,
        project: &ProjectContext,
        _context: &RoutingContext,
    ) -> RoutingDecision {
        self.route_hinted(
            prompt,
            has_images,
            budget,
            health,
            quota,
            tier_override,
            effort,
            project,
        )
        .await
    }

    /// Route to the top-`n` DISTINCT-PROVIDER candidates for the same task (model arena / `/duel`):
    /// each entry is a full [`RoutingDecision`] as if that model were the primary pick, so the
    /// caller can run the same task concurrently across several models. The default just wraps a
    /// single [`Router::route`] call (a one-candidate arena) so implementations that don't have a
    /// natural notion of "next-best" (e.g. `FixedRouter` in tests) still satisfy the trait;
    /// [`HeuristicRouter`] overrides this to actually rank alternatives.
    #[allow(clippy::too_many_arguments)]
    async fn route_candidates(
        &self,
        prompt: &str,
        has_images: bool,
        budget: BudgetState,
        health: &ModelHealth,
        quota: &SubscriptionQuota,
        effort: Option<EffortLevel>,
        project: &ProjectContext,
        _n: usize,
    ) -> Vec<RoutingDecision> {
        vec![
            self.route(prompt, has_images, budget, health, quota, effort, project)
                .await,
        ]
    }

    /// Ordered trivial-tier candidate shortlist (health applied by the caller). Default empty so
    /// non-classifying routers are unaffected. Used to route cheap side-calls (classify, compact)
    /// with real failover instead of a single fixed model.
    fn trivial_candidates(&self) -> Vec<String> {
        Vec::new()
    }
}

/// The default v0.1 router: deterministic heuristics over cheap local signals (ADR-0006).
pub struct HeuristicRouter {
    config: Config,
    /// A user-pinned model (`--model`) that bypasses classification, subject to the budget
    /// contract. `None` = classify normally.
    pin: Option<String>,
    /// Whether `model`'s provider has a usable key (for provider fallback). Injectable so
    /// tests are deterministic; defaults to a real env/keyring check.
    model_available: fn(&str) -> bool,
    /// Bundled+configured rates, used to rank candidate models by relative cost.
    pricing: pricing::Pricing,
    /// Live catalog of usable models (auto-discovery). When present and `mesh.auto_discover` is
    /// on, the router ranks the best discovered model per tier instead of the configured lists.
    catalog: Option<ModelCatalog>,
    /// Known context-window sizes (model id → token count). Used to filter out models that
    /// cannot fit the current transcript during routing.
    context_windows: std::collections::HashMap<String, u32>,
    /// Per-repo routing boost learned from past `/duel` outcomes (model id → boost). Applied as a
    /// stable reorder over the ranked candidate list — a model that has won duels in THIS repo
    /// floats above an otherwise-equally-ranked peer; empty = no-op (today's behaviour).
    repo_boosts: std::collections::HashMap<String, f64>,
}

fn default_model_available(model: &str) -> bool {
    forge_config::has_api_key(forge_config::provider_of(model))
}

/// Whether an explicit `--model` pin can be dispatched straight to its provider, bypassing mesh
/// classification: exactly the predicate [`HeuristicRouter::is_usable`] applies to a pin (the
/// provider must have a usable key, or be keyless). Exposed as the SINGLE source of truth so any
/// caller that honors a hard pin — `forge run --model <id>` and the OpenAI-compatible `forge api`
/// endpoint alike — agrees on what "a dispatchable pin" is and the two paths cannot silently
/// diverge (the gap that let #509's API fix miss valid, un-advertised models).
/// Documented in docs/features/mesh-routing.md.
pub fn pin_is_dispatchable(model: &str) -> bool {
    default_model_available(model)
}

/// A measured score gap above this is a material quality improvement and defeats affinity.
const AFFINITY_QUALITY_BAND: f64 = 1.0;
/// Explicitly adversarial/review-critical turns trade cache warmth for a smaller measured edge.
const AFFINITY_CRITICAL_QUALITY_BAND: f64 = 0.5;
/// Below this much reusable context, switching is cheap enough to keep normal mesh ordering.
const AFFINITY_MIN_COLD_PREFIX_TOKENS: u64 = 4_096;
/// Conservative prefill estimate used only to compare a calibrated latency advantage with a cold
/// model switch. It is not provider billing or a claim about a particular backend.
const AFFINITY_ESTIMATED_PREFILL_TOKENS_PER_SECOND: f64 = 4_000.0;
const AFFINITY_LATENCY_MIN_SAMPLES: u32 = 20;
const AFFINITY_MIN_SUCCESS_RATE: f64 = 0.90;
const AFFINITY_LATENCY_MARGIN_MS: f64 = 5_000.0;
const INITIAL_ANCHOR_LATENCY_MARGIN_MS: f64 = 1_000.0;

/// Scale the minimum required context window by the active effort level. HIGH effort inflates it
/// by 1.5×, XHIGH by 2×. No adjustment for Low/Medium or when no minimum is set.
fn effective_min_context(min_tokens: Option<u32>, effort: Option<EffortLevel>) -> Option<u32> {
    min_tokens.map(|t| match effort {
        Some(EffortLevel::High) => t.saturating_mul(3) / 2,
        Some(EffortLevel::XHigh) | Some(EffortLevel::WhiteHot) => t.saturating_mul(2),
        _ => t,
    })
}

impl HeuristicRouter {
    /// Documented in docs/features/mesh-routing.md.
    pub fn new(config: Config) -> Self {
        let pricing = pricing::Pricing::from_config(&config);
        Self {
            config,
            pin: None,
            model_available: default_model_available,
            pricing,
            catalog: None,
            context_windows: std::collections::HashMap::new(),
            repo_boosts: std::collections::HashMap::new(),
        }
    }

    /// Pin a model (`--model`); empty/`None` clears it.
    /// Documented in docs/features/mesh-routing.md.
    pub fn with_pin(mut self, pin: Option<String>) -> Self {
        self.pin = pin.filter(|s| !s.is_empty());
        self
    }

    /// Attach a discovered model catalog for auto-discovery routing (no-op when empty).
    /// Documented in docs/features/mesh-routing.md.
    pub fn with_catalog(mut self, catalog: ModelCatalog) -> Self {
        self.catalog = Some(catalog);
        self
    }

    /// Attach known context-window sizes so the router can skip models that can't fit the
    /// current transcript.
    /// Documented in docs/features/mesh-routing.md.
    pub fn with_context_windows(mut self, windows: std::collections::HashMap<String, u32>) -> Self {
        self.context_windows = windows;
        self
    }

    /// Attach per-repo routing boosts learned from past `/duel` outcomes (empty = no-op).
    /// Documented in docs/features/mesh-routing.md.
    pub fn with_repo_boosts(mut self, boosts: std::collections::HashMap<String, f64>) -> Self {
        self.repo_boosts = boosts;
        self
    }

    /// Returns `true` when `model`'s known context window comfortably exceeds `min_tokens`.
    /// Models with no recorded window are assumed to fit (fail-open).
    fn context_fits(&self, model: &str, min_tokens: Option<u32>) -> bool {
        let Some(min) = min_tokens else {
            return true;
        };
        self.context_windows.get(model).is_none_or(|&w| w > min)
    }

    /// Whether auto-discovery routing is active (enabled + a non-empty catalog attached).
    fn auto_active(&self) -> bool {
        self.config.mesh.auto_discover && self.catalog.as_ref().is_some_and(|c| !c.is_empty())
    }

    /// Ordered shortlist used by the LLM classifier. It classifies with capable, FREE models —
    /// deliberately NOT the weakest trivial-tier models (which mislabel real code work as trivial,
    /// then route it to a model too weak to do it) and NOT subscription models (which would burn
    /// quota on every turn's classification). Ranked at the Standard tier and filtered to free, so
    /// the label is reliable at zero cost. Falls back to the trivial-tier shortlist if no free
    /// Standard model is available. Health is applied later because it changes between turns.
    pub fn classifier_candidates(&self) -> Vec<String> {
        let free: Vec<String> = self
            .candidates_for_tier(
                TaskTier::Standard,
                RouteHints::default(),
                &SubscriptionQuota::default(),
                None,
            )
            .into_iter()
            .filter(|m| catalog::is_free(m, self.pricing.estimated_cost(m), false))
            .collect();
        let mut capable_free: Vec<String> = free
            .iter()
            .filter(|model| {
                self.catalog
                    .as_ref()
                    .and_then(|catalog| catalog.benchmark_for(model))
                    .map_or_else(
                        || capability::quality_class(model) >= 2,
                        |(intelligence, _)| intelligence >= capability::CAPABLE_BENCH_THRESHOLD,
                    )
            })
            .cloned()
            .collect();
        // If the catalog has no capable free classifier at all, retain the old availability-first
        // fallback rather than disabling LLM classification. A weak model is acceptable only when
        // it is the sole free option; it must never outrank a measured capable alternative.
        if capable_free.is_empty() {
            capable_free = free;
        }
        // Classification is latency-sensitive and has a hard 15s total budget. A high-quality
        // free NIM model is a poor first choice when it routinely spends that entire budget,
        // forcing the real route onto the heuristic. Keep the Standard-tier quality ordering
        // within each class, but place known low-latency free providers first. If none is usable
        // at call time LlmRouter still tries the remaining candidates and then falls back safely.
        capable_free.sort_by_key(|m| match catalog::provider_of(m) {
            "groq" => 0,
            "cerebras" => 1,
            "sambanova" => 2,
            "gemini" => 3,
            "ollama" => 4,
            _ => 5,
        });
        capable_free.truncate(3);
        if !capable_free.is_empty() {
            return capable_free;
        }
        self.candidates_for_tier(
            TaskTier::Trivial,
            RouteHints::default(),
            &SubscriptionQuota::default(),
            None,
        )
        .into_iter()
        .take(3)
        .collect()
    }

    /// [`auto_active`](Self::auto_active); otherwise the configured `[mesh.models]` candidates
    /// (the manual/override path, and the offline/no-catalog default).
    fn candidates_for_tier(
        &self,
        tier: TaskTier,
        hints: RouteHints,
        quota: &SubscriptionQuota,
        effort: Option<EffortLevel>,
    ) -> Vec<String> {
        let candidates = if self.auto_active() {
            // Rank EVERY routable discovered model (not a top-N): the result feeds the failover
            // chain, and the mesh must keep trying down the full list rather than give up after a
            // handful when tens of usable free models remain. The primary pick is still the
            // first usable entry, so a longer tail never changes selection — it only deepens
            // failover. (The bug: a top-5 cap meant ~6 unique models across tiers, so a few dead
            // providers exhausted the chain while most of the catalog went untried.)
            let Some(catalog) = self.catalog.as_ref() else {
                return self.apply_repo_boosts(self.config.candidates_for(tier));
            };
            let ranked = catalog.ranked_seeded(
                tier,
                &self.pricing,
                catalog.models().len(),
                hints.code_heavy,
                hints.seed,
                quota,
                effort,
            );
            if ranked.is_empty() {
                self.config.candidates_for(tier)
            } else {
                ranked
            }
        } else {
            self.config.candidates_for(tier)
        };
        self.apply_repo_boosts(candidates)
    }

    /// Stable-reorder `candidates` by repo-learned boost, highest first. A model with no recorded
    /// boost sorts as `0.0`, so ties among unboosted models keep their original (ranked) order —
    /// `sort_by` is a stable sort. No-op when no boosts are attached.
    fn apply_repo_boosts(&self, mut candidates: Vec<String>) -> Vec<String> {
        if self.repo_boosts.is_empty() {
            return candidates;
        }
        candidates.sort_by(|a, b| {
            let ba = self.repo_boosts.get(a).copied().unwrap_or(0.0);
            let bb = self.repo_boosts.get(b).copied().unwrap_or(0.0);
            bb.total_cmp(&ba)
        });
        candidates
    }

    /// Inject a deterministic provider-availability predicate (tests only).
    #[cfg(test)]
    fn with_availability(mut self, f: fn(&str) -> bool) -> Self {
        self.model_available = f;
        self
    }

    fn classify(prompt: &str, project: &ProjectContext) -> (TaskTier, String) {
        let c = score_prompt(prompt, project);
        (c.tier, c.reasons.join(", "))
    }

    fn classify_contextual(
        prompt: &str,
        project: &ProjectContext,
        context: &RoutingContext,
    ) -> (TaskTier, String) {
        let current = score_prompt(prompt, project);
        let Some(active_task) = context
            .is_dependent_turn(prompt)
            .then(|| context.active_task_material())
            .flatten()
        else {
            return (current.tier, current.reasons.join(", "));
        };
        let inherited = score_prompt(&active_task, project);
        let tier = max_tier(current.tier, inherited.tier);
        (
            tier,
            format!(
                "contextual follow-up; current: {}; active task floor: {} ({})",
                current.tier.as_str(),
                inherited.tier.as_str(),
                inherited.reasons.join(", ")
            ),
        )
    }

    /// Return the active portion of an unstructured prompt for classification. Role-aware callers
    /// should keep this compatibility switch off and pass the user task directly.
    pub fn classification_activity<'a>(&self, prompt: &'a str) -> &'a str {
        if !self.config.mesh.classifier_activity_focused {
            return prompt;
        }
        prompt
            .rsplit("\n\n")
            .map(str::trim)
            .find(|part| !part.is_empty())
            .unwrap_or(prompt)
    }

    /// Like [`classify`] but also reports whether the heuristic is confident enough that an
    /// LLM second-opinion would add little value. High confidence means the score is far from
    /// both tier boundaries (≤−4 for Trivial, ≥8 for Complex) OR a COMPLEX_HINTS hard-override
    /// fired. A near-boundary score (−3…7) is "uncertain" — hybrid classifiers should call an
    /// LLM to decide. This is the hook that makes [`ClassifierKind::Hybrid`] cheap: obvious
    /// Trivial / strongly-signalled Complex skip the extra model call entirely.
    /// Documented in docs/features/mesh-routing.md.
    pub fn classify_confident(prompt: &str, project: &ProjectContext) -> (TaskTier, bool, String) {
        let c = score_prompt(prompt, project);
        // score == i32::MAX → COMPLEX_HINTS hard override (always confident).
        // score ≤ −4 → strong Trivial pull (TRIVIAL_PATTERNS or double TRIVIAL_HINTS).
        // score ≥ 8  → two or more strong Complex signals (REASONING_TERM + something else).
        let confident = c.score == i32::MAX || c.score <= -4 || c.score >= 8;
        (c.tier, confident, c.reasons.join(", "))
    }

    /// A model is usable if its provider key is present (or it's keyless) AND it isn't
    /// currently benched (rate-limited / unavailable — failover).
    fn is_usable(&self, m: &str, health: &ModelHealth, quota: &SubscriptionQuota) -> bool {
        if !(self.model_available)(m)
            || forge_config::is_model_disabled(m, &self.config.mesh.disabled)
            || health.is_benched(m)
        {
            return false;
        }
        // An exhausted subscription is routed around entirely (L3), like a benched model.
        !(catalog::is_subscription(m) && quota.is_exhausted(forge_config::provider_of(m)))
    }

    /// Whether `m` may be auto-routed / failed-over to under the active credit mode. `Strict` means
    /// "free + subscription only" (the doc contract): a paid, metered model is dropped from the
    /// candidate set so neither the primary pick nor the failover chain can ever spend API credit
    /// without the user asking. Normal/Frugal impose no model restriction (Frugal is a token cap).
    /// This gates AUTO routing only — an explicit `--model` pin bypasses it (the pin path checks
    /// [`is_usable`] directly), so a deliberate paid pin still works.
    fn allowed_under_credit_mode(&self, m: &str) -> bool {
        if self.config.mesh.credit_mode != forge_types::CreditMode::Strict {
            return true;
        }
        catalog::is_subscription(m) || catalog::is_free(m, self.pricing.estimated_cost(m), false)
    }

    /// Drop a CLI bridge when its explicitly-paired OAuth twin passed every routing eligibility
    /// gate for this turn. This is intentionally later than catalog scoring: an OAuth model that
    /// is disabled, benched, quota-exhausted, context-incompatible, or unavailable must leave its
    /// bridge routable as the recovery surface. The pair registry in `catalog` makes this apply to
    /// every supported OAuth/CLI pair rather than to Codex-specific code.
    fn suppress_usable_oauth_superseded_bridges(models: &mut Vec<String>) {
        let usable: std::collections::HashSet<String> = models.iter().cloned().collect();
        models.retain(|model| {
            catalog::oauth_twin_for_bridge(model)
                .is_none_or(|oauth_twin| !usable.contains(&oauth_twin))
        });
    }

    /// Pick the cheapest *usable* model from `candidates` (L1). Ranking key:
    /// `(prefer_subscription && subscription ? 0 : 1, estimated_cost, config_order)` — so a
    /// paid subscription (the $0 CLI bridges) wins when preferred, then lowest est. cost, then
    /// the order the user listed candidates. `None` when none are usable. The production path
    /// uses [`ordered_usable_for_tier`](Self::ordered_usable_for_tier); this stays for the
    /// cost-ranking unit tests.
    #[cfg(test)]
    fn cheapest_usable(&self, candidates: &[String], health: &ModelHealth) -> Option<String> {
        let quota = SubscriptionQuota::default();
        candidates
            .iter()
            .enumerate()
            .filter(|(_, m)| self.is_usable(m, health, &quota))
            .min_by(|(ia, a), (ib, b)| self.cost_rank(a).cmp(&self.cost_rank(b)).then(ia.cmp(ib)))
            .map(|(_, m)| m.clone())
    }

    /// Comparable cost ranking key for one model: `(not-preferred-subscription, est_cost)`.
    fn cost_rank(&self, m: &str) -> (u8, CostKey) {
        let prefer = self.config.mesh.prefer_subscription;
        (
            u8::from(!(prefer && catalog::is_subscription(m))),
            CostKey(self.pricing.estimated_cost(m)),
        )
    }

    /// Usable candidates for one tier, in preference order: the auto-discovered capability
    /// ranking (cost folded in) when auto is active, else cheapest-first over the configured
    /// candidates.
    #[allow(clippy::too_many_arguments)]
    fn ordered_usable_for_tier(
        &self,
        tier: TaskTier,
        health: &ModelHealth,
        hints: RouteHints,
        quota: &SubscriptionQuota,
        effort: Option<EffortLevel>,
        min_context: Option<u32>,
        has_images: bool,
    ) -> Vec<String> {
        let candidates = self.candidates_for_tier(tier, hints, quota, effort);
        let min = effective_min_context(min_context, effort);
        let mut usable: Vec<String> = candidates
            .iter()
            .filter(|m| self.is_usable(m, health, quota))
            .filter(|m| self.allowed_under_credit_mode(m))
            .filter(|m| self.context_fits(m, min))
            .cloned()
            .collect();
        if has_images {
            // Prefer a vision-capable model when this turn has image attachments; fail OPEN to
            // the unfiltered list if none of the usable candidates support vision — better to
            // attempt with a non-vision model (and surface the provider's real error) than to
            // refuse to route at all.
            let vision_only: Vec<String> = usable
                .iter()
                .filter(|m| catalog::supports_vision(m))
                .cloned()
                .collect();
            if !vision_only.is_empty() {
                usable = vision_only;
            }
        }
        Self::suppress_usable_oauth_superseded_bridges(&mut usable);
        if !self.auto_active() {
            // Configured path: cost-aware order (auto path keeps the ranked order verbatim).
            usable.sort_by_key(|m| self.cost_rank(m));
        }
        // Demote a near-limit subscription (Warning, L3) to the back — still a fallback, but the
        // mesh tries everything else first. Stable, so it preserves the order within each group.
        usable.sort_by_key(|m| quota.is_pressured(forge_config::provider_of(m)));

        // The first turn defines the implementation every later continuation inherits. On a
        // complex coding task, constrain the anchor to the strongest measured quality band.
        // Within that band, sufficiently sampled latency chooses the faster anchor; without
        // comparable calibration, the exact strongest score still leads. The old catalog-only
        // guard could see a
        // high-quality but unavailable alternative, fire conservation, and then let a much weaker
        // healthy model lead the real chain. Follow-up turns retain the normal score/pressure
        // order, preserving mesh's speed and quota benefits after the task foundation is sound.
        if self.auto_active()
            && tier == TaskTier::Complex
            && hints.code_heavy
            && !hints.continuation
            && effort != Some(EffortLevel::Low)
        {
            if let Some(catalog) = self.catalog.as_ref() {
                let metric = |model: &str| catalog.benchmark_for(model).map(|(_, coding)| coding);
                if let Some(max_quality) = usable
                    .iter()
                    .filter_map(|model| metric(model))
                    .max_by(f64::total_cmp)
                {
                    usable.sort_by(|a, b| {
                        let a_score = metric(a);
                        let b_score = metric(b);
                        match (a_score, b_score) {
                            (Some(a_score), Some(b_score)) => {
                                let a_in_band = max_quality - a_score <= AFFINITY_QUALITY_BAND;
                                let b_in_band = max_quality - b_score <= AFFINITY_QUALITY_BAND;
                                if a_in_band && b_in_band {
                                    if let (Some(a_latency), Some(b_latency)) = (
                                        dependable_calibrated_latency(catalog, a),
                                        dependable_calibrated_latency(catalog, b),
                                    ) {
                                        if (a_latency - b_latency).abs()
                                            >= INITIAL_ANCHOR_LATENCY_MARGIN_MS
                                        {
                                            return a_latency.total_cmp(&b_latency);
                                        }
                                    }
                                }
                                b_score.total_cmp(&a_score)
                            }
                            (Some(_), None) => std::cmp::Ordering::Less,
                            (None, Some(_)) => std::cmp::Ordering::Greater,
                            (None, None) => std::cmp::Ordering::Equal,
                        }
                    });
                }
            }
        }
        // Failover follows the mesh ranking verbatim: the Nth model Forge tries is the Nth-best
        // ranked model, not the top model of the Nth provider. (A previous round-robin interleave
        // destroyed cross-provider rank order — e.g. it sent release work to a low-ranked free
        // model after a higher-ranked provider's first model failed.) Rate-limit storms are
        // handled lazily downstream instead: forge-core skips a provider's *remaining* chain
        // entries only after one of its models actually returns a rate-limit error, so rank order
        // is preserved for every other failure mode.
        usable
    }

    /// Build the ordered failover chain for the routed tier: that tier's usable models first,
    /// then the other tiers (Complex → Standard → Trivial) as cross-tier fallbacks, deduped.
    #[allow(clippy::too_many_arguments)]
    fn build_chain(
        &self,
        routed: TaskTier,
        health: &ModelHealth,
        hints: RouteHints,
        quota: &SubscriptionQuota,
        effort: Option<EffortLevel>,
        min_context: Option<u32>,
        has_images: bool,
    ) -> Vec<String> {
        let mut chain = self.ordered_usable_for_tier(
            routed,
            health,
            hints,
            quota,
            effort,
            min_context,
            has_images,
        );
        for tier in [TaskTier::Complex, TaskTier::Standard, TaskTier::Trivial] {
            if tier == routed {
                continue;
            }
            for m in self.ordered_usable_for_tier(
                tier,
                health,
                hints,
                quota,
                effort,
                min_context,
                has_images,
            ) {
                if !chain.contains(&m) {
                    chain.push(m);
                }
            }
        }
        // A twin can appear in a different tier's configured list. Apply the same rule across
        // the completed chain so a cross-tier fallback never reintroduces a bridge that has a
        // usable OAuth surface elsewhere in this turn.
        Self::suppress_usable_oauth_superseded_bridges(&mut chain);
        chain
    }

    fn affinity_quality(&self, model: &str, code_heavy: bool) -> Option<f64> {
        self.catalog
            .as_ref()
            .and_then(|catalog| catalog.benchmark_for(model))
            .map(|(intelligence, coding)| if code_heavy { coding } else { intelligence })
    }

    fn estimated_cold_prefix_ms(prefix_tokens: u64) -> f64 {
        prefix_tokens as f64 / AFFINITY_ESTIMATED_PREFILL_TOKENS_PER_SECOND * 1_000.0
    }

    /// Quality-bounded, session-local reordering over the normal usable/failover chain.
    ///
    /// Reorder an already-classified decision with the same cache-affinity policy used by
    /// [`Router::route_contextual`].
    ///
    /// Classifier wrappers must call this after replacing the heuristic tier decision; otherwise
    /// rebuilding the candidate order silently discards the live session's affinity inputs. The
    /// normal mesh decision remains authoritative unless the exact model that completed the
    /// previous dependent turn is still usable, in the same task tier, sufficiently close in
    /// measured quality, and not beaten by enough calibrated latency to pay for a cold prefix.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_session_affinity(
        &self,
        mut decision: RoutingDecision,
        context: &RoutingContext,
        hints: RouteHints,
        health: &ModelHealth,
        quota: &SubscriptionQuota,
        effort: Option<EffortLevel>,
        min_context: Option<u32>,
        has_images: bool,
    ) -> RoutingDecision {
        let Some(affinity) = context.session_affinity() else {
            return decision;
        };
        if !hints.continuation || decision.pinned {
            return decision;
        }

        let warm = affinity.model.as_str();
        let provider = forge_config::provider_of(warm);
        let cold_tokens = context.reusable_prefix_tokens();
        let append_override = |decision: &mut RoutingDecision, reason: &str| {
            decision.rationale.push_str(&format!(
                " — session affinity overridden for {warm}: {reason}; estimated cold prefix \
                 {cold_tokens} tokens"
            ));
        };

        if decision.tier != affinity.tier {
            append_override(&mut decision, "material task-class change");
            return decision;
        }
        if hints.code_heavy != affinity.code_heavy {
            append_override(&mut decision, "material task-class change");
            return decision;
        }
        if !(self.model_available)(warm) {
            append_override(&mut decision, "model unavailable");
            return decision;
        }
        if health.is_benched(warm) {
            append_override(&mut decision, "model unhealthy or degraded");
            return decision;
        }
        if quota.is_exhausted(provider) {
            append_override(&mut decision, "provider quota exhausted");
            return decision;
        }
        if quota.is_pressured(provider) {
            append_override(&mut decision, "provider quota under pressure");
            return decision;
        }
        if !self.allowed_under_credit_mode(warm) {
            append_override(&mut decision, "credit policy excludes model");
            return decision;
        }
        if !self.context_fits(warm, effective_min_context(min_context, effort)) {
            append_override(&mut decision, "context window exhausted");
            return decision;
        }
        if has_images && !catalog::supports_vision(warm) {
            append_override(
                &mut decision,
                "current turn requires unsupported capability",
            );
            return decision;
        }

        let warm_is_candidate =
            decision.model == warm || decision.fallbacks.iter().any(|candidate| candidate == warm);
        if !warm_is_candidate {
            append_override(
                &mut decision,
                "model is outside the current usable routing chain",
            );
            return decision;
        }

        if self
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.runtime_calibration_for(warm))
            .is_some_and(|calibration| {
                calibration.samples >= AFFINITY_LATENCY_MIN_SAMPLES
                    && (!calibration.success_rate.is_finite()
                        || calibration.success_rate < AFFINITY_MIN_SUCCESS_RATE)
            })
        {
            append_override(&mut decision, "measured runtime reliability degraded");
            return decision;
        }

        let quality_band = if hints.quality_critical {
            AFFINITY_CRITICAL_QUALITY_BAND
        } else {
            AFFINITY_QUALITY_BAND
        };

        // Normal continuation ranking deliberately values speed and conservation,
        // so its primary is not necessarily the strongest measured alternative.
        // Compare the warm model with the whole usable chain. This also restores
        // the quality anchor after a transient failover made a weaker fallback
        // warm, without imposing frontier models on standard/trivial work.
        if decision.tier == TaskTier::Complex {
            let stronger = self
                .affinity_quality(warm, hints.code_heavy)
                .and_then(|warm_score| {
                    std::iter::once(&decision.model)
                        .chain(decision.fallbacks.iter())
                        .filter(|candidate| (self.model_available)(candidate))
                        .filter(|candidate| !health.is_benched(candidate))
                        .filter(|candidate| {
                            let candidate_provider = forge_config::provider_of(candidate);
                            !quota.is_exhausted(candidate_provider)
                                && !quota.is_pressured(candidate_provider)
                        })
                        .filter(|candidate| self.allowed_under_credit_mode(candidate))
                        .filter(|candidate| self.context_fits(candidate, min_context))
                        .filter_map(|candidate| {
                            self.affinity_quality(candidate, hints.code_heavy)
                                .map(|score| (candidate.clone(), score))
                        })
                        .max_by(|(_, left), (_, right)| left.total_cmp(right))
                        .filter(|(_, score)| *score - warm_score > quality_band)
                        .map(|(model, score)| (model, score - warm_score))
                });

            if let Some((stronger_model, advantage)) = stronger {
                let previous_primary =
                    std::mem::replace(&mut decision.model, stronger_model.clone());
                decision.fallbacks.retain(|candidate| {
                    candidate != &stronger_model
                        && candidate != warm
                        && candidate != &previous_primary
                });
                if previous_primary != stronger_model && previous_primary != warm {
                    decision.fallbacks.insert(0, previous_primary);
                }
                decision.fallbacks.insert(0, warm.to_string());
                append_override(
                    &mut decision,
                    &format!(
                        "{stronger_model} offers measured quality advantage {advantage:.2}, \
                         exceeding {quality_band:.2}-point {}band",
                        if hints.quality_critical {
                            "quality-critical "
                        } else {
                            ""
                        }
                    ),
                );
                return decision;
            }
        }

        if decision.model == warm {
            decision.rationale.push_str(&format!(
                " — session affinity retained {warm}: already best usable route; avoiding \
                 estimated {cold_tokens}-token cold prefix"
            ));
            return decision;
        }

        if cold_tokens < AFFINITY_MIN_COLD_PREFIX_TOKENS {
            append_override(
                &mut decision,
                "reusable prefix is too small to justify reordering",
            );
            return decision;
        }

        let best = decision.model.clone();
        let quality_gap = match (
            self.affinity_quality(&best, hints.code_heavy),
            self.affinity_quality(warm, hints.code_heavy),
        ) {
            (Some(best_score), Some(warm_score)) => Some(best_score - warm_score),
            (Some(_), None) => {
                append_override(
                    &mut decision,
                    "warm model lacks comparable quality evidence",
                );
                return decision;
            }
            (None, Some(_)) | (None, None) => {
                append_override(
                    &mut decision,
                    "models lack comparable measured quality evidence",
                );
                return decision;
            }
        };
        if let Some(gap) = quality_gap {
            if gap > quality_band {
                append_override(
                    &mut decision,
                    &format!(
                        "alternative quality advantage {gap:.2} exceeds \
                         {quality_band:.2}-point {}band",
                        if hints.quality_critical {
                            "quality-critical "
                        } else {
                            ""
                        }
                    ),
                );
                return decision;
            }
        }

        if let Some(catalog) = self.catalog.as_ref() {
            let warm_calibration = catalog.runtime_calibration_for(warm);
            let best_calibration = catalog.runtime_calibration_for(&best);
            if let (Some(warm_latency), Some(best_latency)) = (warm_calibration, best_calibration) {
                if warm_latency.samples >= AFFINITY_LATENCY_MIN_SAMPLES
                    && best_latency.samples >= AFFINITY_LATENCY_MIN_SAMPLES
                    && warm_latency.mean_latency_ms.is_finite()
                    && best_latency.mean_latency_ms.is_finite()
                    && warm_latency.mean_latency_ms > 0.0
                    && best_latency.mean_latency_ms > 0.0
                {
                    let improvement_ms =
                        warm_latency.mean_latency_ms - best_latency.mean_latency_ms;
                    let cold_ms = Self::estimated_cold_prefix_ms(cold_tokens);
                    if improvement_ms > cold_ms + AFFINITY_LATENCY_MARGIN_MS {
                        append_override(
                            &mut decision,
                            &format!(
                                "calibrated latency advantage {:.0}ms pays {:.0}ms cold-prefix \
                                 estimate plus {:.0}ms margin",
                                improvement_ms, cold_ms, AFFINITY_LATENCY_MARGIN_MS
                            ),
                        );
                        return decision;
                    }
                }
            }
        }

        let previous_primary = std::mem::replace(&mut decision.model, affinity.model.clone());
        decision.fallbacks.retain(|candidate| candidate != warm);
        decision.fallbacks.insert(0, previous_primary);
        let quality_reason = describe_affinity_quality(quality_gap, quality_band);
        decision.rationale.push_str(&format!(
            " — session affinity retained {warm}: dependent continuation, {quality_reason}; \
             avoiding estimated {cold_tokens}-token cold prefix"
        ));
        decision
    }
}

fn dependable_calibrated_latency(catalog: &ModelCatalog, model: &str) -> Option<f64> {
    catalog
        .runtime_calibration_for(model)
        .filter(|calibration| {
            calibration.samples >= AFFINITY_LATENCY_MIN_SAMPLES
                && calibration.success_rate.is_finite()
                && calibration.success_rate >= AFFINITY_MIN_SUCCESS_RATE
                && calibration.mean_latency_ms.is_finite()
                && calibration.mean_latency_ms > 0.0
        })
        .map(|calibration| calibration.mean_latency_ms)
}

fn initial_anchor_speed_note(
    catalog: &ModelCatalog,
    candidates: &[String],
    selected: &str,
) -> Option<String> {
    let (strongest_model, strongest_quality) = candidates
        .iter()
        .filter_map(|model| {
            catalog
                .benchmark_for(model)
                .map(|(_, coding)| (model, coding))
        })
        .max_by(|(_, left), (_, right)| left.total_cmp(right))?;
    if strongest_model == selected {
        return None;
    }

    let selected_quality = catalog.benchmark_for(selected)?.1;
    let quality_gap = strongest_quality - selected_quality;
    if !(0.0..=AFFINITY_QUALITY_BAND).contains(&quality_gap) {
        return None;
    }

    let selected_latency = dependable_calibrated_latency(catalog, selected)?;
    let strongest_latency = dependable_calibrated_latency(catalog, strongest_model)?;
    let latency_advantage = strongest_latency - selected_latency;
    if latency_advantage < INITIAL_ANCHOR_LATENCY_MARGIN_MS {
        return None;
    }

    Some(format!(
        "calibrated low-latency quality anchor: {selected} is {quality_gap:.2} points \
         from {strongest_model} within {AFFINITY_QUALITY_BAND:.2}-point band and \
         {latency_advantage:.0}ms faster per model call"
    ))
}

fn describe_affinity_quality(quality_gap: Option<f64>, quality_band: f64) -> String {
    quality_gap.map_or_else(
        || "warm model has measured quality advantage".to_string(),
        |gap| {
            if gap < 0.0 {
                format!("warm model quality advantage {:.2}", -gap)
            } else {
                format!(
                    "quality gap {:.2} within {:.2}-point band",
                    gap, quality_band
                )
            }
        },
    )
}

/// A `(u8, f64)`-comparable cost key. `f64` isn't `Ord`, so wrap it for use inside tuple
/// `.cmp()`; NaN (no price → treated as a stable max) can't occur here as costs are finite.
#[derive(PartialEq)]
struct CostKey(f64);
impl Eq for CostKey {}
impl PartialOrd for CostKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for CostKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl HeuristicRouter {
    /// Given an already-decided tier (from the heuristic OR an external classifier) + the
    /// reason it was chosen, apply pin / budget pressure / cost-aware candidate selection.
    /// Pure + sync, so any [`Router`] (incl. the LLM one) can reuse the whole selection path.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    /// Documented in docs/features/mesh-routing.md.
    pub fn decide(
        &self,
        classified_tier: TaskTier,
        classify_reason: String,
        budget: BudgetState,
        health: &ModelHealth,
        hints: RouteHints,
        quota: &SubscriptionQuota,
        effort: Option<EffortLevel>,
        has_images: bool,
    ) -> RoutingDecision {
        let exhausted = budget.status() == BudgetStatus::Exhausted;
        let bg_override_pin = self.config.mesh.budget.cap_overrides_pin;
        let min_context = budget.min_context_tokens;

        // A pin bypasses classification unless an exhausted budget may override it.
        if let Some(pin) = self
            .pin
            .as_ref()
            .filter(|_| !(exhausted && bg_override_pin))
        {
            let mut why = "pinned via --model".to_string();
            let mut chain = self.build_chain(
                classified_tier,
                health,
                hints,
                quota,
                effort,
                min_context,
                has_images,
            );
            let model = if self.is_usable(pin, health, quota) {
                pin.clone()
            } else {
                why.push_str(" — unavailable");
                pin.clone()
            };
            chain.retain(|m| m != &model);
            // An explicit pin remains explicit even if the provider is currently unavailable;
            // dispatching it surfaces the provider's actionable error instead of silently changing
            // the user's requested model.
            let pinned = true;
            // Strict pin semantics (harness-robustness wave 2, fix 2): an explicit pin gets NO
            // cross-model fallback chain — mid-turn failover off a pinned model silently
            // contaminated runs that depended on the exact model (the SWE-bench baseline switched
            // 2 pinned instances to a different model). A rate limit is waited out on the SAME
            // model (the pinned backoff in forge-core); a permanent error fails the turn with the
            // real cause. `mesh.pin_failover = true` restores the old switch-away behaviour.
            if pinned && !self.config.mesh.pin_failover {
                chain.clear();
            }
            return RoutingDecision {
                tier: classified_tier,
                model,
                rationale: why,
                fallbacks: chain,
                pinned,
            };
        }

        // Apply budget pressure (FR-5).
        let mut tier = classified_tier;
        let mut why = if self.pin.is_some() {
            // pin was set but an exhausted budget overrode it (see filter above)
            tier = TaskTier::Trivial;
            "budget cap reached — pin overridden, trivial tier".to_string()
        } else if exhausted && tier != TaskTier::Trivial {
            tier = TaskTier::Trivial;
            "budget cap reached — downshifted to trivial tier".to_string()
        } else {
            classify_reason
        };

        // The failover chain: usable models for the routed tier first, then cross-tier picks.
        // `routed_usable` lets us tell a same-tier pick (normal rationale) from a cross-tier
        // fallback ("fell back …") for the message.
        let auto = self.auto_active();
        let routed_usable = self.ordered_usable_for_tier(
            tier,
            health,
            hints,
            quota,
            effort,
            min_context,
            has_images,
        );
        let mut chain =
            self.build_chain(tier, health, hints, quota, effort, min_context, has_images);
        match chain.first().cloned() {
            Some(model) => {
                if routed_usable.contains(&model) {
                    // `routed_usable` (computed above) already applies the FULL routing filter
                    // (usable + credit-mode + context-fit) — reuse its count rather than
                    // `usable_count()`, which only checks `is_usable` and so overstates how many
                    // candidates `decide()` actually considered (e.g. it counts paid models even
                    // under `credit_mode = Strict`, where they're never actually routable).
                    let n = routed_usable.len();
                    if auto {
                        why.push_str(&format!(
                            " — auto-selected best of {n} usable {} models: {model}",
                            tier.as_str()
                        ));
                        if tier == TaskTier::Complex
                            && hints.code_heavy
                            && !hints.continuation
                            && effort != Some(EffortLevel::Low)
                        {
                            if let Some(note) = self.catalog.as_ref().and_then(|catalog| {
                                initial_anchor_speed_note(catalog, &routed_usable, &model)
                            }) {
                                why.push_str(&format!(" — {note}"));
                            }
                        }
                    } else if n > 1 {
                        why.push_str(&format!(
                            " — cheapest of {n} usable {} models: {model}",
                            tier.as_str()
                        ));
                    }
                } else {
                    let original = self
                        .candidates_for_tier(tier, hints, quota, effort)
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "unknown".into());
                    // Report WHY the primary was skipped — `is_usable` has three failure modes and
                    // only one is a missing key; for a benched or quota-exhausted model the key IS
                    // present, so "no usable key" was misleading. A model can also be `is_usable`
                    // but still dropped by the separate strict-credit-mode filter (a paid/metered
                    // model policy exclusion, not a quota problem) — check that before defaulting
                    // to "quota exhausted".
                    let reason = if !(self.model_available)(&original) {
                        "no usable key"
                    } else if health.is_benched(&original) {
                        "model benched"
                    } else if !self.allowed_under_credit_mode(&original) {
                        "excluded by strict credit mode"
                    } else {
                        "quota exhausted"
                    };
                    why.push_str(&format!(
                        " — fell back to {model} ({reason} for {original})"
                    ));
                }
                if self.config.mesh.prefer_subscription && catalog::is_subscription(&model) {
                    why.push_str(" (paid subscription)");
                }
                chain.retain(|m| m != &model);
                RoutingDecision {
                    tier,
                    model,
                    rationale: why,
                    fallbacks: chain,
                    pinned: false,
                }
            }
            None => {
                let original = self
                    .candidates_for_tier(tier, hints, quota, effort)
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "unknown".into());
                why.push_str(&format!(
                    " — warning: no usable key for {original} and no fallback"
                ));
                RoutingDecision {
                    tier,
                    model: original,
                    rationale: why,
                    fallbacks: Vec::new(),
                    pinned: false,
                }
            }
        }
    }
}

#[async_trait]
impl Router for HeuristicRouter {
    async fn route(
        &self,
        prompt: &str,
        has_images: bool,
        budget: BudgetState,
        health: &ModelHealth,
        quota: &SubscriptionQuota,
        effort: Option<EffortLevel>,
        project: &ProjectContext,
    ) -> RoutingDecision {
        let activity = self.classification_activity(prompt);
        let (tier, reason) = Self::classify(activity, project);
        self.decide(
            tier,
            reason,
            budget,
            health,
            RouteHints::from_prompt(activity),
            quota,
            effort,
            has_images,
        )
    }

    async fn route_hinted(
        &self,
        prompt: &str,
        has_images: bool,
        budget: BudgetState,
        health: &ModelHealth,
        quota: &SubscriptionQuota,
        tier_override: Option<TaskTier>,
        effort: Option<EffortLevel>,
        project: &ProjectContext,
    ) -> RoutingDecision {
        let activity = self.classification_activity(prompt);
        match tier_override {
            // A command/skill tier hint replaces classification but goes through the same
            // selection path (pin, budget pressure, cost-aware candidates all still apply).
            Some(tier) => self.decide(
                tier,
                format!("tier hint: {}", tier.as_str()),
                budget,
                health,
                RouteHints::from_prompt(activity),
                quota,
                effort,
                has_images,
            ),
            None => {
                self.route(prompt, has_images, budget, health, quota, effort, project)
                    .await
            }
        }
    }

    async fn route_contextual(
        &self,
        prompt: &str,
        has_images: bool,
        budget: BudgetState,
        health: &ModelHealth,
        quota: &SubscriptionQuota,
        tier_override: Option<TaskTier>,
        effort: Option<EffortLevel>,
        project: &ProjectContext,
        context: &RoutingContext,
    ) -> RoutingDecision {
        let activity = self.classification_activity(prompt);
        let hints = RouteHints::from_context(activity, context);
        let decision = match tier_override {
            Some(tier) => self.decide(
                tier,
                format!("tier hint: {}", tier.as_str()),
                budget,
                health,
                hints,
                quota,
                effort,
                has_images,
            ),
            None => {
                let (tier, reason) = Self::classify_contextual(activity, project, context);
                self.decide(
                    tier, reason, budget, health, hints, quota, effort, has_images,
                )
            }
        };
        self.apply_session_affinity(
            decision,
            context,
            hints,
            health,
            quota,
            effort,
            budget.min_context_tokens,
            has_images,
        )
    }

    async fn route_candidates(
        &self,
        prompt: &str,
        has_images: bool,
        budget: BudgetState,
        health: &ModelHealth,
        quota: &SubscriptionQuota,
        effort: Option<EffortLevel>,
        project: &ProjectContext,
        n: usize,
    ) -> Vec<RoutingDecision> {
        let activity = self.classification_activity(prompt);
        let (tier, reason) = Self::classify(activity, project);
        let hints = RouteHints::from_prompt(activity);
        let ranked = self.ordered_usable_for_tier(
            tier,
            health,
            hints,
            quota,
            effort,
            budget.min_context_tokens,
            has_images,
        );

        // Distinct-provider top-n: a duel across three models of the SAME provider isn't a useful
        // arena (correlated failure modes, same weights family in some setups) — one pick per
        // provider, in the mesh's own rank order.
        let mut seen = std::collections::HashSet::new();
        let mut picks: Vec<String> = Vec::new();
        for m in ranked {
            let provider = forge_config::provider_of(&m).to_string();
            if seen.insert(provider) {
                picks.push(m);
                if picks.len() >= n {
                    break;
                }
            }
        }

        picks
            .into_iter()
            .enumerate()
            .map(|(i, model)| RoutingDecision {
                tier,
                model,
                rationale: format!("duel candidate #{} — {reason}", i + 1),
                fallbacks: Vec::new(),
                pinned: false,
            })
            .collect()
    }

    fn trivial_candidates(&self) -> Vec<String> {
        self.classifier_candidates()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failover_chain_follows_mesh_rank_order_not_provider_interleave() {
        // The failover chain must walk models in the SAME order the mesh ranks them — the Nth
        // model tried is the Nth-best ranked model, NOT the top model of the Nth provider. (A
        // prior round-robin interleave broke this, which is how release work landed on a
        // low-ranked free model after a higher-ranked provider's first model failed over.)
        let r = mixed_router();
        let health = ModelHealth::default();
        let quota = SubscriptionQuota::default();
        let hints = RouteHints::default();
        let tier = TaskTier::Complex;

        let ranked_usable: Vec<String> = r
            .candidates_for_tier(tier, hints, &quota, None)
            .into_iter()
            .filter(|m| r.is_usable(m, &health, &quota))
            .collect();
        let chain = r.ordered_usable_for_tier(tier, &health, hints, &quota, None, None, false);
        assert_eq!(
            chain, ranked_usable,
            "failover order must equal mesh rank order with no provider interleaving"
        );
    }

    #[test]
    fn complex_coding_anchor_uses_best_measured_usable_model_before_conserving_followups() {
        let mut bench = BenchmarkScores::new();
        bench.insert("gpt-5.6-sol", 58.9, 77.4);
        bench.insert("qwen3.6-flash", 50.1, 69.2);
        bench.insert("kimi-k3", 57.1, 76.2);
        let models = vec![
            "codex-cli::gpt-5.6-sol".to_string(),
            "qwencloud::qwen3.6-flash".to_string(),
            "opencode_go::kimi-k3".to_string(),
        ];
        let catalog = ModelCatalog::new(models.clone()).with_benchmarks(Some(bench.clone()));
        let router = HeuristicRouter::new(Config::default())
            .with_availability(|model| !model.starts_with("opencode_go::"))
            .with_catalog(catalog);
        let quota = conserve_quota(0.7, "plus", "plus");
        let seed = (0..10_000)
            .find(|seed| {
                catalog::conserve_decision(
                    &models,
                    TaskTier::Complex,
                    true,
                    *seed,
                    &quota,
                    Some(&bench),
                )
                .fired
            })
            .expect("test setup must exercise active conservation");

        let anchor = router.ordered_usable_for_tier(
            TaskTier::Complex,
            &ModelHealth::default(),
            RouteHints {
                code_heavy: true,
                seed,
                continuation: false,
                quality_critical: false,
            },
            &quota,
            None,
            None,
            false,
        );
        assert_eq!(
            anchor.first().map(String::as_str),
            Some("codex-cli::gpt-5.6-sol"),
            "an unavailable near-peer must not let conservation weaken the task-defining turn"
        );

        let continuation = router.ordered_usable_for_tier(
            TaskTier::Complex,
            &ModelHealth::default(),
            RouteHints {
                code_heavy: true,
                seed,
                continuation: true,
                quality_critical: false,
            },
            &quota,
            None,
            None,
            false,
        );
        assert_eq!(
            continuation.first().map(String::as_str),
            Some("qwencloud::qwen3.6-flash"),
            "continuations should retain normal quota-aware mesh ordering"
        );
    }

    #[tokio::test]
    async fn usable_oauth_twin_removes_cli_bridge_from_routing_and_failover() {
        // A bridge is only a recovery path when its native OAuth twin is unavailable. Keeping
        // both in an otherwise healthy chain makes the mesh present duplicate providers and can
        // retry the same account through a less reliable surface in the same turn.
        let r = HeuristicRouter::new(list_config(
            "complex",
            &[
                "codex-oauth::gpt-5.6-luna",
                "codex-cli::gpt-5.6-luna",
                "groq::llama-3.3-70b-versatile",
            ],
        ))
        .with_availability(|_| true);
        let d = r
            .route(
                "design and architect a complex concurrency refactor across modules",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;

        assert_eq!(d.model, "codex-oauth::gpt-5.6-luna");
        assert!(
            !d.fallbacks
                .iter()
                .any(|model| model == "codex-cli::gpt-5.6-luna"),
            "a usable OAuth twin must suppress its CLI bridge: {:?}",
            d.fallbacks
        );
    }

    #[tokio::test]
    async fn unavailable_oauth_twin_keeps_cli_bridge_routable() {
        // Suppression must not sacrifice resilience: if OAuth cannot dispatch, the bridge is the
        // legitimate recovery surface and must remain eligible.
        let r = HeuristicRouter::new(list_config(
            "complex",
            &[
                "codex-oauth::gpt-5.6-luna",
                "codex-cli::gpt-5.6-luna",
                "groq::llama-3.3-70b-versatile",
            ],
        ))
        .with_availability(|model| !model.starts_with("codex-oauth::"));
        let d = r
            .route(
                "design and architect a complex concurrency refactor across modules",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;

        assert_eq!(d.model, "codex-cli::gpt-5.6-luna");
    }

    fn router() -> HeuristicRouter {
        // Treat every provider as available so tier-classification tests are deterministic
        // (no dependence on ambient env/keyring) and exercise no fallback.
        HeuristicRouter::new(Config::default()).with_availability(|_| true)
    }

    #[test]
    fn classifier_candidates_prefer_low_latency_free_providers() {
        // Regression: an NIM candidate could consume the classifier's entire 15-second budget,
        // so every uncertain request silently fell back to the heuristic despite a fast Groq
        // model being both configured and usable. The classifier must start with Groq whenever
        // it is present, while retaining other free providers as bounded fallbacks.
        let catalog = ModelCatalog::new(vec![
            "ollama::llama3.2".to_string(),
            "gemini::gemini-2.5-flash".to_string(),
            "groq::qwen/qwen3.6-27b".to_string(),
        ]);
        let candidates = HeuristicRouter::new(Config::default())
            .with_catalog(catalog)
            .classifier_candidates();

        assert_eq!(
            candidates.first().map(String::as_str),
            Some("groq::qwen/qwen3.6-27b"),
            "classifier must use the fast Groq candidate before slower free providers: {candidates:?}"
        );
        assert!(candidates.len() <= 3);
    }

    #[test]
    fn classifier_candidates_exclude_measured_weak_models_when_capable_free_exists() {
        let mut bench = BenchmarkScores::new();
        bench.insert("allam 2 7b", 4.0, 3.0);
        bench.insert("gemini 2.5 flash", 14.0, 16.0);
        let catalog = ModelCatalog::new(vec![
            "groq::allam-2-7b".to_string(),
            "gemini::gemini-2.5-flash".to_string(),
        ])
        .with_benchmarks(Some(bench));

        let candidates = HeuristicRouter::new(Config::default())
            .with_catalog(catalog)
            .classifier_candidates();

        assert!(
            candidates
                .iter()
                .any(|candidate| candidate == "gemini::gemini-2.5-flash"),
            "{candidates:?}"
        );
        assert!(
            !candidates
                .iter()
                .any(|candidate| candidate == "groq::allam-2-7b"),
            "a measured weak 7B model must not displace a capable free classifier: {candidates:?}"
        );
    }

    async fn contextual_decision(messages: &[Message], prompt: &str) -> RoutingDecision {
        router()
            .route_contextual(
                prompt,
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                None,
                &ProjectContext::default(),
                &RoutingContext::from_messages(messages),
            )
            .await
    }

    #[tokio::test]
    async fn complex_task_continuation_remains_complex() {
        let history = [
            Message::user("debug the race condition in the scheduler and prove the fix"),
            Message::assistant("I found the unsafe interleaving and am implementing the fix."),
        ];
        let decision = contextual_decision(&history, "continue").await;
        assert_eq!(decision.tier, TaskTier::Complex, "{}", decision.rationale);
    }

    #[tokio::test]
    async fn standard_task_do_it_remains_standard_and_code_heavy() {
        let history = [Message::user(
            "add a retry-with-backoff wrapper around the HTTP client",
        )];
        let context = RoutingContext::from_messages(&history);
        let decision = contextual_decision(&history, "do it").await;
        assert_eq!(decision.tier, TaskTier::Standard, "{}", decision.rationale);
        assert!(RouteHints::from_context("do it", &context).code_heavy);
    }

    #[tokio::test]
    async fn repeated_continuations_find_the_original_task_anchor() {
        let history = [
            Message::user("audit the permission checks across the authentication flow"),
            Message::assistant("I found two inconsistent authorization paths."),
            Message::user("continue"),
            Message::assistant("The first path is now fixed; the second still needs validation."),
            Message::user("go on"),
            Message::assistant("I am validating the recovery path."),
        ];
        let decision = contextual_decision(&history, "continue").await;
        assert_eq!(decision.tier, TaskTier::Complex, "{}", decision.rationale);
    }

    #[tokio::test]
    async fn explicit_new_trivial_task_does_not_inherit_complexity() {
        let history = [
            Message::user("architect a plugin system for the CLI"),
            Message::assistant("The architecture proposal is complete."),
        ];
        let decision = contextual_decision(&history, "fix this typo").await;
        assert_eq!(decision.tier, TaskTier::Trivial, "{}", decision.rationale);
    }

    #[tokio::test]
    async fn terminal_acknowledgement_after_complex_task_stays_trivial() {
        let history = [Message::user(
            "design a lock-free queue and prove its correctness",
        )];
        let decision = contextual_decision(&history, "thanks").await;
        assert_eq!(decision.tier, TaskTier::Trivial, "{}", decision.rationale);
    }

    #[tokio::test]
    async fn referential_refinement_inherits_active_task_tier() {
        let history = [Message::user(
            "investigate the intermittent deadlock in the scheduler",
        )];
        let decision = contextual_decision(&history, "fix that").await;
        assert_eq!(decision.tier, TaskTier::Complex, "{}", decision.rationale);
    }

    #[tokio::test]
    async fn compaction_summary_can_anchor_a_continuation() {
        let history = [Message::system(format!(
            "{COMPACTION_SUMMARY_PREFIX}\nActive task: debug a race condition in the scheduler, \
             prove the concurrency fix, and run stress tests."
        ))];
        let decision = contextual_decision(&history, "continue").await;
        assert_eq!(decision.tier, TaskTier::Complex, "{}", decision.rationale);
    }

    #[tokio::test]
    async fn endurance_history_keeps_followup_routing_bounded_and_complex() {
        let mut history = vec![Message::system(format!(
            "{COMPACTION_SUMMARY_PREFIX}\nActive task: debug a race condition in the scheduler, \
             prove the concurrency fix, and run stress tests."
        ))];
        for turn in 0..650 {
            history.push(Message::user("continue"));
            history.push(Message::assistant(format!(
                "still working through concurrency invariant {turn}"
            )));
            for tool in 0..10 {
                history.push(Message::tool_result(
                    format!("call-{turn}-{tool}"),
                    "bounded tool output",
                ));
            }
        }

        let started = std::time::Instant::now();
        let context = RoutingContext::from_messages(&history);
        let classifier_prompt = context.classifier_prompt("continue");
        let decision = router()
            .route_contextual(
                "continue",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                None,
                &ProjectContext::default(),
                &context,
            )
            .await;

        assert_eq!(decision.tier, TaskTier::Complex, "{}", decision.rationale);
        assert!(
            classifier_prompt.chars().count() <= 16_000,
            "classifier input must remain bounded, got {} chars",
            classifier_prompt.chars().count()
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "routing a 7,800-message endurance history became pathologically slow: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn contextual_tier_override_still_wins() {
        let history = [Message::user("architect a plugin system for the CLI")];
        let context = RoutingContext::from_messages(&history);
        let decision = router()
            .route_contextual(
                "continue",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                Some(TaskTier::Trivial),
                None,
                &ProjectContext::default(),
                &context,
            )
            .await;
        assert_eq!(decision.tier, TaskTier::Trivial);
        assert!(decision.rationale.contains("tier hint"));
    }

    #[test]
    fn routing_context_excludes_ui_chrome_and_bounds_classifier_prompt() {
        let huge = "design the distributed scheduler architecture ".repeat(2_000);
        let history = [
            Message::user(&huge),
            Message::system("Working…").ui_only(),
            Message::assistant(&huge),
        ];
        let context = RoutingContext::from_messages(&history);
        let rendered = context.classifier_prompt("continue");

        assert!(rendered.contains("ACTIVE USER TASK"));
        assert!(rendered.contains("CURRENT USER TURN TO CLASSIFY"));
        assert!(!rendered.contains("Working…"));
        assert!(
            rendered.chars().count() < 14_000,
            "classifier prompt was not bounded: {} chars",
            rendered.chars().count()
        );
    }

    #[test]
    fn independent_turn_classifier_prompt_excludes_prior_task_context() {
        let injected_system =
            "comprehensive analyse review understand integrate audit ".repeat(500);
        let history = [
            Message::system(injected_system),
            Message::user("design a distributed scheduler and prove its correctness"),
            Message::assistant("The architecture is complete."),
        ];
        let context = RoutingContext::from_messages(&history);
        let rendered = context.classifier_prompt("How many tasks are due today?");

        assert_eq!(
            rendered, "TASK TO CLASSIFY:\nHow many tasks are due today?",
            "an independent user task must not carry prior or system context into classification"
        );
    }

    #[test]
    fn current_tool_results_do_not_hide_a_referential_user_turn() {
        let context = RoutingContext::from_messages(&[Message::user(
            "debug the intermittent race condition in the scheduler",
        )]);
        let rendered = context.classifier_prompt(
            "continue\nTOOL RESULT:\nThe scheduler trace contains many detailed events.",
        );

        assert!(rendered.contains("ACTIVE USER TASK"));
        assert!(rendered.contains("CURRENT USER TURN TO CLASSIFY"));
    }

    #[tokio::test]
    async fn activity_focused_compatibility_ignores_prepended_system_context() {
        let mut config = Config::default();
        config.mesh.classifier = forge_config::ClassifierKind::Heuristic;
        config.mesh.classifier_activity_focused = true;
        let router = HeuristicRouter::new(config).with_availability(|_| true);
        let injected = "comprehensive analyse review understand integrate audit ".repeat(500);
        let prompt = format!("{injected}\n\nHow many tasks are due today?");

        let decision = router
            .route(
                &prompt,
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;

        assert_eq!(decision.tier, TaskTier::Trivial, "{}", decision.rationale);
        assert!(!decision.rationale.contains("long prompt"));
        assert!(!decision.rationale.contains("reasoning/algorithmic term"));
    }

    #[test]
    fn strict_credit_mode_excludes_paid_models_from_routing_and_failover() {
        // Regression: `credit_mode = "strict"` promises "free + subscription only", but it was wired
        // only to the token cap — paid models stayed in the failover chain, so a free pick that
        // failed over could land on a PAID model (e.g. openrouter/gemini-pro) without consent.
        let strict = {
            let mut c = Config::default();
            c.mesh.credit_mode = forge_types::CreditMode::Strict;
            HeuristicRouter::new(c)
                .with_availability(|_| true)
                .with_catalog(mixed_catalog())
        };
        let (health, quota, hints) = (
            ModelHealth::default(),
            SubscriptionQuota::default(),
            RouteHints::default(),
        );
        let chain = strict.build_chain(
            TaskTier::Standard,
            &health,
            hints,
            &quota,
            None,
            None,
            false,
        );
        // Paid, metered model is gone from the WHOLE chain (primary + every failover step).
        assert!(
            !chain.iter().any(|m| m == "gemini::gemini-2.5-pro"),
            "strict must drop paid gemini-pro; chain = {chain:?}"
        );
        // Subscription ($0 marginal) and unpriced-free local models remain routable.
        assert!(
            chain.iter().any(|m| m == "claude-cli::sonnet"),
            "subscription stays under strict; chain = {chain:?}"
        );
        assert!(
            chain.iter().any(|m| m == "ollama::llama3.2"),
            "free local stays under strict; chain = {chain:?}"
        );

        // Control: under Normal (default) the paid model stays in the chain.
        let normal = mixed_router();
        let normal_chain = normal.build_chain(
            TaskTier::Standard,
            &health,
            hints,
            &quota,
            None,
            None,
            false,
        );
        assert!(
            normal_chain.iter().any(|m| m == "gemini::gemini-2.5-pro"),
            "normal mode keeps paid models routable"
        );
    }

    #[tokio::test]
    async fn strict_credit_mode_exclusion_reports_correct_fallback_reason() {
        // Regression: a candidate that IS `is_usable` (key present, not benched, not exhausted)
        // but gets dropped only by the separate strict-credit-mode filter must not be mislabeled
        // "quota exhausted" in the fallback rationale — that's misleading since the provider
        // quota is fine; it was simply disallowed by policy.
        let mut c = Config::default();
        c.mesh.models.insert(
            TaskTier::Standard.as_str().into(),
            forge_config::OneOrMany::Many(vec!["gemini::gemini-2.5-pro".to_string()]),
        );
        c.mesh.credit_mode = forge_types::CreditMode::Strict;
        let r = HeuristicRouter::new(c).with_availability(|_| true);
        let prompt = "add a new endpoint that returns the list of users as json".repeat(2);
        let d = r
            .route(
                &prompt,
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Standard);
        assert_ne!(
            d.model, "gemini::gemini-2.5-pro",
            "strict mode must not pick the paid model"
        );
        assert!(
            d.rationale.contains("excluded by strict credit mode"),
            "{}",
            d.rationale
        );
        assert!(
            !d.rationale.contains("quota exhausted"),
            "must not mislabel a credit-mode policy exclusion as quota exhaustion: {}",
            d.rationale
        );
    }

    /// A realistic mixed catalog mirroring a user with claude+codex CLIs, local ollama, and
    /// keys for free-tier groq + metered gemini — the setup the routing policy targets.
    fn mixed_catalog() -> ModelCatalog {
        ModelCatalog::new(vec![
            "claude-cli::".into(),
            "claude-cli::opus".into(),
            "claude-cli::sonnet".into(),
            "claude-cli::haiku".into(),
            "codex-cli::".into(),
            "codex-cli::gpt-5.5".into(),
            "codex-cli::gpt-5.3-codex".into(),
            "codex-cli::gpt-5.4".into(),
            "codex-cli::gpt-5.4-mini".into(),
            "ollama::qwen3-coder:30b".into(),
            "ollama::llama3.2".into(),
            "groq::llama-3.1-8b-instant".into(),
            "groq::llama-3.3-70b-versatile".into(),
            "gemini::gemini-2.5-pro".into(),
            "gemini::gemini-2.5-flash".into(),
        ])
    }

    fn mixed_router() -> HeuristicRouter {
        HeuristicRouter::new(Config::default())
            .with_availability(|_| true)
            .with_catalog(mixed_catalog())
    }

    async fn route_model(r: &HeuristicRouter, prompt: &str) -> String {
        r.route(
            prompt,
            false,
            BudgetState::default(),
            &ModelHealth::default(),
            &SubscriptionQuota::default(),
            None,
            &ProjectContext::default(),
        )
        .await
        .model
    }

    async fn route_model_q(r: &HeuristicRouter, prompt: &str, q: &SubscriptionQuota) -> String {
        r.route(
            prompt,
            false,
            BudgetState::default(),
            &ModelHealth::default(),
            q,
            None,
            &ProjectContext::default(),
        )
        .await
        .model
    }

    /// A conservation-enabled quota: both subs at `frac` of their window, given plan slugs, Ok
    /// status (so we isolate proactive spreading from the hard Warning/Exhausted backstops).
    fn conserve_quota(frac: f64, plan_claude: &str, plan_codex: &str) -> SubscriptionQuota {
        let mut fr = std::collections::HashMap::new();
        fr.insert("claude-cli".to_string(), frac);
        fr.insert("codex-cli".to_string(), frac);
        let mut pl = std::collections::HashMap::new();
        pl.insert("claude-cli".to_string(), plan_claude.to_string());
        pl.insert("codex-cli".to_string(), plan_codex.to_string());
        SubscriptionQuota::new(std::collections::HashMap::new())
            .with_fractions(fr)
            .with_plans(pl)
            .with_conserve(true)
    }

    /// [`conserve_quota`] plus a pace projection on `claude-cli` — mesh-routing.md. Lets a
    /// test isolate the effect of a fast-burning-but-early window (low `frac`, high projection)
    /// from the plain fraction-only spreading `conserve_quota` alone exercises.
    fn conserve_quota_with_pace(
        frac: f64,
        plan_claude: &str,
        plan_codex: &str,
        projected_fraction_at_reset: f64,
    ) -> SubscriptionQuota {
        let mut pc = std::collections::HashMap::new();
        pc.insert(
            "claude-cli".to_string(),
            forge_types::QuotaPace {
                rate_per_hour: 0.0,
                rate_per_day: 0.0,
                projected_fraction_at_reset: Some(projected_fraction_at_reset),
                time_to_exhaustion_secs: None,
                exhaustion_warning: false,
            },
        );
        conserve_quota(frac, plan_claude, plan_codex).with_paces(pc)
    }

    /// Distinct complex prompts (varying the seed) for measuring routing spread.
    fn complex_workload(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| {
                format!(
                    "prove the correctness and analyze the asymptotic complexity of this \
                     distributed consensus approach, scenario {i}"
                )
            })
            .collect()
    }

    async fn subscription_share(
        r: &HeuristicRouter,
        q: &SubscriptionQuota,
        prompts: &[String],
    ) -> usize {
        let mut sub = 0;
        for p in prompts {
            if catalog::is_subscription(&route_model_q(r, p, q).await) {
                sub += 1;
            }
        }
        sub
    }

    #[tokio::test]
    async fn conservation_spreads_some_complex_off_subscriptions_while_fresh() {
        // The core ask: even with subscriptions fresh, NOT every complex task hits the best-2
        // subscriptions — a share spreads to the free-frontier pool to preserve the plan.
        let r = mixed_router();
        let prompts = complex_workload(80);
        let q = conserve_quota(0.0, "plus", "plus");
        let sub = subscription_share(&r, &q, &prompts).await;
        let free = prompts.len() - sub;
        assert!(
            free > 0,
            "some complex tasks must spread to free frontier: free={free}"
        );
        assert!(
            sub > free,
            "but subscriptions still take the majority while fresh: sub={sub} free={free}"
        );
    }

    #[tokio::test]
    async fn conservation_grows_as_the_weekly_window_fills() {
        let r = mixed_router();
        let prompts = complex_workload(80);
        let fresh_free = prompts.len()
            - subscription_share(&r, &conserve_quota(0.0, "plus", "plus"), &prompts).await;
        let full_free = prompts.len()
            - subscription_share(&r, &conserve_quota(0.7, "plus", "plus"), &prompts).await;
        assert!(
            full_free > fresh_free,
            "more tasks must spread off subscriptions as the window fills: fresh={fresh_free} full={full_free}"
        );
    }

    #[test]
    fn a_pace_projecting_near_exhaustion_ramps_conservation_like_a_full_window() {
        let models = vec![
            "claude-cli::sonnet".to_string(),
            "codex-cli::gpt-5.5".to_string(),
            "groq::llama-3.3-70b-versatile".to_string(),
        ];
        let seed = (0..10_000)
            .find(|seed| {
                let d = catalog::conserve_decision(
                    &models,
                    TaskTier::Complex,
                    false,
                    *seed,
                    &conserve_quota(0.2, "plus", "plus"),
                    None,
                );
                d.roll > 0.4 && d.roll < 0.99
            })
            .unwrap();
        let fresh = conserve_quota(0.2, "plus", "plus");
        let paced = conserve_quota_with_pace(0.2, "plus", "plus", 1.0);
        let at_cap = conserve_quota(1.0, "plus", "plus");
        let fresh_decision =
            catalog::conserve_decision(&models, TaskTier::Complex, false, seed, &fresh, None);
        let paced_decision =
            catalog::conserve_decision(&models, TaskTier::Complex, false, seed, &paced, None);
        let cap_decision =
            catalog::conserve_decision(&models, TaskTier::Complex, false, seed, &at_cap, None);

        assert!(!catalog::provider_conservation_fired(
            "claude-cli",
            TaskTier::Complex,
            false,
            fresh_decision,
            &fresh
        ));
        assert!(catalog::provider_conservation_fired(
            "claude-cli",
            TaskTier::Complex,
            false,
            paced_decision,
            &paced
        ));
        assert!(catalog::provider_conservation_fired(
            "claude-cli",
            TaskTier::Complex,
            false,
            cap_decision,
            &at_cap
        ));
        assert!(!catalog::provider_conservation_fired(
            "codex-cli",
            TaskTier::Complex,
            false,
            paced_decision,
            &paced
        ));
    }

    #[tokio::test]
    async fn bigger_plan_is_spent_more_than_a_smaller_one() {
        // A larger plan has more headroom → conserved less → used more. (Consumes the initializer
        // subscription type.)
        let r = mixed_router();
        let prompts = complex_workload(80);
        let big =
            subscription_share(&r, &conserve_quota(0.5, "max-20x", "max-20x"), &prompts).await;
        let small = subscription_share(&r, &conserve_quota(0.5, "plus", "plus"), &prompts).await;
        assert!(
            big > small,
            "the bigger plan should be used for more complex tasks: max-20x={big} plus={small}"
        );
    }

    #[tokio::test]
    async fn conservation_disabled_keeps_the_greedy_flagship() {
        // Opt-out (config.mesh.subscription_conserve = false): old behaviour, always the flagship.
        let r = mixed_router();
        let prompts = complex_workload(40);
        let q = SubscriptionQuota::default(); // conserve = false
        let sub = subscription_share(&r, &q, &prompts).await;
        assert_eq!(
            sub,
            prompts.len(),
            "with conservation off every complex task uses a subscription"
        );
    }

    #[tokio::test]
    async fn conservation_never_drops_a_complex_task_onto_a_weak_model() {
        // Guard: when the only frontier-calibre option IS the subscription (no frontier free
        // alternative), conservation must not fire — quality wins over conservation.
        let r = HeuristicRouter::new(Config::default())
            .with_availability(|_| true)
            .with_catalog(ModelCatalog::new(vec![
                "claude-cli::opus".into(),
                "codex-cli::gpt-5.5".into(),
                "groq::llama-3.1-8b-instant".into(), // small, NOT a frontier alternative
            ]));
        let prompts = complex_workload(30);
        let q = conserve_quota(0.7, "plus", "plus"); // high pressure + conserve on
        let sub = subscription_share(&r, &q, &prompts).await;
        assert_eq!(
            sub,
            prompts.len(),
            "no frontier alternative → keep using the subscription"
        );
    }

    #[tokio::test]
    async fn route_hinted_pins_the_given_tier_over_classification() {
        let r = mixed_router();
        // A SHORT prompt the heuristic would classify Trivial, forced Complex by a skill hint.
        let d = r
            .route_hinted(
                "fix typo",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                Some(TaskTier::Complex),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Complex);
        assert!(d.rationale.contains("tier hint"));
        // A None hint behaves exactly like plain route().
        let plain = route_model(&r, "fix typo").await;
        let none_hint = r
            .route_hinted(
                "fix typo",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                None,
                &ProjectContext::default(),
            )
            .await
            .model;
        assert_eq!(plain, none_hint);
    }

    #[tokio::test]
    async fn trivial_tasks_use_a_free_model_to_preserve_subscription_quota() {
        let r = mixed_router();
        for p in [
            "fix this typo in the readme",
            "rename foo to bar",
            "format this file",
        ] {
            let m = route_model(&r, p).await;
            assert!(
                !catalog::is_subscription(&m),
                "trivial '{p}' should route to a free model, not burn subscription: got {m}"
            );
        }
    }

    #[tokio::test]
    async fn complex_tasks_use_the_subscription_flagship() {
        let r = mixed_router();
        for p in [
            "design a lock-free queue and prove it is correct",
            "refactor the auth module to use the new token store",
        ] {
            let d = r
                .route(
                    p,
                    false,
                    BudgetState::default(),
                    &ModelHealth::default(),
                    &SubscriptionQuota::default(),
                    None,
                    &ProjectContext::default(),
                )
                .await;
            assert_eq!(d.tier, TaskTier::Complex, "{p}");
            assert!(
                catalog::is_subscription(&d.model),
                "complex '{p}' should use the subscription flagship: got {}",
                d.model
            );
        }
    }

    #[tokio::test]
    async fn routing_spreads_across_providers_not_only_claude() {
        // The regression this fixes: every task went to claude-cli (alphabetical tie-break).
        let r = mixed_router();
        let prompts = [
            "fix this typo",
            "rename the variable",
            "write a function that validates an email and wire it into signup",
            "add a unit test for the parser",
            "implement a retry wrapper around the http client",
            "refactor the auth module to use the new token store",
            "design a lock-free queue and prove it is correct",
            "debug why the scheduler stalls under load",
            "optimize the hot path in the parser",
            "explain how tokio's scheduler works",
        ];
        let mut providers = std::collections::HashSet::new();
        for p in prompts {
            providers.insert(forge_config::provider_of(&route_model(&r, p).await).to_string());
        }
        // Must use more than one provider, and specifically both subscription bridges + a free one.
        assert!(
            providers.len() >= 3,
            "routing should spread across providers, got {providers:?}"
        );
        assert!(
            providers.contains("claude-cli") && providers.contains("codex-cli"),
            "both subscription bridges should be used across a workload, got {providers:?}"
        );
        assert!(
            providers
                .iter()
                .any(|p| p == "groq" || p == "ollama" || p == "gemini"),
            "a free provider should be used for the easy tasks, got {providers:?}"
        );
    }

    #[tokio::test]
    async fn code_heavy_complex_prefers_a_coding_provider() {
        let r = mixed_router();
        // A code-heavy complex task should land on a coding-tuned provider (codex/claude), not
        // a general free model, via the mild prior + complex subscription preference.
        let m = route_model(
            &r,
            "refactor the auth module and add tests for the token store",
        )
        .await;
        assert!(
            forge_config::provider_of(&m) == "codex-cli"
                || forge_config::provider_of(&m) == "claude-cli",
            "code-heavy complex should use a coding provider: got {m}"
        );
    }

    #[tokio::test]
    async fn exhausted_subscription_is_routed_around() {
        // L3: a subscription at its limit is skipped entirely, like a benched model.
        let r = mixed_router();
        let mut map = std::collections::HashMap::new();
        map.insert(
            "claude-cli".to_string(),
            forge_types::QuotaStatus::Exhausted,
        );
        map.insert("codex-cli".to_string(), forge_types::QuotaStatus::Exhausted);
        let quota = SubscriptionQuota::new(map);
        let d = r
            .route(
                "design a lock-free queue and prove it is correct",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &quota,
                None,
                &ProjectContext::default(),
            )
            .await;
        assert!(
            !catalog::is_subscription(&d.model),
            "both subs exhausted → {}",
            d.model
        );
        assert!(
            !d.fallbacks.iter().any(|m| catalog::is_subscription(m)),
            "exhausted subs absent from the chain too: {:?}",
            d.fallbacks
        );
    }

    #[tokio::test]
    async fn near_limit_subscription_is_demoted_below_alternatives() {
        // L3: a Warning subscription is still usable but ranks behind everything else.
        let r = mixed_router();
        let mut map = std::collections::HashMap::new();
        map.insert("claude-cli".to_string(), forge_types::QuotaStatus::Warning);
        map.insert("codex-cli".to_string(), forge_types::QuotaStatus::Warning);
        let quota = SubscriptionQuota::new(map);
        let d = r
            .route(
                "design a lock-free queue and prove it is correct",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &quota,
                None,
                &ProjectContext::default(),
            )
            .await;
        // Complex normally picks the subscription flagship; under quota pressure a non-subscription
        // model leads instead, with the subscription kept only as a later fallback.
        assert!(
            !catalog::is_subscription(&d.model),
            "near-limit subs demoted below alternatives: got {}",
            d.model
        );
    }

    #[tokio::test]
    async fn weekly_warning_complex_picks_the_best_other_frontier() {
        // User scenario: claude & codex ~80% weekly → a complex task uses the best OTHER
        // available FRONTIER model, not merely any non-subscription model. (80% → Warning.)
        let r = mixed_router();
        let mut map = std::collections::HashMap::new();
        map.insert("claude-cli".to_string(), forge_types::QuotaStatus::Warning);
        map.insert("codex-cli".to_string(), forge_types::QuotaStatus::Warning);
        let quota = SubscriptionQuota::new(map);
        let d = r
            .route(
                "design a lock-free queue and prove it is correct",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &quota,
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Complex);
        assert!(
            !catalog::is_subscription(&d.model),
            "demoted off subscription: {}",
            d.model
        );
        assert!(
            crate::capability::is_frontier(&d.model),
            "complex under weekly pressure must still pick a FRONTIER alternative: got {}",
            d.model
        );
    }

    #[tokio::test]
    async fn fully_exhausted_routes_around_subscriptions_for_every_tier() {
        // User scenario: both subs at 100% weekly/session → use the best other available model
        // for ALL tasks, not just complex ones.
        let r = mixed_router();
        let mut map = std::collections::HashMap::new();
        map.insert(
            "claude-cli".to_string(),
            forge_types::QuotaStatus::Exhausted,
        );
        map.insert("codex-cli".to_string(), forge_types::QuotaStatus::Exhausted);
        let quota = SubscriptionQuota::new(map);
        for p in [
            "fix this typo",                                 // trivial
            "write a function that validates an email",      // standard
            "design a lock-free queue and prove it correct", // complex
        ] {
            let d = r
                .route(
                    p,
                    false,
                    BudgetState::default(),
                    &ModelHealth::default(),
                    &quota,
                    None,
                    &ProjectContext::default(),
                )
                .await;
            assert!(
                !catalog::is_subscription(&d.model),
                "'{p}' ({:?}) must route around exhausted subs: got {}",
                d.tier,
                d.model
            );
            assert!(
                !d.fallbacks.iter().any(|m| catalog::is_subscription(m)),
                "'{p}': exhausted subs must be absent from the failover chain too: {:?}",
                d.fallbacks
            );
        }
    }

    // DIAGNOSTIC (ignored): print what the mesh routes to across a realistic catalog.
    // Run: cargo test -p forge-mesh routing_distribution_diagnostic -- --nocapture --ignored
    #[ignore]
    #[tokio::test]
    async fn routing_distribution_diagnostic() {
        let cat = ModelCatalog::new(vec![
            "claude-cli::".into(),
            "claude-cli::opus".into(),
            "claude-cli::sonnet".into(),
            "claude-cli::haiku".into(),
            "codex-cli::".into(),
            "codex-cli::gpt-5.5".into(),
            "codex-cli::gpt-5.3-codex".into(),
            "codex-cli::gpt-5.4".into(),
            "codex-cli::gpt-5.4-mini".into(),
            "ollama::qwen3-coder:30b".into(),
            "ollama::llama3.2".into(),
            "groq::llama-3.1-8b-instant".into(),
            "groq::llama-3.3-70b-versatile".into(),
            "gemini::gemini-2.5-pro".into(),
            "gemini::gemini-2.5-flash".into(),
        ]);
        let pricing = crate::pricing::Pricing::default();
        println!("\n=== ranked_for (top 6) per tier ===");
        for tier in [TaskTier::Trivial, TaskTier::Standard, TaskTier::Complex] {
            println!(
                "{:<9} {:?}",
                tier.as_str(),
                cat.ranked_for(tier, &pricing, 6)
            );
        }

        let r = HeuristicRouter::new(Config::default())
            .with_availability(|_| true)
            .with_catalog(cat);
        let prompts = [
            "fix this typo in the readme",
            "rename the variable foo to bar",
            "format this file",
            "write a function that validates an email address and wire it into the signup handler",
            "add a unit test for the parser",
            "refactor the auth module to use the new token store",
            "design a lock-free queue and prove it is correct",
            "debug why the mesh routes everything to one provider and propose a fix",
            "explain how tokio's scheduler works",
        ];
        println!("\n=== route() per prompt ===");
        for p in prompts {
            let d = r
                .route(
                    p,
                    false,
                    BudgetState::default(),
                    &ModelHealth::default(),
                    &SubscriptionQuota::default(),
                    None,
                    &ProjectContext::default(),
                )
                .await;
            println!("[{:?}] {} -> {}", d.tier, &p[..p.len().min(46)], d.model);
        }
        println!();
    }

    #[test]
    fn default_classifier_is_llm() {
        assert_eq!(
            forge_config::ClassifierKind::default(),
            forge_config::ClassifierKind::Llm
        );
    }

    #[test]
    fn numbered_build_brief_is_not_trivial_in_heuristic_mode() {
        let brief = "Fix 11 UI bugs in the Forge mobile app.\n1. Fix navigation state.\n2. Repair keyboard dismissal.\n3. Correct loading state.\n4. Fix settings persistence.\n5. Repair deep links.\n6. Correct accessibility labels.\n7. Fix theme switching.\n8. Repair offline recovery.\n9. Fix list rendering.\n10. Correct error handling.\n11. Update tests. Edit multiple files, run tsc, and commit the changes.";
        assert_ne!(
            score_prompt(brief, &ProjectContext::default()).tier,
            TaskTier::Trivial
        );
    }

    #[tokio::test]
    async fn short_prompt_is_trivial() {
        let d = router()
            .route(
                "fix typo",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Trivial);
    }

    // --- Scoring classifier: capability over length (the headline fix) ---

    #[test]
    fn hard_short_prompt_is_complex_despite_length() {
        // "design a lock-free queue" is 24 chars — the old <80 rule called this Trivial.
        assert_eq!(
            score_prompt("design a lock-free queue", &ProjectContext::default()).tier,
            TaskTier::Complex
        );
        assert_eq!(
            score_prompt("prove this sort is stable", &ProjectContext::default()).tier,
            TaskTier::Complex
        );
        assert_eq!(
            score_prompt("debug this deadlock", &ProjectContext::default()).tier,
            TaskTier::Complex
        );
    }

    #[test]
    fn trivial_edit_stays_trivial_even_with_a_path() {
        assert_eq!(
            score_prompt("rename foo to bar in utils.rs", &ProjectContext::default()).tier,
            TaskTier::Trivial
        );
        assert_eq!(
            score_prompt("fix typo", &ProjectContext::default()).tier,
            TaskTier::Trivial
        );
        assert_eq!(
            score_prompt("bump version to 1.2.0", &ProjectContext::default()).tier,
            TaskTier::Trivial
        );
    }

    #[test]
    fn action_and_multistep_is_standard_not_complex() {
        let p = "write a function that validates email addresses against the RFC rules and \
                 returns which inputs were rejected, then wire it into the signup handler";
        assert_eq!(
            score_prompt(p, &ProjectContext::default()).tier,
            TaskTier::Standard
        ); // AC-A3
    }

    #[test]
    fn long_prose_without_signals_is_not_auto_complex() {
        // Length alone is a capped nudge — 200 plain words must not force Complex.
        let p = "word ".repeat(200);
        assert_ne!(
            score_prompt(&p, &ProjectContext::default()).tier,
            TaskTier::Complex
        ); // AC-A7
    }

    #[test]
    fn every_decision_names_a_signal() {
        for p in [
            "fix typo",
            "design a lock-free queue",
            "add a logging helper module",
        ] {
            assert!(
                !score_prompt(p, &ProjectContext::default())
                    .reasons
                    .is_empty(),
                "no reason for {p:?}"
            );
        }
    }

    #[test]
    fn budget_status_thresholds() {
        let mk = |spent| BudgetState {
            spent_today_usd: spent,
            daily_cap_usd: Some(10.0),
            ..Default::default()
        };
        assert_eq!(mk(0.0).status(), BudgetStatus::Ok);
        assert_eq!(mk(7.99).status(), BudgetStatus::Ok);
        assert_eq!(mk(8.0).status(), BudgetStatus::Warning); // 80% of cap
        assert_eq!(mk(9.5).status(), BudgetStatus::Warning);
        assert_eq!(mk(10.0).status(), BudgetStatus::Exhausted);
        assert_eq!(mk(99.0).status(), BudgetStatus::Exhausted);
    }

    #[test]
    fn no_cap_is_always_ok() {
        let b = BudgetState {
            spent_today_usd: 1000.0,
            ..Default::default()
        };
        assert_eq!(b.status(), BudgetStatus::Ok);
    }

    #[test]
    fn stricter_axis_wins() {
        // day Ok, month Exhausted -> Exhausted (AC-8).
        let b = BudgetState {
            spent_today_usd: 1.0,
            daily_cap_usd: Some(100.0),
            spent_week_usd: 0.0,
            weekly_cap_usd: None,
            spent_month_usd: 80.0,
            monthly_cap_usd: Some(80.0),
            warn_fraction: DEFAULT_WARN_FRACTION,
            min_context_tokens: None,
        };
        assert_eq!(b.status(), BudgetStatus::Exhausted);
    }

    #[tokio::test]
    async fn keyword_forces_complex() {
        let d = router()
            .route(
                "refactor the auth module",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Complex);
    }

    #[tokio::test]
    async fn medium_prompt_is_standard() {
        let prompt = "add a new endpoint that returns the list of users as json".repeat(2);
        let d = router()
            .route(
                &prompt,
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Standard);
    }

    #[tokio::test]
    async fn exhausted_budget_downshifts() {
        let budget = BudgetState {
            spent_today_usd: 5.0,
            daily_cap_usd: Some(5.0),
            ..Default::default()
        };
        let d = router()
            .route(
                "refactor the whole architecture",
                false,
                budget,
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Trivial);
        assert!(d.rationale.contains("budget"));
    }

    // --- New: richer signals (AC-5, AC-6, AC-7) ---

    #[tokio::test]
    async fn explicit_think_hard_hint_forces_complex() {
        let d = router()
            .route(
                "rename x; but think hard about edge cases",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Complex); // AC-6
    }

    #[tokio::test]
    async fn fenced_code_is_at_least_standard_despite_short_length() {
        let d = router()
            .route(
                "```rust\nlet x=1;\n```",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Standard); // AC-5
    }

    #[tokio::test]
    async fn dev_verb_lifts_short_prompt_to_standard() {
        let d = router()
            .route(
                "integrate the parser",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Standard);
    }

    #[tokio::test]
    async fn fix_typo_stays_trivial_no_regression() {
        let d = router()
            .route(
                "fix typo",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Trivial); // AC-7
    }

    // --- New: pin / override (AC-1, AC-2) ---

    #[tokio::test]
    async fn pin_overrides_classification() {
        let r = HeuristicRouter::new(Config::default())
            .with_availability(|_| true)
            .with_pin(Some("openai::gpt-4o".into()));
        let d = r
            .route(
                "fix typo",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.model, "openai::gpt-4o"); // AC-1
        assert!(d.rationale.contains("pinned"));
        assert!(
            d.pinned,
            "an explicit --model pin must be flagged as pinned"
        );
        assert!(
            d.fallbacks.is_empty(),
            "strict pins (default): no cross-model fallback chain for a pinned model, got {:?}",
            d.fallbacks
        );
    }

    #[tokio::test]
    async fn pin_honor_tracks_the_public_dispatchable_predicate() {
        // `forge run --model <id>` and the OpenAI-compatible `forge api` endpoint must agree on which
        // explicit pins are honored verbatim. Both consult `pin_is_dispatchable`; this binds that
        // public predicate to the router's ACTUAL pin decision so the two can't drift — the exact
        // divergence that let #509's API fix reject valid models the CLI pin path dispatches fine.
        // No availability override: the router uses the same default (`has_api_key`) the predicate
        // wraps, so both are env-independent here (a keyless provider and an unknown one both resolve
        // "dispatchable" without any key configured).
        for m in ["ollama::llama3.2", "nonexistent::typo-model"] {
            let r = HeuristicRouter::new(Config::default()).with_pin(Some(m.to_string()));
            let d = r
                .route(
                    "fix typo",
                    false,
                    BudgetState::default(),
                    &ModelHealth::default(),
                    &SubscriptionQuota::default(),
                    None,
                    &ProjectContext::default(),
                )
                .await;
            let honored_verbatim = d.pinned && d.model == m;
            assert_eq!(
                honored_verbatim,
                pin_is_dispatchable(m),
                "router pin-honor for '{m}' must equal pin_is_dispatchable('{m}') — the shared rule"
            );
        }
    }

    #[tokio::test]
    async fn pin_failover_escape_hatch_keeps_the_fallback_chain() {
        // `mesh.pin_failover = true` restores the pre-wave-2 behaviour: a pinned decision keeps
        // the mesh fallback chain so a failing pin may still switch away mid-turn.
        let mut config = Config::default();
        config.mesh.pin_failover = true;
        let r = HeuristicRouter::new(config)
            .with_availability(|_| true)
            .with_pin(Some("openai::gpt-4o".into()));
        let d = r
            .route(
                "fix typo",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.model, "openai::gpt-4o");
        assert!(d.pinned);
        assert!(
            !d.fallbacks.is_empty(),
            "escape hatch keeps the old pin fallback chain"
        );
    }

    #[tokio::test]
    async fn exhausted_budget_overrides_pin() {
        // hard_stop is enforced pre-routing in core; here cap_overrides_pin governs.
        let mut config = Config::default();
        config.mesh.budget.cap_overrides_pin = true;
        let r = HeuristicRouter::new(config)
            .with_availability(|_| true)
            .with_pin(Some("anthropic::claude-opus-4-8".into()));
        let budget = BudgetState {
            spent_today_usd: 5.0,
            daily_cap_usd: Some(5.0),
            ..Default::default()
        };
        let d = r
            .route(
                "design a system",
                false,
                budget,
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        // pin ignored; trivial-tier model chosen (AC-2)
        assert_eq!(
            d.model,
            Config::default().model_for(TaskTier::Trivial).unwrap()
        );
        assert_ne!(d.model, "anthropic::claude-opus-4-8");
    }

    // --- New: provider fallback (AC-3, AC-4) ---

    #[tokio::test]
    async fn falls_back_to_an_available_model_when_key_missing() {
        // Only ollama (the trivial-tier default) is "available"; complex (anthropic) is not.
        let r =
            HeuristicRouter::new(Config::default()).with_availability(|m| m.starts_with("ollama"));
        let d = r
            .route(
                "design the architecture",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Complex, "tier still reflects difficulty");
        assert!(
            d.model.starts_with("ollama"),
            "fell back to a usable model: {}",
            d.model
        );
        assert!(d.rationale.contains("fell back"), "{}", d.rationale);
    }

    #[tokio::test]
    async fn no_usable_model_keeps_original_and_warns() {
        // Nothing available → keep the routed model (errors downstream as today).
        let r = HeuristicRouter::new(Config::default()).with_availability(|_| false);
        let d = r
            .route(
                "design the architecture",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(
            d.model,
            Config::default().model_for(TaskTier::Complex).unwrap()
        ); // AC-4
        assert!(d.rationale.contains("no usable key"));
    }

    // --- Cost-aware selection (L1) + subscription-first (L2) ---

    fn list_config(tier: &str, models: &[&str]) -> Config {
        let mut c = Config::default();
        c.mesh.models.insert(
            tier.to_string(),
            forge_config::OneOrMany::Many(models.iter().map(|s| s.to_string()).collect()),
        );
        c
    }

    #[test]
    fn cheapest_usable_picks_lowest_estimated_cost() {
        // gpt-4o-mini (~$0.00045/turn) is cheaper than deepseek-chat (~$0.00082/turn).
        let r = HeuristicRouter::new(Config::default()).with_availability(|_| true);
        let cands = vec![
            "deepseek::deepseek-chat".to_string(),
            "openai::gpt-4o-mini".to_string(),
        ];
        assert_eq!(
            r.cheapest_usable(&cands, &ModelHealth::default()).unwrap(),
            "openai::gpt-4o-mini"
        ); // AC-L1a
    }

    #[test]
    fn cheapest_usable_skips_models_without_a_key() {
        // ollama is "cheapest" ($0) but unavailable here → the usable openai wins.
        let r =
            HeuristicRouter::new(Config::default()).with_availability(|m| !m.starts_with("ollama"));
        let cands = vec![
            "ollama::free".to_string(),
            "openai::gpt-4o-mini".to_string(),
        ];
        assert_eq!(
            r.cheapest_usable(&cands, &ModelHealth::default()).unwrap(),
            "openai::gpt-4o-mini"
        ); // AC-L1b
    }

    #[tokio::test]
    async fn route_picks_cheapest_standard_candidate_with_rationale() {
        let c = list_config(
            "standard",
            &["deepseek::deepseek-chat", "openai::gpt-4o-mini"],
        );
        let r = HeuristicRouter::new(c).with_availability(|_| true);
        let prompt = "add a new endpoint that returns the list of users as json".repeat(2);
        let d = r
            .route(
                &prompt,
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Standard);
        assert_eq!(d.model, "openai::gpt-4o-mini");
        assert!(d.rationale.contains("cheapest of 2"), "{}", d.rationale);
    }

    #[tokio::test]
    async fn auto_discovery_routes_to_the_capability_ranked_catalog_model() {
        // Auto-discovery on (default) + a catalog → the mesh ranks by capability (cost folded in),
        // NOT pure cheapest, so a Complex task picks the frontier model over a tiny free one.
        let cat = ModelCatalog::new(vec![
            "groq::llama-3.1-8b-instant".into(),
            "anthropic::claude-opus-4-8".into(),
        ]);
        let r = HeuristicRouter::new(Config::default())
            .with_availability(|_| true)
            .with_catalog(cat);
        let prompt = "design and architect a complex concurrency refactor across modules";
        let d = r
            .route(
                prompt,
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Complex);
        assert_eq!(d.model, "anthropic::claude-opus-4-8", "{}", d.rationale);
        assert!(d.rationale.contains("auto-selected"), "{}", d.rationale);
    }

    #[tokio::test]
    async fn auto_discovery_trivial_prefers_the_small_fast_model() {
        let cat = ModelCatalog::new(vec![
            "groq::llama-3.1-8b-instant".into(),
            "anthropic::claude-opus-4-8".into(),
        ]);
        let r = HeuristicRouter::new(Config::default())
            .with_availability(|_| true)
            .with_catalog(cat);
        let d = r
            .route(
                "fix typo",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Trivial);
        assert_eq!(d.model, "groq::llama-3.1-8b-instant", "{}", d.rationale);
    }

    #[tokio::test]
    async fn auto_discovery_off_uses_configured_candidates() {
        // With auto off, the catalog is ignored and the configured tier wins (manual override).
        let mut config = Config::default();
        config.mesh.auto_discover = false;
        config.mesh.models.insert(
            "complex".to_string(),
            forge_config::OneOrMany::One("openai::gpt-4o-mini".to_string()),
        );
        let r = HeuristicRouter::new(config)
            .with_availability(|_| true)
            .with_catalog(ModelCatalog::new(vec!["anthropic::claude-opus-4-8".into()]));
        let d = r
            .route(
                "design and architect a complex concurrency refactor across modules",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.model, "openai::gpt-4o-mini", "{}", d.rationale);
    }

    #[tokio::test]
    async fn legacy_single_string_tier_routes_unchanged() {
        // AC-L1c: the single-string form behaves as a one-candidate list. (Built explicitly —
        // the shipped defaults now lead each tier with free multi-candidate lists.)
        let mut c = Config::default();
        c.mesh.models.insert(
            "standard".to_string(),
            forge_config::OneOrMany::One("openai::gpt-4o-mini".to_string()),
        );
        let r = HeuristicRouter::new(c).with_availability(|_| true);
        let prompt = "add a new endpoint that returns the list of users as json".repeat(2);
        let d = r
            .route(
                &prompt,
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.model, "openai::gpt-4o-mini");
    }

    #[tokio::test]
    async fn subscription_is_preferred_when_enabled() {
        // AC-L2a: a $0 paid subscription (CLI bridge) wins over a metered API model.
        let r = HeuristicRouter::new(list_config(
            "complex",
            &["anthropic::claude-opus-4-8", "claude-cli::"],
        ))
        .with_availability(|_| true);
        let d = r
            .route(
                "design the system architecture carefully",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.model, "claude-cli::");
        assert!(d.rationale.contains("paid subscription"), "{}", d.rationale);
    }

    #[tokio::test]
    async fn subscription_still_cheapest_when_preference_disabled() {
        // prefer_subscription off → pure cost ranking; the $0 bridge is still cheapest, but the
        // rationale no longer flags it as a subscription.
        let mut c = list_config("complex", &["anthropic::claude-opus-4-8", "claude-cli::"]);
        c.mesh.prefer_subscription = false;
        let r = HeuristicRouter::new(c).with_availability(|_| true);
        let d = r
            .route(
                "design the system architecture carefully",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.model, "claude-cli::");
        assert!(!d.rationale.contains("paid subscription"));
    }

    #[test]
    fn cost_rank_prefers_every_subscription_surface() {
        // Fix 1: `cost_rank` used to build its ranking key from forge-mesh's private
        // `is_subscription`, which only knew the three CLI bridges. Every call site (incl.
        // `cost_rank`) now delegates to the public `catalog::is_subscription`, which covers all
        // all subscription surfaces — so with `prefer_subscription` on, OAuth and API-key plans
        // must sort
        // rank 0 (preferred), same as the CLI bridges, not rank 1 (behind) as before.
        let mut c = Config::default();
        c.mesh.prefer_subscription = true;
        let r = HeuristicRouter::new(c).with_availability(|_| true);
        for id in [
            "claude-cli::opus",
            "codex-cli::gpt-5.5",
            "agy-cli::gemini-pro",
            "codex-oauth::gpt-5.6-sol",
            "xai-oauth::grok-4",
            "qwencloud::qwen3.8-max-preview",
        ] {
            assert_eq!(
                r.cost_rank(id).0,
                0,
                "{id} must rank as preferred-subscription (tier 0) under prefer_subscription"
            );
        }
        assert_eq!(
            r.cost_rank("openai::gpt-4o-mini").0,
            1,
            "a metered API model must not rank as preferred-subscription"
        );
    }

    // --- Model health / failover ---

    fn benched(models: &[&str]) -> ModelHealth {
        ModelHealth::new(models.iter().map(|s| s.to_string()).collect())
    }

    #[tokio::test]
    async fn disabled_models_are_filtered_from_live_routing() {
        let mut config = Config::default();
        config.mesh.disabled = vec!["anthropic".into()];
        let r = HeuristicRouter::new(config)
            .with_availability(|_| true)
            .with_catalog(ModelCatalog::new(vec![
                "anthropic::claude-opus-4-8".into(),
                "groq::llama-3.1-8b-instant".into(),
            ]));
        let d = r
            .route(
                "design and architect a complex concurrency refactor across modules",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;

        assert_ne!(d.model, "anthropic::claude-opus-4-8");
        assert!(
            !d.fallbacks
                .iter()
                .any(|model| model == "anthropic::claude-opus-4-8"),
            "disabled model leaked into failover chain: {:?}",
            d.fallbacks
        );
    }

    #[tokio::test]
    async fn unavailable_explicit_pin_is_not_silently_rerouted() {
        let r = HeuristicRouter::new(Config::default())
            .with_availability(|model| model != "openai::gpt-4o")
            .with_pin(Some("openai::gpt-4o".into()));
        let d = r
            .route(
                "fix typo",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;

        assert_eq!(d.model, "openai::gpt-4o");
        assert!(d.pinned);
        assert!(d.rationale.contains("unavailable"));
    }

    #[tokio::test]
    async fn benched_model_is_skipped_and_next_best_chosen() {
        // Auto-discovery ranks opus #1 for Complex; bench it → the next usable model wins (AC-3).
        let cat = ModelCatalog::new(vec![
            "anthropic::claude-opus-4-8".into(),
            "groq::llama-3.1-8b-instant".into(),
        ]);
        let r = HeuristicRouter::new(Config::default())
            .with_availability(|_| true)
            .with_catalog(cat);
        let prompt = "design and architect a complex concurrency refactor across modules";
        let d = r
            .route(
                prompt,
                false,
                BudgetState::default(),
                &benched(&["anthropic::claude-opus-4-8"]),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Complex);
        assert_ne!(
            d.model, "anthropic::claude-opus-4-8",
            "benched model must not be chosen"
        );
        assert!(
            !d.fallbacks
                .contains(&"anthropic::claude-opus-4-8".to_string()),
            "benched model must not appear as a fallback: {:?}",
            d.fallbacks
        );
    }

    #[tokio::test]
    async fn decision_carries_an_ordered_failover_chain_excluding_the_pick() {
        let cat = ModelCatalog::new(vec![
            "anthropic::claude-opus-4-8".into(),
            "groq::llama-3.1-8b-instant".into(),
        ]);
        let r = HeuristicRouter::new(Config::default())
            .with_availability(|_| true)
            .with_catalog(cat);
        let d = r
            .route(
                "design and architect a complex concurrency refactor across modules",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert!(
            !d.fallbacks.is_empty(),
            "expected a non-empty failover chain"
        );
        assert!(
            !d.fallbacks.contains(&d.model),
            "the pick must not also be a fallback"
        );
    }

    #[tokio::test]
    async fn all_benched_falls_through_to_the_no_fallback_warning() {
        // Every model benched → behaves like nothing usable (AC-6 surfaces downstream).
        let r = HeuristicRouter::new(Config::default()).with_availability(|_| true);
        let everything = HeuristicRouter::new(Config::default()).candidates_for_tier(
            TaskTier::Complex,
            RouteHints::default(),
            &SubscriptionQuota::default(),
            None,
        );
        let refs: Vec<&str> = everything.iter().map(String::as_str).collect();
        // Bench the complex candidates AND the cross-tier ones by benching all configured tiers.
        let mut all: Vec<String> = Vec::new();
        for t in [TaskTier::Complex, TaskTier::Standard, TaskTier::Trivial] {
            all.extend(HeuristicRouter::new(Config::default()).candidates_for_tier(
                t,
                RouteHints::default(),
                &SubscriptionQuota::default(),
                None,
            ));
        }
        let all_refs: Vec<&str> = all.iter().map(String::as_str).collect();
        let _ = refs; // (kept for clarity; all_refs is the superset used below)
        let d = r
            .route(
                "design the architecture",
                false,
                BudgetState::default(),
                &benched(&all_refs),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert!(d.fallbacks.is_empty());
        assert!(d.rationale.contains("no usable key"), "{}", d.rationale);
    }

    // --- Classification signal coverage ---

    #[test]
    fn investigation_terms_are_complex() {
        for p in [
            "investigate why the cache warms slowly",
            "audit the permission checks in the auth module",
            "diagnose the memory issue in the worker process",
            "evaluate the design of the new token store API",
            "is there a vulnerability in this authentication code",
            "there is a memory leak in the connection pool",
        ] {
            assert_eq!(
                score_prompt(p, &ProjectContext::default()).tier,
                TaskTier::Complex,
                "expected Complex for {p:?}"
            );
        }
    }

    #[test]
    fn analysis_terms_alone_lift_to_standard_not_trivial() {
        for p in [
            "review the authentication flow",
            "check the performance of this endpoint",
            "compare these two data structures",
            "help me understand how the scheduler works",
            "is there a security issue here",
            "find the bottleneck in the rendering path",
        ] {
            let tier = score_prompt(p, &ProjectContext::default()).tier;
            assert_ne!(
                tier,
                TaskTier::Trivial,
                "expected Standard or Complex for {p:?}, got Trivial"
            );
        }
    }

    #[test]
    fn combined_analysis_terms_reach_complex() {
        // Two ANALYSIS_TERMS signals (3+3 = 6 ≥ 5) → Complex.
        assert_eq!(
            score_prompt(
                "there is a performance bottleneck in the hot path",
                &ProjectContext::default()
            )
            .tier,
            TaskTier::Complex,
            "performance + bottleneck → Complex"
        );
        // ANALYSIS_TERM + reasoning term → Complex.
        assert_eq!(
            score_prompt(
                "review and analyze the trade-offs in this design",
                &ProjectContext::default()
            )
            .tier,
            TaskTier::Complex,
            "review + analyze + trade-off → Complex"
        );
        // ANALYSIS_TERM + code → 3+3 = 6 → Complex.
        assert_eq!(
            score_prompt(
                "security review of this ```rust\nfn login() {}\n```",
                &ProjectContext::default()
            )
            .tier,
            TaskTier::Complex,
            "security + code → Complex"
        );
    }

    #[test]
    fn depth_hints_force_complex_regardless_of_prompt_length() {
        for p in [
            "explain this in depth",
            "give an in-depth analysis",
            "deep dive into the scheduler",
            "comprehensive review of the auth module",
            "do a thorough audit of the codebase",
        ] {
            assert_eq!(
                HeuristicRouter::classify(p, &ProjectContext::default()).0,
                TaskTier::Complex,
                "depth hint must force Complex for {p:?}"
            );
        }
    }

    #[test]
    fn minor_qualifier_cancels_complexity_signal() {
        // "minor" (−5) cancels a REASONING_TERM (+5) → net 0 → Trivial.
        assert_eq!(
            score_prompt(
                "minor refactor of this helper function",
                &ProjectContext::default()
            )
            .tier,
            TaskTier::Trivial,
            "minor + refactor: trivial qualifier must win"
        );
        // "small fix" (−5) cancels a reasoning term.
        assert_eq!(
            score_prompt("small fix for the debug output", &ProjectContext::default()).tier,
            TaskTier::Trivial,
            "small fix + debug: trivial qualifier must win"
        );
        // "briefly" (−5) cancels "explain" (+5).
        assert_eq!(
            score_prompt("briefly explain this function", &ProjectContext::default()).tier,
            TaskTier::Trivial,
            "briefly + explain: trivial qualifier must win"
        );
    }

    #[test]
    fn port_and_convert_are_standard_action_verbs() {
        assert_ne!(
            score_prompt(
                "port this Python module to Rust",
                &ProjectContext::default()
            )
            .tier,
            TaskTier::Trivial,
            "porting is non-trivial work"
        );
        assert_ne!(
            score_prompt(
                "convert the callback API to async",
                &ProjectContext::default()
            )
            .tier,
            TaskTier::Trivial,
            "conversion is non-trivial work"
        );
    }

    #[test]
    fn report_and_export_do_not_falsely_match_the_port_action_verb() {
        // Regression: "port " (an ACTION_VERBS entry, to catch "port this module to Rust") is a
        // substring of "report " and "export ", so naive `str::contains` gave these common words
        // a spurious "dev-action verb" point and marked them code_heavy — unrelated to porting.
        assert!(
            !is_code_heavy("please generate a report for the crash"),
            "\"report\" must not match the \"port \" action verb"
        );
        assert!(
            !is_code_heavy("export the data to csv"),
            "\"export\" must not match the \"port \" action verb"
        );
        for p in [
            "please generate a report for the crash",
            "export the data to csv",
        ] {
            assert!(
                !score_prompt(p, &ProjectContext::default())
                    .reasons
                    .contains(&"dev-action verb"),
                "{p:?} must not score a dev-action verb point: {:?}",
                score_prompt(p, &ProjectContext::default()).reasons
            );
        }
    }

    #[test]
    fn latest_fastest_and_contest_do_not_falsely_match_test() {
        // Regression: `lower.contains("test")` also matched inside "latest"/"fastest"/"contest",
        // spuriously adding a "tests/edge-cases" point unrelated to actual testing.
        for p in [
            "what is the latest version of this crate",
            "pick the fastest algorithm here",
            "there was a contest about this last year",
        ] {
            assert!(
                !score_prompt(p, &ProjectContext::default())
                    .reasons
                    .contains(&"tests/edge-cases"),
                "{p:?} must not score a tests/edge-cases point: {:?}",
                score_prompt(p, &ProjectContext::default()).reasons
            );
        }
        // Control: a real "test" mention still scores the point.
        assert!(
            score_prompt("please add a test for this", &ProjectContext::default())
                .reasons
                .contains(&"tests/edge-cases")
        );
    }

    #[test]
    fn multistep_with_parenthesised_numbers_is_detected() {
        let p = "1) add the migration 2) update the handler 3) write tests";
        assert!(is_multistep(&p.to_lowercase()), "1) 2) format not detected");
    }

    #[test]
    fn new_trivial_patterns_stay_trivial() {
        for p in [
            "update the version to 2.0.0",
            "change the version in Cargo.toml",
            "delete this line from the config",
            "remove this line and nothing else",
        ] {
            assert_eq!(
                score_prompt(p, &ProjectContext::default()).tier,
                TaskTier::Trivial,
                "expected Trivial for {p:?}"
            );
        }
    }

    // --- /duel: route_candidates + repo_boosts (feature: model arena with routing learning) ---

    #[tokio::test]
    async fn route_candidates_returns_distinct_providers_up_to_n() {
        let r = mixed_router();
        let cands = r
            .route_candidates(
                "implement pagination for the /users endpoint",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
                3,
            )
            .await;
        assert!(
            cands.len() >= 2 && cands.len() <= 3,
            "expected 2-3 candidates, got {}: {:?}",
            cands.len(),
            cands.iter().map(|d| &d.model).collect::<Vec<_>>()
        );
        let providers: std::collections::HashSet<&str> = cands
            .iter()
            .map(|d| forge_config::provider_of(&d.model))
            .collect();
        assert_eq!(
            providers.len(),
            cands.len(),
            "every candidate must be a different provider: {:?}",
            cands.iter().map(|d| &d.model).collect::<Vec<_>>()
        );
        for d in &cands {
            assert!(d.rationale.contains("duel candidate"));
        }
    }

    #[tokio::test]
    async fn route_candidates_default_impl_falls_back_to_a_single_route() {
        // A `Router` with no override (the trait default) must still satisfy `/duel`'s "at least
        // one candidate" contract — proves the default doesn't panic / return empty.
        struct Trivial;
        #[async_trait]
        impl Router for Trivial {
            async fn route(
                &self,
                _prompt: &str,
                _has_images: bool,
                _budget: BudgetState,
                _health: &ModelHealth,
                _quota: &SubscriptionQuota,
                _effort: Option<EffortLevel>,
                _project: &ProjectContext,
            ) -> RoutingDecision {
                RoutingDecision {
                    tier: TaskTier::Standard,
                    model: "fixed::model".into(),
                    rationale: "fixed".into(),
                    fallbacks: vec![],
                    pinned: false,
                }
            }
        }
        let cands = Trivial
            .route_candidates(
                "anything",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
                3,
            )
            .await;
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].model, "fixed::model");
    }

    #[tokio::test]
    async fn repo_boosts_float_a_winning_model_above_equally_ranked_peers() {
        let mut c = Config::default();
        c.mesh.models.insert(
            TaskTier::Standard.as_str().into(),
            forge_config::OneOrMany::Many(vec![
                "provA::one".to_string(),
                "provB::two".to_string(),
                "provC::three".to_string(),
            ]),
        );
        let prompt = "add a retry-with-backoff wrapper around the http client";

        // Baseline: no boosts → configured order (cheapest-first with equal cost = config order).
        let plain = HeuristicRouter::new(c.clone()).with_availability(|_| true);
        let baseline = route_model(&plain, prompt).await;
        assert_eq!(baseline, "provA::one", "baseline should keep config order");

        // Boost the third model above the other two → it must now win.
        let mut boosts = std::collections::HashMap::new();
        boosts.insert("provC::three".to_string(), 2.0);
        let boosted = HeuristicRouter::new(c.clone())
            .with_availability(|_| true)
            .with_repo_boosts(boosts);
        let winner = route_model(&boosted, prompt).await;
        assert_eq!(
            winner, "provC::three",
            "boosted model must float to the top"
        );

        // An unboosted router must be unaffected by an EMPTY boost map (no-op).
        let empty_boosted = HeuristicRouter::new(c)
            .with_availability(|_| true)
            .with_repo_boosts(std::collections::HashMap::new());
        assert_eq!(route_model(&empty_boosted, prompt).await, baseline);
    }

    // --- Part A: route around image-incapable models (vision routing) ---

    #[test]
    fn has_images_filters_candidates_to_vision_capable_models() {
        let mut c = Config::default();
        c.mesh.models.insert(
            TaskTier::Standard.as_str().into(),
            forge_config::OneOrMany::Many(vec![
                "textonly::model-a".to_string(),
                "anthropic::claude-opus-4-8".to_string(),
            ]),
        );
        let r = HeuristicRouter::new(c).with_availability(|_| true);
        let hints = RouteHints::default();

        // Baseline (no images): the first configured candidate wins, same as today.
        let no_images = r.decide(
            TaskTier::Standard,
            "test".into(),
            BudgetState::default(),
            &ModelHealth::default(),
            hints,
            &SubscriptionQuota::default(),
            None,
            false,
        );
        assert_eq!(no_images.model, "textonly::model-a");

        // With images attached, the mesh must route to the vision-capable candidate instead.
        let with_images = r.decide(
            TaskTier::Standard,
            "test".into(),
            BudgetState::default(),
            &ModelHealth::default(),
            hints,
            &SubscriptionQuota::default(),
            None,
            true,
        );
        assert!(
            catalog::supports_vision(&with_images.model),
            "has_images=true must pick a vision-capable model: {}",
            with_images.model
        );
        assert_eq!(with_images.model, "anthropic::claude-opus-4-8");
    }

    #[test]
    fn has_images_fails_open_when_no_vision_candidate_is_usable() {
        // Every configured candidate is text-only — has_images must NOT refuse to route; it
        // falls back to the unfiltered list rather than leaving the turn with no model at all.
        let mut c = Config::default();
        c.mesh.models.insert(
            TaskTier::Standard.as_str().into(),
            forge_config::OneOrMany::Many(vec![
                "textonly::model-a".to_string(),
                "textonly::model-b".to_string(),
            ]),
        );
        let r = HeuristicRouter::new(c).with_availability(|_| true);
        let hints = RouteHints::default();
        let d = r.decide(
            TaskTier::Standard,
            "test".into(),
            BudgetState::default(),
            &ModelHealth::default(),
            hints,
            &SubscriptionQuota::default(),
            None,
            true,
        );
        assert_eq!(
            d.model, "textonly::model-a",
            "fail-open: still routes even though no candidate supports vision"
        );
    }
}
