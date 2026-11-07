//! Layer 1 of the outage drill: a provider that goes down and comes back on a virtual clock.
//!
//! Every test runs with `start_paused`, so a four hour outage costs milliseconds. The provider
//! returns the exact errors the real classifier produces for a gateway `503 no_capacity` and for
//! a refused connection, and switches back to healthy a fixed span after the outage began.

use super::*;
use std::time::Duration;

#[derive(Clone, Copy)]
enum Fault {
    Busy { retry_after_secs: Option<u64> },
    Refused,
    Mixed,
}

struct OutageProvider {
    /// Healthy requests served before the outage begins.
    healthy_first: usize,
    outage: Duration,
    fault: Fault,
    /// A model that answers `Auth` instead of the outage fault.
    auth_for: Option<String>,
    began: Mutex<Option<tokio::time::Instant>>,
    script: Vec<(String, Vec<forge_types::ToolCall>)>,
    served: std::sync::atomic::AtomicUsize,
    failed: std::sync::atomic::AtomicUsize,
    /// Per healthy request: how many tool results the transcript it carried held, and the model.
    seen: Mutex<Vec<(usize, String)>>,
}

impl OutageProvider {
    fn new(healthy_first: usize, outage: Duration, fault: Fault) -> Self {
        Self {
            healthy_first,
            outage,
            fault,
            auth_for: None,
            began: Mutex::new(None),
            script: vec![("DRILL-OK".into(), vec![])],
            served: Default::default(),
            failed: Default::default(),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn with_script(mut self, script: Vec<(&str, Vec<forge_types::ToolCall>)>) -> Self {
        self.script = script.into_iter().map(|(t, c)| (t.into(), c)).collect();
        self
    }

    fn failed(&self) -> usize {
        self.failed.load(std::sync::atomic::Ordering::SeqCst)
    }
}

fn refused() -> forge_provider::ProviderError {
    forge_provider::ProviderError::Unavailable(
        "could not connect: error sending request for url \
         (http://127.0.0.1:1/v1/chat/completions): tcp connect error: Connection refused \
         (os error 111)"
            .into(),
    )
}

fn no_capacity(retry_after_secs: Option<u64>) -> forge_provider::ProviderError {
    forge_provider::ProviderError::Overloaded {
        status: 503,
        reason: "no_capacity".into(),
        message: "No capacity available for cb/deepseek-v4.1-flash".into(),
        retry_after: retry_after_secs.map(Duration::from_secs),
    }
}

#[async_trait::async_trait]
impl Provider for OutageProvider {
    async fn complete(
        &self,
        model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let served = self.served.load(std::sync::atomic::Ordering::SeqCst);
        if served >= self.healthy_first {
            let now = tokio::time::Instant::now();
            let began = *self.began.lock().unwrap().get_or_insert(now);
            if now.duration_since(began) < self.outage {
                let n = self
                    .failed
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if self.auth_for.as_deref() == Some(model) {
                    return Err(forge_provider::ProviderError::Auth("bad key".into()));
                }
                return Err(match self.fault {
                    Fault::Busy { retry_after_secs } => no_capacity(retry_after_secs),
                    Fault::Refused => refused(),
                    Fault::Mixed if n.is_multiple_of(2) => no_capacity(None),
                    Fault::Mixed => refused(),
                });
            }
        }
        self.served
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let tool_results = messages.iter().filter(|m| m.role == Role::Tool).count();
        self.seen
            .lock()
            .unwrap()
            .push((tool_results, model.to_string()));
        let (content, tool_calls) = self
            .script
            .get(served)
            .or_else(|| self.script.last())
            .cloned()
            .unwrap_or_default();
        on_event(StreamEvent::Text(content.clone()));
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content,
            tool_calls,
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

struct Drill {
    session: Session,
    store: Arc<Store>,
    events: Arc<Mutex<Vec<PresenterEvent>>>,
}

fn drill(provider: Arc<OutageProvider>, router: Arc<dyn Router>, outage_wait_secs: u64) -> Drill {
    let (store, mut session) = fixed_session(provider, router);
    session.config.mesh.outage_wait_secs = outage_wait_secs;
    let capture = CapturePresenter::default();
    let events = capture.events.clone();
    session.presenter = Box::new(capture);
    Drill {
        session,
        store,
        events,
    }
}

fn pinned(model: &str) -> Arc<dyn Router> {
    Arc::new(PinnedRouter {
        model: model.into(),
        fallbacks: vec![],
    })
}

const ALLOWANCE: u64 = 8 * 60 * 60;

impl Drill {
    fn prompts(&self) -> Vec<String> {
        self.store
            .load_messages(&self.session.id)
            .unwrap()
            .into_iter()
            .filter(|m| m.role == Role::User)
            .map(|m| m.content)
            .collect()
    }

    fn notes(&self) -> Vec<String> {
        self.store
            .load_messages(&self.session.id)
            .unwrap()
            .into_iter()
            .filter(|m| m.role == Role::System && m.visibility == forge_types::Visibility::UiOnly)
            .map(|m| m.content)
            .collect()
    }

    fn warnings(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                PresenterEvent::Warning(w) => Some(w.clone()),
                _ => None,
            })
            .collect()
    }

    fn outage_events(&self) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, PresenterEvent::ProviderOutage { .. }))
            .count()
    }
}

fn mins(n: u64) -> Duration {
    Duration::from_secs(n * 60)
}

#[tokio::test(start_paused = true)]
async fn t1_a_four_minute_503_is_ridden_out_by_the_hot_backoff() {
    let provider = Arc::new(OutageProvider::new(
        0,
        mins(4),
        Fault::Busy {
            retry_after_secs: None,
        },
    ));
    let mut d = drill(provider, pinned("drill::flash"), ALLOWANCE);
    let out = d.session.run_turn("fix it").await.unwrap();
    assert_eq!(out.text, "DRILL-OK");
    assert!(
        d.warnings()
            .iter()
            .any(|w| w.contains("provider busy (503 no_capacity), retrying in")),
        "{:?}",
        d.warnings()
    );
    assert!(d
        .events
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, PresenterEvent::ModelSearch { retrying: true, .. })));
    assert_eq!(
        d.outage_events(),
        0,
        "the long wait is not entered for a blip"
    );
    assert!(d.notes().iter().all(|n| !n.starts_with('⏸')));
}

#[tokio::test(start_paused = true)]
async fn t2_a_refused_connection_is_waited_out_not_called_a_rejection() {
    // Before the fix the second identical sub-two-second error failed the turn as "its provider
    // rejected it twice" after one 5s backoff.
    let provider = Arc::new(OutageProvider::new(0, mins(2), Fault::Refused));
    let mut d = drill(provider.clone(), pinned("drill::flash"), ALLOWANCE);
    let out = d.session.run_turn("fix it").await.unwrap();
    assert_eq!(out.text, "DRILL-OK");
    assert!(
        provider.failed() >= 3,
        "kept retrying: {}",
        provider.failed()
    );
    assert!(
        d.warnings().iter().all(|w| !w.contains("rejected")),
        "{:?}",
        d.warnings()
    );
}

#[tokio::test(start_paused = true)]
async fn t3_a_four_hour_outage_resumes_the_same_turn() {
    let outage = mins(4 * 60 + 9);
    let provider = Arc::new(OutageProvider::new(
        0,
        outage,
        Fault::Busy {
            retry_after_secs: None,
        },
    ));
    let mut d = drill(provider.clone(), pinned("drill::flash"), ALLOWANCE);
    let started = tokio::time::Instant::now();
    let out = d.session.run_turn("fix it").await.unwrap();
    assert_eq!(out.text, "DRILL-OK");
    assert_eq!(out.stop_reason, StopReason::FinalAnswer);
    let waited = started.elapsed();
    assert!(
        waited >= outage && waited <= outage + mins(5),
        "resumed within one probe of the recovery: {waited:?}"
    );
    let notes = d.notes();
    let entered: Vec<_> = notes.iter().filter(|n| n.starts_with('⏸')).collect();
    let back: Vec<_> = notes.iter().filter(|n| n.starts_with('▶')).collect();
    assert_eq!(entered.len(), 1, "{notes:?}");
    assert!(
        entered[0].contains("drill::flash: provider down since")
            && entered[0].contains("503 no_capacity")
            && entered[0].contains("retrying every <=5 min until")
            && entered[0].ends_with("Esc to stop."),
        "{}",
        entered[0]
    );
    assert_eq!(back.len(), 1, "{notes:?}");
    assert!(
        back[0].contains("drill::flash back after 4h"),
        "{}",
        back[0]
    );
    let probes = d.outage_events();
    assert!(
        (40..=80).contains(&probes),
        "about one probe per five minutes: {probes}"
    );
    assert!(d.events.lock().unwrap().iter().any(|e| matches!(
        e,
        PresenterEvent::ProviderRecovered { down_secs, .. } if *down_secs >= 4 * 3600
    )));
}

#[tokio::test(start_paused = true)]
async fn t3b_retry_after_one_and_the_attempt_cap_no_longer_end_the_turn() {
    // Forty one-second retries used to exhaust OVERLOAD_MAX_ATTEMPTS inside a minute.
    let provider = Arc::new(OutageProvider::new(
        0,
        mins(30),
        Fault::Busy {
            retry_after_secs: Some(1),
        },
    ));
    let mut d = drill(provider, pinned("drill::flash"), ALLOWANCE);
    let out = d.session.run_turn("fix it").await.unwrap();
    assert_eq!(out.text, "DRILL-OK");
    assert!(d.outage_events() > 0);
}

#[tokio::test(start_paused = true)]
async fn t3c_the_horizon_ends_the_turn_as_provider_down() {
    let provider = Arc::new(OutageProvider::new(0, mins(4 * 60), Fault::Refused));
    let mut d = drill(provider, pinned("drill::flash"), 3600);
    let started = tokio::time::Instant::now();
    let err = d.session.run_turn("fix it").await.unwrap_err();
    assert!(matches!(err, CoreError::ProviderDown { .. }), "{err}");
    assert_eq!(err.stop_reason(), StopReason::ProviderDown);
    assert!(err.to_string().contains("still down after 1h00m"), "{err}");
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_secs(3600) && waited < mins(70),
        "{waited:?}"
    );
    assert!(d.notes().iter().all(|n| !n.starts_with('▶')));
    assert_eq!(
        d.session.last_end_reason().as_deref(),
        Some("provider_down")
    );
    let notes = d.notes();
    assert!(
        notes
            .last()
            .is_some_and(|n| n.starts_with("⚠ Turn failed:") && n.contains("gave up waiting")),
        "the outage's outcome is the last stored note: {notes:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn t3d_zero_restores_the_old_behaviour() {
    let provider = Arc::new(OutageProvider::new(
        0,
        mins(4 * 60),
        Fault::Busy {
            retry_after_secs: None,
        },
    ));
    let mut d = drill(provider, pinned("drill::flash"), 0);
    let started = tokio::time::Instant::now();
    let err = d.session.run_turn("fix it").await.unwrap_err();
    assert!(!matches!(err, CoreError::ProviderDown { .. }), "{err}");
    assert!(started.elapsed() <= mins(11), "{:?}", started.elapsed());
    assert_eq!(d.outage_events(), 0);
}

#[tokio::test(start_paused = true)]
async fn t3e_the_run_deadline_bounds_the_wait() {
    let provider = Arc::new(OutageProvider::new(0, mins(4 * 60), Fault::Refused));
    let mut d = drill(provider, pinned("drill::flash"), ALLOWANCE);
    d.session.set_run_budget(Some(crate::run_budget::RunBudget {
        deadline: Some(std::time::Instant::now() + mins(30)),
        ..Default::default()
    }));
    let started = tokio::time::Instant::now();
    let err = d.session.run_turn("fix it").await.unwrap_err();
    assert!(matches!(err, CoreError::ProviderDown { .. }), "{err}");
    // The deadline is a real `Instant`, which the virtual clock does not advance, so it counts
    // from when the long wait begins (after the ten minute hot phase) rather than from the start.
    assert!(
        started.elapsed() >= mins(10) && started.elapsed() <= mins(10 + 36),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test(start_paused = true)]
async fn t3f_the_turn_deadline_bounds_the_wait() {
    let provider = Arc::new(OutageProvider::new(0, mins(4 * 60), Fault::Refused));
    let mut d = drill(provider, pinned("drill::flash"), ALLOWANCE);
    d.session
        .set_turn_deadline(std::time::Instant::now() + mins(20));
    let started = tokio::time::Instant::now();
    let err = d.session.run_turn("fix it").await.unwrap_err();
    assert!(matches!(err, CoreError::ProviderDown { .. }), "{err}");
    assert!(
        started.elapsed() >= mins(10) && started.elapsed() <= mins(10 + 26),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test(start_paused = true)]
async fn t4_the_tool_result_is_sent_exactly_once_across_a_mixed_outage() {
    let list = forge_types::ToolCall {
        id: "call-list".into(),
        name: "list_dir".into(),
        args: serde_json::json!({ "path": test_workspace().to_str().unwrap() }),
    };
    let provider = Arc::new(
        OutageProvider::new(1, mins(30), Fault::Mixed)
            .with_script(vec![("", vec![list]), ("DRILL-OK", vec![])]),
    );
    let mut d = drill(provider.clone(), pinned("drill::flash"), ALLOWANCE);
    let out = d.session.run_turn("look around").await.unwrap();
    assert_eq!(out.text, "DRILL-OK");
    let seen = provider.seen.lock().unwrap().clone();
    // Side calls after the answer (titles, memory) also reach the provider; what matters is that
    // exactly one request carried the tool result and that it followed the first, tool-calling one.
    let with_result: Vec<usize> = seen.iter().map(|(n, _)| *n).filter(|n| *n > 0).collect();
    assert_eq!(seen[0].0, 0, "{seen:?}");
    assert_eq!(with_result, vec![1], "{seen:?}");
    let tool_rows = d
        .store
        .load_messages(&d.session.id)
        .unwrap()
        .into_iter()
        .filter(|m| m.role == Role::Tool)
        .count();
    assert_eq!(tool_rows, 1, "the tool ran once and was stored once");
    assert!(provider.failed() > 5);
}

#[tokio::test(start_paused = true)]
async fn t5_an_unpinned_chain_that_is_wholly_down_waits_and_resumes_on_the_head() {
    let provider = Arc::new(OutageProvider::new(0, mins(90), Fault::Mixed));
    let router = Arc::new(FixedRouter {
        model: "alpha::big".into(),
        fallbacks: vec!["beta::mid".into()],
    });
    let mut d = drill(provider.clone(), router, ALLOWANCE);
    let out = d.session.run_turn("fix it").await.unwrap();
    assert_eq!(out.text, "DRILL-OK");
    assert_eq!(provider.seen.lock().unwrap()[0].1, "alpha::big");
    assert!(d.outage_events() > 5);
    let notes = d.notes();
    assert_eq!(
        notes.iter().filter(|n| n.starts_with('⏸')).count(),
        1,
        "{notes:?}"
    );
    assert_eq!(
        notes.iter().filter(|n| n.starts_with('▶')).count(),
        1,
        "{notes:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn t5b_a_dead_credential_in_the_chain_is_not_waited_for() {
    let mut provider = OutageProvider::new(0, mins(90), Fault::Refused);
    provider.auth_for = Some("beta::mid".into());
    let router = Arc::new(FixedRouter {
        model: "alpha::big".into(),
        fallbacks: vec!["beta::mid".into()],
    });
    let mut d = drill(Arc::new(provider), router, ALLOWANCE);
    let started = tokio::time::Instant::now();
    let err = d.session.run_turn("fix it").await.unwrap_err();
    assert!(matches!(err, CoreError::NoHealthyModel { .. }), "{err}");
    assert!(started.elapsed() < mins(5), "{:?}", started.elapsed());
    assert_eq!(d.outage_events(), 0);
}

#[tokio::test(start_paused = true)]
async fn t6_esc_during_the_wait_cancels_it_and_nothing_probes_afterwards() {
    let provider = Arc::new(OutageProvider::new(0, mins(4 * 60), Fault::Refused));
    let mut d = drill(provider.clone(), pinned("drill::flash"), ALLOWANCE);
    // Dropping the turn future is what the TUI's Esc and the daemon's interrupt do
    // (`turn_handle.abort()`): the sleep is cancelled at its await point.
    let interrupted = tokio::time::timeout(mins(45), d.session.run_turn("fix it")).await;
    assert!(interrupted.is_err(), "still waiting when interrupted");
    let calls = provider.failed();
    assert!(d.outage_events() > 0 && calls > 0);
    tokio::time::sleep(mins(3 * 60)).await;
    assert_eq!(
        provider.failed(),
        calls,
        "no probe after the turn was dropped"
    );
    assert!(d.notes().iter().all(|n| !n.starts_with('▶')));
}

/// Routes to its model, except that a benched model is reported unroutable, as the mesh does when
/// its only candidate is benched and nothing else is usable.
struct BenchAwareRouter(&'static str);

#[async_trait::async_trait]
impl Router for BenchAwareRouter {
    async fn route(
        &self,
        _prompt: &str,
        _has_images: bool,
        _budget: BudgetState,
        health: &forge_types::ModelHealth,
        _quota: &forge_types::SubscriptionQuota,
        _effort: Option<forge_types::EffortLevel>,
        _project: &forge_types::ProjectContext,
    ) -> forge_mesh::RoutingDecision {
        forge_mesh::RoutingDecision {
            effort: None,
            tier: forge_types::TaskTier::Trivial,
            model: self.0.into(),
            rationale: "test".into(),
            fallbacks: vec![],
            pinned: false,
            unroutable: health.is_benched(self.0),
        }
    }
}

// The bench clock is the wall clock (epoch seconds in the store), so these run in real time with
// benches of a couple of seconds rather than on the virtual clock.

#[tokio::test]
async fn t7_a_transient_bench_at_turn_start_is_waited_out_not_refused() {
    let provider = Arc::new(OutageProvider::new(0, Duration::ZERO, Fault::Refused));
    let mut d = drill(
        provider,
        Arc::new(BenchAwareRouter("drill::flash")),
        ALLOWANCE,
    );
    d.store
        .bench_for(
            "drill::flash",
            Duration::from_secs(2),
            "provider overloaded",
        )
        .unwrap();
    let out = d.session.run_turn("continue").await.unwrap();
    assert_eq!(out.text, "DRILL-OK");
    assert_eq!(out.stop_reason, StopReason::FinalAnswer);
    assert!(d.outage_events() >= 1);
    let notes = d.notes();
    assert_eq!(
        notes.iter().filter(|n| n.starts_with('⏸')).count(),
        1,
        "{notes:?}"
    );
    assert!(notes.iter().any(|n| n.starts_with('▶')), "{notes:?}");
    assert_eq!(d.prompts(), vec!["continue".to_string()]);
}

/// A daemon restart or Esc drops the turn task in the middle of the turn-start wait. The prompt
/// must already be in the transcript then, or it is lost and the tail reads as finished, so the
/// restart re-drive skips the session.
#[tokio::test]
async fn t7d_a_prompt_waiting_out_a_bench_survives_the_turn_being_dropped() {
    let provider = Arc::new(OutageProvider::new(0, Duration::ZERO, Fault::Refused));
    let mut d = drill(
        provider,
        Arc::new(BenchAwareRouter("drill::flash")),
        ALLOWANCE,
    );
    d.store
        .bench_for(
            "drill::flash",
            Duration::from_secs(3600),
            "provider overloaded",
        )
        .unwrap();
    let cut = tokio::time::timeout(
        Duration::from_millis(500),
        d.session.run_turn("ship the fix"),
    )
    .await;
    assert!(cut.is_err(), "the turn should still be waiting");
    assert_eq!(d.prompts(), vec!["ship the fix".to_string()]);
    assert_eq!(
        d.store.session_tail_state(&d.session.id).unwrap(),
        forge_store::TailState::Interrupted(forge_store::InterruptedTail::UnansweredPrompt)
    );
}

#[tokio::test]
async fn t7b_a_permanent_exclusion_at_turn_start_is_refused_at_once() {
    let provider = Arc::new(OutageProvider::new(0, Duration::ZERO, Fault::Refused));
    let mut d = drill(
        provider,
        Arc::new(BenchAwareRouter("drill::flash")),
        ALLOWANCE,
    );
    d.store
        .exclude_model("drill::flash", "no tool support")
        .unwrap();
    let started = std::time::Instant::now();
    let out = d.session.run_turn("continue").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::NoOutput);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(d.outage_events(), 0);
}

#[tokio::test]
async fn t7c_the_horizon_bounds_the_turn_start_wait() {
    let provider = Arc::new(OutageProvider::new(0, Duration::ZERO, Fault::Refused));
    let mut d = drill(provider, Arc::new(BenchAwareRouter("drill::flash")), 2);
    d.store
        .bench_for(
            "drill::flash",
            Duration::from_secs(3600),
            "provider overloaded",
        )
        .unwrap();
    let started = std::time::Instant::now();
    let out = d.session.run_turn("continue").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::NoOutput);
    assert!(
        started.elapsed() >= Duration::from_secs(2) && started.elapsed() < Duration::from_secs(6),
        "{:?}",
        started.elapsed()
    );
    assert!(d.outage_events() >= 1);
    assert_eq!(d.prompts(), vec!["continue".to_string()]);
}

#[tokio::test(start_paused = true)]
async fn t4_the_recovery_line_comes_before_the_answer_it_unblocked() {
    let provider = Arc::new(OutageProvider::new(
        0,
        mins(20),
        Fault::Busy {
            retry_after_secs: None,
        },
    ));
    let mut d = drill(provider, pinned("drill::flash"), ALLOWANCE);
    let out = d.session.run_turn("fix it").await.unwrap();
    assert_eq!(out.text, "DRILL-OK");
    let events = d.events.lock().unwrap();
    let recovered = events
        .iter()
        .position(|e| matches!(e, PresenterEvent::ProviderRecovered { .. }))
        .expect("a recovery event");
    let answer = events
        .iter()
        .position(|e| {
            matches!(
                e,
                PresenterEvent::AssistantDelta(_) | PresenterEvent::AssistantText(_)
            )
        })
        .expect("the answer");
    assert!(recovered < answer, "recovery after the answer: {events:?}");
    let back_warnings = events
        .iter()
        .filter(|e| matches!(e, PresenterEvent::Warning(w) if w.starts_with('▶')))
        .count();
    assert_eq!(back_warnings, 1);
    drop(events);
    let back: Vec<_> = d
        .notes()
        .into_iter()
        .filter(|n| n.starts_with('▶'))
        .collect();
    assert_eq!(back.len(), 1, "one stored recovery note");
}
