//! `run_code` end to end on a real session: every inner call goes through `invoke_tool` and the
//! shared bookkeeping, and only the outer result reaches the transcript (docs/features/codemode.md).

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct Ledgers {
    tools_ran: Arc<AtomicU64>,
    inspect_ran: Arc<AtomicU64>,
    mutations_ran: Arc<AtomicU64>,
    verification: Arc<Mutex<VerificationLedger>>,
}

impl Ledgers {
    fn new() -> Self {
        Self {
            tools_ran: Arc::default(),
            inspect_ran: Arc::default(),
            mutations_ran: Arc::default(),
            verification: Arc::default(),
        }
    }
}

/// A presenter that approves every prompt after `delay`, counting prompts and asks.
#[derive(Clone, Default)]
struct Approver {
    delay_ms: u64,
    prompts: Arc<AtomicU64>,
    asks: Arc<AtomicU64>,
}

impl Presenter for Approver {
    fn emit(&mut self, _event: PresenterEvent) {}
    fn confirm(&mut self, _tool: &str, _side_effect: SideEffect) -> forge_types::ConfirmOutcome {
        self.prompts.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(self.delay_ms));
        forge_types::ConfirmOutcome::Allow
    }
    fn ask(&mut self, _q: &str, _o: &[forge_types::QChoice], _a: bool) -> String {
        self.asks.fetch_add(1, Ordering::SeqCst);
        String::new()
    }
    fn read_line(&mut self) -> Option<String> {
        None
    }
}

fn workspace(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("forge-codemode-{tag}-{}", forge_types::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn session_in(dir: &std::path::Path, mode: PermissionMode, presenter: Approver) -> Session {
    session_with_config(dir, mode, presenter, Config::default())
}

fn session_with_config(
    dir: &std::path::Path,
    mode: PermissionMode,
    presenter: Approver,
    config: Config,
) -> Session {
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(MockProvider),
        Arc::new(FixedRouter {
            model: "direct::codemode".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir),
        Box::new(presenter),
        config,
        dir.to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();
    session.set_mode(mode);
    session
}

fn run_code_call(code: &str, timeout_secs: Option<u64>) -> forge_types::ToolCall {
    let mut args = serde_json::json!({ "code": code });
    if let Some(t) = timeout_secs {
        args["timeout_secs"] = t.into();
    }
    forge_types::ToolCall {
        id: "rc1".into(),
        name: codemode::RUN_CODE_TOOL.into(),
        args,
    }
}

/// Runs one model step holding `call`, the same entry point the model loop uses. Returns the
/// assistant message id and the step's tool-result text.
async fn step(
    session: &mut Session,
    ledgers: &Ledgers,
    call: forge_types::ToolCall,
) -> (String, String) {
    let msg_id = session
        .store
        .add_message_full(session.id(), 0, Role::Assistant, "", None, &[], None)
        .unwrap();
    let (mut last_sig, mut repeat, mut recent) = (None, 0, std::collections::VecDeque::new());
    let (mut doom_nudged, mut failure_nudged) = (false, false);
    let mut failure_counts = std::collections::HashMap::new();
    let mut observations = Vec::new();
    let halted = session
        .execute_model_tool_step(
            &msg_id,
            &[call],
            &ledgers.tools_ran,
            &ledgers.inspect_ran,
            &ledgers.mutations_ran,
            &ledgers.verification,
            &mut last_sig,
            &mut repeat,
            &mut recent,
            &mut doom_nudged,
            &mut failure_counts,
            &mut failure_nudged,
            &mut observations,
        )
        .await
        .unwrap();
    assert!(!halted);
    let result = session
        .transcript
        .iter()
        .rev()
        .find(|m| m.role == Role::Tool)
        .map(|m| m.content.clone())
        .unwrap_or_default();
    (msg_id, result)
}

fn audit(session: &Session, msg_id: &str) -> Vec<(String, String)> {
    session
        .store
        .tool_call_rows(msg_id)
        .unwrap()
        .into_iter()
        .map(|r| (r.tool_name, r.status))
        .collect()
}

#[tokio::test]
async fn inner_writes_count_and_only_the_outer_result_enters_the_transcript() {
    let dir = workspace("write");
    let mut session = session_in(&dir, PermissionMode::AcceptEdits, Approver::default());
    let ledgers = Ledgers::new();
    let path = dir.join("note.txt");
    let code = format!(
        "await tools.write_file({{path: {p:?}, content: 'hello'}});\n\
         const back = await tools.read_file({{path: {p:?}}});\n\
         return back.includes('hello') ? 'round trip ok' : back;",
        p = path.to_str().unwrap()
    );
    let (msg_id, result) = step(&mut session, &ledgers, run_code_call(&code, None)).await;

    assert!(result.starts_with("round trip ok"), "{result}");
    assert!(
        result.contains("[run_code: 2 tool calls, 2 ok, 0 failed"),
        "{result}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
    assert_eq!(
        ledgers.mutations_ran.load(Ordering::SeqCst),
        1,
        "phantom-edit gate sees the write"
    );
    assert_eq!(
        ledgers.tools_ran.load(Ordering::SeqCst),
        3,
        "outer + two inner calls"
    );
    assert_eq!(
        session.edits_this_turn, 1,
        "the write took the snapshot path"
    );
    let tool_messages: Vec<&Message> = session
        .transcript
        .iter()
        .filter(|m| m.role == Role::Tool)
        .collect();
    assert_eq!(
        tool_messages.len(),
        1,
        "inner outputs never enter the transcript"
    );
    assert_eq!(tool_messages[0].tool_call_id.as_deref(), Some("rc1"));
    assert_eq!(
        audit(&session, &msg_id),
        vec![
            ("write_file".to_string(), "ok".to_string()),
            ("read_file".to_string(), "ok".to_string()),
            ("run_code".to_string(), "ok".to_string()),
        ],
        "one audit row per inner call plus the outer one"
    );
    // Only the inner read counts as evidence; the run_code wrapper itself adds none.
    assert_eq!(ledgers.verification.lock().unwrap().checkpoint(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_denied_inner_write_throws_and_changes_nothing() {
    let dir = workspace("denied");
    let mut session = session_in(&dir, PermissionMode::Plan, Approver::default());
    let ledgers = Ledgers::new();
    let path = dir.join("nope.txt");
    let code = format!(
        "try {{ await tools.write_file({{path: {p:?}, content: 'x'}}); return 'wrote'; }}\n\
         catch (e) {{ return 'refused: ' + e.message; }}",
        p = path.to_str().unwrap()
    );
    let (_, result) = step(&mut session, &ledgers, run_code_call(&code, None)).await;
    assert!(result.starts_with("refused: "), "{result}");
    assert!(result.contains("1 failed"), "{result}");
    assert!(!path.exists());
    assert_eq!(ledgers.mutations_ran.load(Ordering::SeqCst), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn session_control_tools_and_recursion_are_refused_inside_a_script() {
    let dir = workspace("deny");
    let presenter = Approver::default();
    let asks = Arc::clone(&presenter.asks);
    let mut session = session_in(&dir, PermissionMode::AcceptEdits, presenter);
    let code = "const out = [];\n\
        for (const name of ['ask_user', 'run_code', 'present_plan']) {\n\
          try { await tools[name]({}); out.push(name + ' ran'); }\n\
          catch (e) { out.push(name + ' refused'); }\n\
        }\n\
        return out.join(', ');";
    let (msg_id, result) = step(&mut session, &Ledgers::new(), run_code_call(code, None)).await;
    assert!(
        result.starts_with("ask_user refused, run_code refused, present_plan refused"),
        "{result}"
    );
    assert_eq!(asks.load(Ordering::SeqCst), 0, "the user was never asked");
    assert_eq!(audit(&session, &msg_id).len(), 1, "only the outer row");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The defect: gpt-6-luna led a script with `tools.update_tasks(...)` and then read files in the
/// same script. `update_tasks` was on the script deny list, so the script died on its first line
/// ("0 tool calls") and the whole step, reads included, was wasted.
#[tokio::test]
async fn update_tasks_runs_inside_a_script_beside_the_calls_batched_with_it() {
    let dir = workspace("tasks");
    std::fs::write(dir.join("a.txt"), "alpha").unwrap();
    let mut session = session_in(&dir, PermissionMode::AcceptEdits, Approver::default());
    let ledgers = Ledgers::new();
    let code = "const t = await tools.update_tasks({tasks: [\n\
          {title: 'Read the spec', status: 'in_progress'},\n\
          {title: 'Implement it', status: 'pending'}]});\n\
        const a = await tools.read_file({path: 'a.txt'});\n\
        return t.split('\\n')[0] + ' | ' + (a.includes('alpha') ? 'read ok' : a);";
    let (msg_id, result) = step(&mut session, &ledgers, run_code_call(code, None)).await;

    assert!(
        result.starts_with("task list updated: 2 task(s) — 0 done, 1 in progress | read ok"),
        "{result}"
    );
    assert!(
        result.contains("[run_code: 2 tool calls, 2 ok, 0 failed"),
        "{result}"
    );
    let titles: Vec<(&str, forge_types::TodoStatus)> = session
        .tasks
        .iter()
        .map(|t| (t.title.as_str(), t.status))
        .collect();
    assert_eq!(
        titles,
        vec![
            ("Read the spec", forge_types::TodoStatus::InProgress),
            ("Implement it", forge_types::TodoStatus::Pending),
        ],
        "the script's update is the session's task list"
    );
    assert_eq!(
        audit(&session, &msg_id),
        vec![
            ("update_tasks".to_string(), "ok".to_string()),
            ("read_file".to_string(), "ok".to_string()),
            ("run_code".to_string(), "ok".to_string()),
        ],
        "the task update is audited like any other inner call"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn an_inner_failing_test_lands_in_the_verification_ledger() {
    let dir = workspace("verify");
    let mut session = session_in(&dir, PermissionMode::Bypass, Approver::default());
    let ledgers = Ledgers::new();
    let code =
        "const out = await tools.shell({command: \"sh -c 'exit 1' # run the test suite\"});\n\
        return out.split('\\n')[0];";
    let (_, result) = step(&mut session, &ledgers, run_code_call(code, None)).await;
    assert!(
        result.starts_with("shell: exit 1"),
        "a non-zero exit resolves: {result}"
    );
    assert!(
        ledgers
            .verification
            .lock()
            .unwrap()
            .unresolved_summary()
            .is_some(),
        "the failing test family is unresolved, exactly as for a plain call"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[tokio::test]
async fn hook_context_from_an_inner_call_is_visible_to_the_script_and_delivered_once() {
    let dir = workspace("hint");
    let config = Config {
        hooks: vec![forge_config::HookConfig {
            event: forge_config::HookEvent::PostToolUse,
            kind: Default::default(),
            if_rule: None,
            url: None,
            headers: Default::default(),
            allowed_env_vars: Vec::new(),
            run_async: false,
            matcher: Some("write_file".into()),
            command: r#"echo '{"action":"inject","context":"HOOK-CONTEXT-7"}'"#.into(),
            prompt: None,
            model: None,
            timeout_secs: 10,
            cc_compat: false,
        }],
        ..Config::default()
    };
    let mut session = session_with_config(
        &dir,
        PermissionMode::AcceptEdits,
        Approver::default(),
        config,
    );
    let code = format!(
        "const r = await tools.write_file({{path: {p:?}, content: 'x'}});\n\
         return r.includes('[hint] HOOK-CONTEXT-7') ? 'script saw the hint' : r;",
        p = dir.join("h.txt").to_str().unwrap()
    );
    let (_, result) = step(&mut session, &Ledgers::new(), run_code_call(&code, None)).await;
    assert!(result.starts_with("script saw the hint"), "{result}");
    let delivered = session
        .transcript
        .iter()
        .filter(|m| m.role == Role::System && m.content.contains("HOOK-CONTEXT-7"))
        .count();
    assert_eq!(delivered, 1, "also delivered once after the outer result");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[tokio::test]
async fn a_hint_the_script_returned_is_not_delivered_a_second_time() {
    let dir = workspace("hint-returned");
    let config = Config {
        hooks: vec![forge_config::HookConfig {
            event: forge_config::HookEvent::PostToolUse,
            kind: Default::default(),
            if_rule: None,
            url: None,
            headers: Default::default(),
            allowed_env_vars: Vec::new(),
            run_async: false,
            matcher: Some("write_file".into()),
            command: r#"echo '{"action":"inject","context":"HOOK-CONTEXT-8"}'"#.into(),
            prompt: None,
            model: None,
            timeout_secs: 10,
            cc_compat: false,
        }],
        ..Config::default()
    };
    let mut session = session_with_config(
        &dir,
        PermissionMode::AcceptEdits,
        Approver::default(),
        config,
    );
    let code = format!(
        "return await tools.write_file({{path: {p:?}, content: 'x'}});",
        p = dir.join("h.txt").to_str().unwrap()
    );
    let (_, result) = step(&mut session, &Ledgers::new(), run_code_call(&code, None)).await;
    assert!(result.contains("[hint] HOOK-CONTEXT-8"), "{result}");
    let delivered = session
        .transcript
        .iter()
        .filter(|m| m.role == Role::System && m.content.contains("HOOK-CONTEXT-8"))
        .count();
    assert_eq!(delivered, 0, "the returned value already carries the hint");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn search_then_call_a_tool_by_name_in_one_step() {
    let dir = workspace("search");
    std::fs::write(dir.join("a.txt"), "x").unwrap();
    let mut session = session_in(&dir, PermissionMode::AcceptEdits, Approver::default());
    let code = format!(
        "const hits = await tools.$search('list_dir');\n\
         const listing = await tools.tool_call({{name: hits[0].name, arguments: {{path: {p:?}}}}});\n\
         return hits[0].name + ' ' + (hits[0].schema ? 'has schema' : 'no schema') + ' ' + listing.includes('a.txt');",
        p = dir.to_str().unwrap()
    );
    let (msg_id, result) = step(&mut session, &Ledgers::new(), run_code_call(&code, None)).await;
    assert!(result.starts_with("list_dir has schema true"), "{result}");
    assert_eq!(
        audit(&session, &msg_id)[0].0,
        "list_dir",
        "audited under the real name"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[tokio::test]
async fn the_deadline_never_cancels_an_in_flight_call_and_refuses_the_next() {
    let dir = workspace("deadline");
    let mut session = session_in(&dir, PermissionMode::Bypass, Approver::default());
    let code = format!(
        "await tools.shell({{command: 'sleep 2'}});\n\
         try {{ await tools.list_dir({{path: {p:?}}}); return 'second call ran'; }}\n\
         catch (e) {{ return 'second refused: ' + e.message; }}",
        p = dir.to_str().unwrap()
    );
    let (msg_id, result) = step(&mut session, &Ledgers::new(), run_code_call(&code, Some(1))).await;
    assert!(
        result.starts_with("second refused: tools.list_dir(")
            && result.contains(") failed: run_code deadline exceeded"),
        "{result}"
    );
    let rows = audit(&session, &msg_id);
    assert_eq!(
        rows[0],
        ("shell".to_string(), "ok".to_string()),
        "the slow call finished"
    );
    assert!(!rows.iter().any(|(n, _)| n == "list_dir"), "{rows:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_script_that_never_settles_is_stopped_after_its_deadline() {
    let dir = workspace("hang");
    let mut session = session_in(&dir, PermissionMode::AcceptEdits, Approver::default());
    let started = std::time::Instant::now();
    let (msg_id, result) = step(
        &mut session,
        &Ledgers::new(),
        run_code_call("await new Promise(() => {});", Some(1)),
    )
    .await;
    assert!(
        result.starts_with("error: run_code deadline exceeded after 1s"),
        "{result}"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(8));
    assert_eq!(
        audit(&session, &msg_id),
        vec![("run_code".into(), "error".into())]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[tokio::test]
async fn time_on_a_permission_prompt_does_not_count_against_the_budget() {
    let dir = workspace("prompt");
    let presenter = Approver {
        delay_ms: 1_500,
        ..Approver::default()
    };
    let prompts = Arc::clone(&presenter.prompts);
    // Ask mode prompts before a mutating shell; the approval outlasts the whole 1s budget.
    let mut session = session_in(&dir, PermissionMode::Default, presenter);
    let code = format!(
        "await tools.shell({{command: 'mkdir made'}});\n\
         return await tools.list_dir({{path: {p:?}}});",
        p = dir.to_str().unwrap()
    );
    let (msg_id, result) = step(&mut session, &Ledgers::new(), run_code_call(&code, Some(1))).await;
    assert_eq!(
        prompts.load(Ordering::SeqCst),
        1,
        "the shell call asked: {result}"
    );
    assert!(!result.contains("deadline"), "{result}");
    let names: Vec<String> = audit(&session, &msg_id).into_iter().map(|r| r.0).collect();
    assert_eq!(names, vec!["shell", "list_dir", "run_code"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_kill_switch_hides_the_tool_and_refuses_the_call() {
    let dir = workspace("off");
    let on = session_in(&dir, PermissionMode::AcceptEdits, Approver::default());
    assert!(on
        .tool_specs()
        .iter()
        .any(|s| s.name == codemode::RUN_CODE_TOOL));
    let mut config = Config::default();
    config.codemode.enabled = false;
    let mut off = session_with_config(
        &dir,
        PermissionMode::AcceptEdits,
        Approver::default(),
        config,
    );
    assert!(!off
        .tool_specs()
        .iter()
        .any(|s| s.name == codemode::RUN_CODE_TOOL));
    let (_, result) = step(&mut off, &Ledgers::new(), run_code_call("return 1;", None)).await;
    assert!(
        result.starts_with("error: unknown tool 'run_code'"),
        "{result}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_wrapper_never_clears_the_failure_latch_or_counts_a_returned_test_failure() {
    let dir = workspace("latch");
    let mut session = session_in(&dir, PermissionMode::AcceptEdits, Approver::default());
    let l = Ledgers::new();
    let mut failure_counts = std::collections::HashMap::new();
    let mut failure_nudged = true;
    let mut observations = Vec::new();
    let mut ledgers = crate::tool_outcome::TurnLedgers {
        tools_ran: &l.tools_ran,
        inspect_ran: &l.inspect_ran,
        mutations_ran: &l.mutations_ran,
        verification_ledger: &l.verification,
        failure_counts: &mut failure_counts,
        failure_nudged: &mut failure_nudged,
        observations: &mut observations,
    };
    // The script returned a failing test's output: the inner shell call already counted that.
    let call = run_code_call("return 1;", None);
    session.observe_tool_outcome(
        &call,
        "shell: exit 101\ntest foo ... FAILED\n\n[run_code: 1 tool call, 0 ok, 1 failed, 3.1s]",
        &mut ledgers,
    );
    assert!(
        *ledgers.failure_nudged,
        "a script returning is not recovery"
    );
    assert!(ledgers.failure_counts.is_empty());
    session.observe_tool_outcome(
        &call,
        "error: ReferenceError: x is not defined\n\n[run_code: 0 tool calls, 0 ok, 0 failed, 0.0s]",
        &mut ledgers,
    );
    assert_eq!(
        ledgers.failure_counts.len(),
        1,
        "a thrown script is a run_code failure"
    );
    assert_eq!(l.mutations_ran.load(Ordering::SeqCst), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[path = "codemode_deferred.rs"]
mod deferred;
