use super::*;
use std::sync::{Arc, Mutex};

fn config() -> forge_config::codemode::CodemodeConfig {
    forge_config::codemode::CodemodeConfig::default()
}

/// Runs `code` through the real wrapper with a `__tool` that records each call and echoes
/// `name:<args json>`; a tool named `boom` rejects.
async fn run(code: &str) -> (serde_json::Value, Vec<String>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    let host = forge_workflow::HostFunction::new("__tool", move |args: Vec<serde_json::Value>| {
        let log = Arc::clone(&log);
        async move {
            let name = args[0].as_str().unwrap_or_default().to_string();
            log.lock().unwrap().push(name.clone());
            if name == "boom" {
                return Err("error: boom failed".to_string());
            }
            Ok(serde_json::Value::String(format!("{name}:{}", args[1])))
        }
    });
    let value = forge_workflow::run_script(vec![host], &wrap(code))
        .await
        .expect("the wrapper always resolves to a report object");
    let calls = seen.lock().unwrap().clone();
    (value, calls)
}

#[tokio::test]
async fn tools_proxy_calls_by_name_and_returns_the_value() {
    let (value, calls) = run("const a = await tools.read_file({path: 'x'});\n\
         const [b, c] = await Promise.all([tools.search({q: 1}), tools.glob()]);\n\
         return [a, b, c];")
    .await;
    let (end, logs, _) = parse_script_value(value);
    assert!(logs.is_empty());
    let ScriptEnd::Returned(v) = end else {
        panic!("expected a return, got {end:?}")
    };
    assert_eq!(v[0], "read_file:{\"path\":\"x\"}");
    assert_eq!(v[2], "glob:{}", "a missing argument object becomes {{}}");
    assert_eq!(calls, vec!["read_file", "search", "glob"]);
}

#[tokio::test]
async fn awaiting_the_proxy_or_reading_symbol_keys_calls_no_tool() {
    let (value, calls) = run("const t = await tools;\n\
         const s = tools[Symbol.iterator];\n\
         const j = JSON.stringify({k: 1});\n\
         return [typeof t, s === undefined, tools.then === undefined, j];")
    .await;
    let (end, _, _) = parse_script_value(value);
    let ScriptEnd::Returned(v) = end else {
        panic!("expected a return, got {end:?}")
    };
    assert_eq!(v, serde_json::json!(["object", true, true, "{\"k\":1}"]));
    assert!(calls.is_empty(), "no tool call for then/Symbol: {calls:?}");
}

#[tokio::test]
async fn logs_survive_a_thrown_error_and_a_rejected_call_is_catchable() {
    let (value, _) = run("console.log('start', {n: 1});\n\
         let caught = '';\n\
         try { await tools.boom(); } catch (e) { caught = e.message; }\n\
         log('caught', caught);\n\
         throw new TypeError('bad');")
    .await;
    let (end, logs, _) = parse_script_value(value);
    assert!(
        matches!(&end, ScriptEnd::Threw(m) if m == "TypeError: bad"),
        "{end:?}"
    );
    assert_eq!(logs[0], "start {\"n\":1}");
    assert!(logs[1].contains("error: boom failed"), "{logs:?}");
}

#[tokio::test]
async fn search_goes_through_the_same_channel() {
    let (_, calls) = run("return await tools.$search('fetch');").await;
    assert_eq!(calls, vec![SEARCH_CALL]);
}

#[tokio::test]
async fn engine_errors_report_the_models_own_line_numbers() {
    let err = forge_workflow::run_script(vec![], &wrap("const ok = 1;\nconst = ;"))
        .await
        .unwrap_err();
    assert!(err.contains("eval_script:3"), "{err}");
    let mapped = engine_error(&err);
    assert!(mapped.contains("line 2"), "{mapped}");
    assert!(!mapped.contains("eval_script"), "{mapped}");
}

#[test]
fn fenced_and_plain_code_both_parse() {
    let (code, timeout) = script_from_args(
        &serde_json::json!({"code": "```js\nreturn 1;\n```", "timeout_secs": 5000}),
        &config(),
    )
    .unwrap();
    assert_eq!(code.trim(), "return 1;");
    assert_eq!(timeout, 900, "clamped to max_timeout_secs");
    let (code, timeout) =
        script_from_args(&serde_json::json!({"code": "return 2;"}), &config()).unwrap();
    assert_eq!((code.as_str(), timeout), ("return 2;", 300));
    let (code, _) = script_from_args(
        &serde_json::json!({"script": "```\nreturn 3;```"}),
        &config(),
    )
    .unwrap();
    assert_eq!(code.trim(), "return 3;");
    assert!(script_from_args(&serde_json::json!({"code": "  "}), &config()).is_err());
}

#[test]
fn only_calls_that_did_not_run_throw() {
    assert!(throws("error: unknown tool 'x'"));
    assert!(throws("permission denied by policy"));
    assert!(throws("blocked by hook: no"));
    assert!(throws("doom-loop: `read_file` called identically 3 times"));
    assert!(!throws("shell: exit 1 in 5ms\n\nFAILED"));
    assert!(!throws("fn main() {}"));
}

#[test]
fn hints_ride_on_the_calls_value() {
    assert_eq!(with_hints("ok", &[]), "ok");
    let v = with_hints("wrote x", &["[lsp] error[E0308]".to_string()]);
    assert!(v.starts_with("wrote x") && v.ends_with("[hint] [lsp] error[E0308]"));
}

fn report(end: ScriptEnd, calls: Vec<(&str, bool)>) -> ScriptReport {
    ScriptReport {
        end,
        logs: vec!["a log".into()],
        dropped_logs: 0,
        calls: calls
            .into_iter()
            .map(|(name, ok)| InnerCall {
                name: name.into(),
                ok,
            })
            .collect(),
        deadline_hit: false,
        timeout_secs: 300,
        elapsed: std::time::Duration::from_millis(2400),
    }
}

#[test]
fn render_keeps_the_footer_and_caps_the_body() {
    let r = report(
        ScriptEnd::Returned(serde_json::Value::String("x".repeat(5_000))),
        vec![("read_file", true), ("edit_file", false)],
    );
    let out = render(&r, 1_000);
    assert!(
        out.chars().count() < 1_200,
        "capped: {}",
        out.chars().count()
    );
    assert!(
        out.ends_with("[run_code: 2 tool calls, 1 ok, 1 failed, 2.4s]"),
        "{out}"
    );
    assert!(out.contains("output truncated"));
}

#[test]
fn the_footer_is_stripped_for_progress_fingerprints() {
    let mut r = report(
        ScriptEnd::Returned(serde_json::Value::String("same answer".into())),
        vec![("read_file", true)],
    );
    r.logs.clear();
    let first = render(&r, 16_000);
    r.elapsed = std::time::Duration::from_millis(900);
    let second = render(&r, 16_000);
    assert_ne!(first, second);
    assert_eq!(without_footer(&first), "same answer");
    assert_eq!(without_footer(&first), without_footer(&second));
    assert_eq!(without_footer("no footer here"), "no footer here");
}

#[test]
fn a_failed_script_names_the_calls_that_already_ran() {
    let r = report(
        ScriptEnd::Threw("ReferenceError: fs is not defined".into()),
        vec![("edit_file", true)],
    );
    let out = render(&r, 16_000);
    assert!(out.starts_with("error: ReferenceError"), "{out}");
    assert!(out.contains("already ran and keep their effects"), "{out}");
    assert!(out.contains("edit_file"));
    assert!(out.contains("no fs/fetch/require"), "{out}");
    assert!(out.contains("[logs]\na log"));
    let expired = render(&report(ScriptEnd::Expired, vec![]), 16_000);
    assert!(
        expired.contains("deadline exceeded after 300s"),
        "{expired}"
    );
}

#[test]
fn the_spec_is_static_and_requires_code() {
    let a = run_code_spec();
    let b = run_code_spec();
    assert_eq!(a.description, b.description);
    assert_eq!(a.schema["required"], serde_json::json!(["code"]));
}
