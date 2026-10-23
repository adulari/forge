//! The shell error interceptor runs off the turn: a failed command's result never waits for the
//! diagnosis, which reaches the model only when it is ready by the next request.

use super::*;
use std::time::Duration;

const DIAGNOSE_PREFIX: &str = "A shell command run by";
const CAUSE: &str = "the opaque stage exited early";

/// Step 0 runs a failing command and a short sleep; step 1 records whether the request carried
/// the diagnosis and sleeps long enough for a slow diagnosis to land; step 2 records again and
/// answers. The diagnosis call answers after `delay`.
struct TimedDiagnosis {
    delay: Duration,
    carried: Arc<Mutex<Vec<bool>>>,
}

fn shell(command: &str) -> forge_types::ToolCall {
    forge_types::ToolCall {
        id: forge_types::new_id(),
        name: "shell".into(),
        args: serde_json::json!({ "command": command }),
    }
}

#[async_trait::async_trait]
impl Provider for TimedDiagnosis {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        if messages
            .iter()
            .any(|m| m.role == Role::System && m.content.starts_with(DIAGNOSE_PREFIX))
        {
            tokio::time::sleep(self.delay).await;
            return Ok(forge_provider::ModelResponse {
                content: format!("{CAUSE}\nFIX: rerun it"),
                ..Default::default()
            });
        }
        let results = messages.iter().filter(|m| m.role == Role::Tool).count();
        if results > 0 {
            let carried = messages.iter().any(|m| m.content.contains(CAUSE));
            self.carried.lock().unwrap().push(carried);
        }
        let tool_calls = match results {
            0 => vec![
                shell("printf opaque_failure_xyz >&2; exit 9"),
                shell("sleep 0.4"),
            ],
            2 => vec![shell("sleep 1.5")],
            _ => vec![],
        };
        Ok(forge_provider::ModelResponse {
            content: if tool_calls.is_empty() {
                "done".into()
            } else {
                String::new()
            },
            tool_calls,
            ..Default::default()
        })
    }
}

fn session_with(
    provider: Arc<dyn Provider>,
    store: Arc<Store>,
) -> (Session, Arc<Mutex<Vec<PresenterEvent>>>) {
    let config = Config {
        permission_mode: forge_types::PermissionMode::Bypass,
        ..Config::default()
    };
    let presenter = CapturePresenter::default();
    let events = presenter.events.clone();
    let session = Session::start(
        store,
        provider,
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(presenter),
        config,
        test_workspace().to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();
    (session, events)
}

fn diagnoses(events: &Mutex<Vec<PresenterEvent>>) -> usize {
    events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| {
            matches!(e, PresenterEvent::ShellDiagnosis { command, diagnosis, .. }
                if command.contains("opaque_failure_xyz") && diagnosis == CAUSE)
        })
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_diagnosis_ready_by_the_next_request_reaches_the_model() {
    let carried = Arc::new(Mutex::new(Vec::new()));
    let (mut session, events) = session_with(
        Arc::new(TimedDiagnosis {
            delay: Duration::ZERO,
            carried: carried.clone(),
        }),
        Arc::new(Store::open_in_memory().unwrap()),
    );

    assert_eq!(session.run_turn("build the project").await.unwrap(), "done");

    assert_eq!(
        carried.lock().unwrap().first(),
        Some(&true),
        "the request right after the failure carries it"
    );
    assert_eq!(diagnoses(&events), 1, "shown once, at the step it reached");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_late_diagnosis_is_shown_but_never_handed_to_the_model() {
    let carried = Arc::new(Mutex::new(Vec::new()));
    let (mut session, events) = session_with(
        Arc::new(TimedDiagnosis {
            delay: Duration::from_millis(800),
            carried: carried.clone(),
        }),
        Arc::new(Store::open_in_memory().unwrap()),
    );

    assert_eq!(session.run_turn("build the project").await.unwrap(), "done");

    assert_eq!(
        *carried.lock().unwrap(),
        [false, false],
        "the model already acted on the failure; a late hint would describe the past"
    );
    assert_eq!(diagnoses(&events), 1, "the person still sees it");
}

/// The diagnosis request never answers, which once held a TUI turn for more than six minutes.
struct StalledDiagnosis;

#[async_trait::async_trait]
impl Provider for StalledDiagnosis {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        if messages
            .iter()
            .any(|m| m.role == Role::System && m.content.starts_with(DIAGNOSE_PREFIX))
        {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
        let failed = messages.iter().any(|m| m.role == Role::Tool);
        Ok(forge_provider::ModelResponse {
            content: if failed { "done".into() } else { String::new() },
            tool_calls: if failed {
                vec![]
            } else {
                vec![shell("printf opaque_auxiliary_stall >&2; exit 23")]
            },
            ..Default::default()
        })
    }
}

#[tokio::test]
async fn a_stalled_diagnosis_never_holds_the_turn() {
    let (mut session, _events) = session_with(
        Arc::new(StalledDiagnosis),
        Arc::new(Store::open_in_memory().unwrap()),
    );
    let started = std::time::Instant::now();
    let answer = tokio::time::timeout(
        Duration::from_secs(10),
        session.run_turn("run the failing test and repair it"),
    )
    .await
    .expect("the optional diagnosis held the turn hostage")
    .unwrap();
    assert_eq!(answer, "done");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the turn waited on the diagnosis: {:?}",
        started.elapsed()
    );
}

/// Counts diagnosis requests.
struct CountingDiagnosis(Arc<Mutex<Vec<String>>>);

#[async_trait::async_trait]
impl Provider for CountingDiagnosis {
    async fn complete(
        &self,
        model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        if messages
            .iter()
            .any(|m| m.role == Role::System && m.content.starts_with(DIAGNOSE_PREFIX))
        {
            self.0.lock().unwrap().push(model.to_string());
        }
        let failed = messages.iter().any(|m| m.role == Role::Tool);
        Ok(forge_provider::ModelResponse {
            content: if failed { "done".into() } else { String::new() },
            tool_calls: if failed {
                vec![]
            } else {
                vec![shell("printf opaque_failure_xyz >&2; exit 9")]
            },
            ..Default::default()
        })
    }
}

/// A headless run pinned to one model would only ask that model about output it reads itself.
#[tokio::test]
async fn an_unattended_pinned_run_makes_no_diagnosis_call() {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let (mut session, events) = session_with(
        Arc::new(CountingDiagnosis(asked.clone())),
        Arc::new(Store::open_in_memory().unwrap()),
    );
    session.pin_model(Some("ollama::qwen3:4b".into()));

    assert_eq!(session.run_turn("build the project").await.unwrap(), "done");
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(
        asked.lock().unwrap().is_empty(),
        "{:?}",
        asked.lock().unwrap()
    );
    assert_eq!(diagnoses(&events), 0);
}
