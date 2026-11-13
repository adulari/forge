//! A local, headless `/goal` run: two turns when the evaluator says "not yet" once, and a
//! not-met outcome at the budget, iteration and gate caps.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use forge_core::Session;
use forge_provider::{EventSink, MockProvider, ModelResponse, Provider, ProviderError, ToolSpec};
use forge_types::{Message, Role};

use super::*;

/// The mock for turns; the evaluator is answered by `verdict(n)` for its n-th call (0-based).
/// Every turn is followed by exactly one evaluation, so `evaluations` is also the turn count.
struct ScriptedEvaluator {
    verdict: Box<dyn Fn(usize) -> String + Send + Sync>,
    evaluations: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for ScriptedEvaluator {
    async fn complete(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError> {
        let is_eval = messages.first().is_some_and(|m| {
            m.role == Role::System
                && m.content
                    .starts_with(forge_core::goal_evaluator::GOAL_EVALUATOR_MARKER)
        });
        if is_eval {
            let n = self.evaluations.fetch_add(1, Ordering::SeqCst);
            let mut reply = MockProvider
                .complete(model, messages, tools, on_event)
                .await?;
            reply.content = (self.verdict)(n);
            return Ok(reply);
        }
        MockProvider
            .complete(model, messages, tools, on_event)
            .await
    }
}

fn verdict_json(achieved: bool, remaining: &[String]) -> String {
    serde_json::json!({
        "achieved": achieved,
        "remaining": remaining,
        "next_step": remaining.first().cloned().unwrap_or_default(),
        "confidence": 0.9,
        "progressed": true,
        "reason": "scripted",
    })
    .to_string()
}

struct Harness {
    session: Session,
    dir: tempfile::TempDir,
    evaluations: Arc<AtomicUsize>,
}

fn harness(verdict: impl Fn(usize) -> String + Send + Sync + 'static) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let evaluations = Arc::new(AtomicUsize::new(0));
    let config = forge_config::Config::default();
    let session = Session::start(
        Arc::new(forge_store::Store::open_in_memory().unwrap()),
        Arc::new(ScriptedEvaluator {
            verdict: Box::new(verdict),
            evaluations: evaluations.clone(),
        }),
        Arc::new(forge_mesh::HeuristicRouter::new(config.clone())),
        forge_tools::ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(forge_tui::HeadlessPresenter::new(false)),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();
    Harness {
        session,
        dir,
        evaluations,
    }
}

fn spec(gates: &[&str], max_tokens: Option<u64>) -> GoalRunSpec {
    GoalRunSpec {
        goal: "ship the thing".into(),
        gates: gates.iter().map(|g| g.to_string()).collect(),
        max_tokens,
        max_minutes: None,
        budget: None,
        contract: false,
    }
}

async fn run(h: &mut Harness, spec: GoalRunSpec) -> GoalRunEnd {
    let cwd = h.dir.path().to_path_buf();
    run_goal_headless(&mut h.session, spec, &[], &cwd, std::future::pending())
        .await
        .unwrap()
}

#[test]
fn only_a_goal_command_is_a_goal_prompt() {
    let spec = parse_goal_prompt("/goal ship it --gate \"cargo test\" --max-tokens 9").unwrap();
    assert_eq!(spec.goal, "ship it");
    assert_eq!(spec.gates, vec!["cargo test"]);
    assert_eq!(spec.max_tokens, Some(9));
    assert!(parse_goal_prompt("/goals ship it").is_none());
    assert!(parse_goal_prompt("fix the goal parser").is_none());
    assert!(parse_goal_prompt("/rust do it").is_none());
}

#[tokio::test]
async fn not_done_once_then_done_runs_two_turns_and_is_met() {
    let mut h = harness(|n| {
        if n == 0 {
            verdict_json(false, &["wire the flag".to_string()])
        } else {
            verdict_json(true, &[])
        }
    });
    let end = run(&mut h, spec(&[], None)).await;
    assert_eq!(end, GoalRunEnd::Met);
    assert_eq!(h.evaluations.load(Ordering::SeqCst), 2);
    assert!(end.into_result().is_ok());
}

#[tokio::test]
async fn a_token_budget_that_is_spent_ends_not_met() {
    let mut h = harness(|_| verdict_json(false, &["more".to_string()]));
    let end = run(&mut h, spec(&[], Some(1))).await;
    let GoalRunEnd::NotMet(reason) = end else {
        panic!("a spent budget is not a met goal");
    };
    assert!(reason.contains("token budget"), "{reason}");
    assert_eq!(
        h.evaluations.load(Ordering::SeqCst),
        0,
        "the turn was stopped inside the loop, so no evaluator call is spent on it"
    );
    assert!(GoalRunEnd::NotMet(reason).into_result().is_err());
}

#[tokio::test]
async fn a_time_budget_that_is_spent_stops_the_turn_before_any_model_call() {
    let mut h = harness(|_| verdict_json(false, &["more".to_string()]));
    let end = run(
        &mut h,
        GoalRunSpec {
            max_minutes: Some(0),
            ..spec(&[], None)
        },
    )
    .await;
    let GoalRunEnd::NotMet(reason) = end else {
        panic!("a spent budget is not a met goal");
    };
    assert!(reason.contains("time budget exhausted"), "{reason}");
    assert_eq!(h.evaluations.load(Ordering::SeqCst), 0);
    assert!(
        h.session.run_budget_hit_last_turn(),
        "the session, not just the driver, ended the turn"
    );
}

#[test]
fn remaining_budget_subtracts_what_earlier_turns_used() {
    let mut budget = AutonomyBudget::new(Some(1_000), Some(2));
    budget.tokens_used = 400;
    let left = budget.remaining().unwrap();
    assert_eq!(left.tokens_remaining, Some(600));
    assert!(left.deadline.is_some());
    assert!(AutonomyBudget::new(None, None).remaining().is_none());
}

#[tokio::test]
async fn the_iteration_ceiling_ends_not_met() {
    let mut h = harness(|n| verdict_json(false, &[format!("item {n}")]));
    let end = run(&mut h, spec(&[], None)).await;
    assert_eq!(
        end,
        GoalRunEnd::NotMet("🎯 goal stopped — iteration ceiling".into())
    );
    assert_eq!(h.evaluations.load(Ordering::SeqCst), GOAL_MAX_ITERS);
}

#[tokio::test]
async fn a_stuck_evaluator_wedges_the_run_instead_of_looping() {
    let mut h = harness(|_| verdict_json(false, &["same thing".to_string()]));
    let end = run(&mut h, spec(&[], None)).await;
    let GoalRunEnd::NotMet(reason) = end else {
        panic!("wedged is not met");
    };
    assert!(reason.contains("goal wedged"), "{reason}");
    assert_eq!(
        h.evaluations.load(Ordering::SeqCst),
        GOAL_EVAL_WEDGED_TURNS + 1
    );
}

#[tokio::test]
async fn gates_run_in_the_sessions_workspace() {
    let mut h = harness(|_| verdict_json(true, &[]));
    std::fs::write(h.dir.path().join("probe.txt"), "x").unwrap();
    // `test -f probe.txt` only passes in the session's workspace, not the process cwd.
    let end = run(&mut h, spec(&["test -f probe.txt"], None)).await;
    assert_eq!(end, GoalRunEnd::Met);

    let mut failing = harness(|_| verdict_json(true, &[]));
    let end = run(&mut failing, spec(&["test -f missing.txt"], None)).await;
    let GoalRunEnd::NotMet(reason) = end else {
        panic!("a failing gate must not end met");
    };
    assert!(reason.contains("quality gate exhausted"), "{reason}");
}

#[tokio::test]
async fn an_interrupt_stops_cleanly_and_is_not_met() {
    let mut h = harness(|_| verdict_json(true, &[]));
    let cwd = h.dir.path().to_path_buf();
    let end = run_goal_headless(&mut h.session, spec(&[], None), &[], &cwd, async {})
        .await
        .unwrap();
    assert!(matches!(end, GoalRunEnd::NotMet(_)));
}

#[tokio::test]
async fn a_headless_goal_refuses_an_envelope_instead_of_ignoring_it() {
    let mut h = harness(|_| verdict_json(false, &[]));
    let end = run(
        &mut h,
        GoalRunSpec {
            budget: Some("claude:2%w".into()),
            ..spec(&[], None)
        },
    )
    .await;
    assert!(matches!(end, GoalRunEnd::NotMet(r) if r.contains("--budget")));
}

/// A model whose worker turn n creates the n-th file (when `creates` allows it), whose drafter
/// answers with a fixed contract reply, and whose evaluator, if it is ever asked, says "achieved".
struct ContractWorld {
    root: std::path::PathBuf,
    draft: String,
    creates: bool,
    /// The scripted worker turn (0-based) that fixes `server.py` to answer `ok v2`.
    fix_server_at: Option<usize>,
    turns: Arc<AtomicUsize>,
    prompts: Arc<std::sync::Mutex<Vec<String>>>,
    evaluations: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for ContractWorld {
    async fn complete(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError> {
        let first = messages.first().map(|m| m.content.as_str()).unwrap_or("");
        let mut reply = MockProvider
            .complete(model, messages, tools, on_event)
            .await?;
        if first.starts_with(forge_core::goal_contract::GOAL_CONTRACT_MARKER) {
            reply.content = self.draft.clone();
        } else if first.starts_with(forge_core::goal_evaluator::GOAL_EVALUATOR_MARKER) {
            self.evaluations.fetch_add(1, Ordering::SeqCst);
            reply.content = verdict_json(true, &[]);
        } else if !tools.is_empty() && messages.last().is_some_and(|m| m.role != Role::Tool) {
            // A worker turn starts from a user prompt; its follow-up after a tool result and the
            // tool-less side calls (recap, title) are not new turns.
            let n = self.turns.fetch_add(1, Ordering::SeqCst);
            if let Some(prompt) = messages.iter().rev().find(|m| m.role == Role::User) {
                self.prompts.lock().unwrap().push(prompt.content.clone());
            }
            if self.fix_server_at == Some(n) {
                let path = self.root.join("server.py");
                let fixed = std::fs::read_to_string(&path)
                    .unwrap()
                    .replace("ok v1", "ok v2");
                std::fs::write(path, fixed).unwrap();
            }
            if self.creates {
                let name = ["a.txt", "b.txt", "c.txt"][n.min(2)];
                std::fs::write(self.root.join(name), "x").unwrap();
            }
        }
        Ok(reply)
    }
}

const CONTRACT: &str = r#"{"assertions":[
  {"assertion":"a exists","check":"test -f a.txt"},
  {"assertion":"b exists","check":"test -f b.txt"},
  {"assertion":"c exists","check":"test -f c.txt"}]}"#;

struct World {
    session: Session,
    dir: tempfile::TempDir,
    turns: Arc<AtomicUsize>,
    prompts: Arc<std::sync::Mutex<Vec<String>>>,
    evaluations: Arc<AtomicUsize>,
}

fn world(draft: &str, creates: bool) -> World {
    world_fixing(draft, creates, None)
}

fn world_fixing(draft: &str, creates: bool, fix_server_at: Option<usize>) -> World {
    let dir = tempfile::tempdir().unwrap();
    let turns = Arc::new(AtomicUsize::new(0));
    let prompts: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let evaluations = Arc::new(AtomicUsize::new(0));
    let config = forge_config::Config::default();
    let session = Session::start(
        Arc::new(forge_store::Store::open_in_memory().unwrap()),
        Arc::new(ContractWorld {
            root: dir.path().to_path_buf(),
            draft: draft.to_string(),
            creates,
            fix_server_at,
            turns: turns.clone(),
            prompts: prompts.clone(),
            evaluations: evaluations.clone(),
        }),
        Arc::new(forge_mesh::HeuristicRouter::new(config.clone())),
        forge_tools::ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(forge_tui::HeadlessPresenter::new(false)),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();
    World {
        session,
        dir,
        turns,
        prompts,
        evaluations,
    }
}

async fn run_world(w: &mut World, spec: GoalRunSpec) -> GoalRunEnd {
    let cwd = w.dir.path().to_path_buf();
    run_goal_headless(&mut w.session, spec, &[], &cwd, std::future::pending())
        .await
        .unwrap()
}

fn contract_spec() -> GoalRunSpec {
    GoalRunSpec {
        contract: true,
        ..spec(&[], None)
    }
}

#[test]
fn the_contract_flag_reaches_the_run_spec() {
    assert!(
        parse_goal_prompt("/goal --contract ship it")
            .unwrap()
            .contract
    );
    assert!(!parse_goal_prompt("/goal ship it").unwrap().contract);
}

#[tokio::test]
async fn failing_assertions_become_the_next_prompt_and_all_passing_ends_the_goal() {
    let mut w = world(CONTRACT, true);
    let end = run_world(&mut w, contract_spec()).await;
    assert_eq!(end, GoalRunEnd::Met);
    assert_eq!(
        w.turns.load(Ordering::SeqCst),
        3,
        "a.txt, then b.txt, then c.txt"
    );
    let prompts = w.prompts.lock().unwrap().clone();
    assert!(
        prompts[1].contains("assertion 2 failed: test -f b.txt"),
        "{}",
        prompts[1]
    );
    assert!(
        prompts[1].contains("assertion 3 failed: test -f c.txt"),
        "{}",
        prompts[1]
    );
    assert!(!prompts[1].contains("assertion 1 failed"), "{}", prompts[1]);
    assert!(
        prompts[2].contains("assertion 3 failed") && !prompts[2].contains("assertion 2 failed")
    );
    assert_eq!(
        w.evaluations.load(Ordering::SeqCst),
        0,
        "the validator, not the evaluator, judges a contract goal"
    );
}

#[tokio::test]
async fn one_failing_assertion_blocks_achievement_whatever_the_model_says() {
    let mut w = world(CONTRACT, false);
    std::fs::write(w.dir.path().join("a.txt"), "x").unwrap();
    std::fs::write(w.dir.path().join("b.txt"), "x").unwrap();
    let end = run_world(&mut w, contract_spec()).await;
    let GoalRunEnd::NotMet(reason) = end else {
        panic!("assertion 3 never passes, so the goal is not met");
    };
    assert!(
        reason.contains("wedged") && reason.contains('3'),
        "{reason}"
    );
    assert_eq!(w.evaluations.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_goal_whose_contract_cannot_be_drafted_does_not_start() {
    let mut w = world("no contract today", true);
    let end = run_world(&mut w, contract_spec()).await;
    let GoalRunEnd::NotMet(reason) = end else {
        panic!("no contract, no goal");
    };
    assert!(reason.contains("goal not started"), "{reason}");
    assert_eq!(w.turns.load(Ordering::SeqCst), 0, "the worker never ran");
}

#[tokio::test]
async fn a_headless_mission_starts_the_next_milestone_in_the_same_session() {
    let mut w = world(CONTRACT, false);
    let end = run_world(
        &mut w,
        GoalRunSpec {
            goal: "--mission ship the thing".into(),
            ..spec(&[], None)
        },
    )
    .await;
    assert_eq!(end, GoalRunEnd::Met);
    assert_eq!(
        w.turns.load(Ordering::SeqCst),
        2,
        "one turn per milestone: there is no driver to host a fresh worker"
    );
    let prompts = w.prompts.lock().unwrap().clone();
    assert!(
        prompts[1].contains("Milestone 1 passed. Now milestone 2/2"),
        "{}",
        prompts[1]
    );
    assert_eq!(w.evaluations.load(Ordering::SeqCst), 0);
}

#[test]
fn the_mission_flag_rides_in_the_goal_text_until_the_run_starts() {
    let spec = parse_goal_prompt("/goal --mission ship it").unwrap();
    assert!(!spec.contract);
    assert_eq!(spec.goal, "--mission ship it");
}

const QA_SERVER: &str = r#"import http.server, sys
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.end_headers(); self.wfile.write(b"ok v1")
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
"#;

const QA_CONTRACT: &str = r#"{"assertions":[
  {"assertion":"server.py exists","check":"test -f server.py"},
  {"assertion":"server.py serves /health","check":"grep -q do_GET server.py"},
  {"assertion":"the workspace is a directory","check":"test -d ."},
  {"assertion":"/health answers ok v2","behaviour":{
    "run":"echo $$ > pid.txt; exec python3 server.py $FORGE_QA_PORT",
    "probe":"GET http://127.0.0.1:$FORGE_QA_PORT/health","expect":"ok v2"}}]}"#;

fn pid_gone(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

/// The F11 drill: QA fails on `ok v1` with proof, the scripted worker fixes the server, QA passes
/// on `ok v2`, nothing is left running, and the proof shows in `forge goal show` and the snapshot.
#[tokio::test]
async fn qa_fails_on_the_old_answer_then_passes_after_the_fix_and_leaks_nothing() {
    let mut w = world_fixing(QA_CONTRACT, false, Some(1));
    std::fs::write(w.dir.path().join("server.py"), QA_SERVER).unwrap();
    let end = run_world(&mut w, contract_spec()).await;
    assert_eq!(end, GoalRunEnd::Met);
    assert_eq!(w.turns.load(Ordering::SeqCst), 2);
    let prompts = w.prompts.lock().unwrap().clone();
    assert!(prompts[1].contains("assertion 4 failed"), "{}", prompts[1]);
    assert!(
        prompts[1].contains("ok v1"),
        "proof reaches the worker: {}",
        prompts[1]
    );
    let pid: i32 = std::fs::read_to_string(w.dir.path().join("pid.txt"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(pid_gone(pid), "the server outlived the QA run");

    let mut contract = forge_types::goal_contract::GoalContract::new(
        forge_types::goal_contract::parse_contract(QA_CONTRACT).unwrap(),
    );
    contract.approved = true;
    let results = w.session.validate_goal_contract(&contract).await;
    contract.record(results, 2);
    assert!(contract.all_passed(), "{:?}", contract.results);
    let record = serde_json::json!({
        "objective": "serve ok v2 on /health", "gates": [], "max_tokens": null,
        "max_minutes": null, "iteration": 2, "tokens_used": 0, "elapsed_secs": 3,
        "reason": null, "envelope": null, "contract": contract, "phase": "achieved"
    })
    .to_string();
    let shown = super::goal_show::render_record("qa-drill", &record).unwrap();
    assert!(shown.contains("proof: ok v2"), "{shown}");
    let snap = crate::remote::goal::SnapContract::from(&contract);
    let json = serde_json::to_string_pretty(&snap).unwrap();
    assert!(
        json.contains("ok v2") && json.contains("\"behaviour\": true"),
        "{json}"
    );
    if let Ok(out) = std::env::var("FORGE_QA_DRILL_OUT") {
        let out = std::path::Path::new(&out);
        let first_failure = prompts[1].clone();
        std::fs::write(
            out.join("worker-prompt-after-qa-failure.txt"),
            first_failure,
        )
        .unwrap();
        std::fs::write(out.join("goal-show.txt"), &shown).unwrap();
        std::fs::write(out.join("snapshot-contract.json"), &json).unwrap();
        std::fs::write(
            out.join("leaked-pid.txt"),
            format!("server pid {pid} gone: {}\n", pid_gone(pid)),
        )
        .unwrap();
    }
}
