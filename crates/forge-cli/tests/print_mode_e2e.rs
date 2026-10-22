//! `forge run` as a drop-in for `claude -p`, through the real binary and the mock provider:
//! print-mode stdout, Claude-Code-shaped stream-json, stream-json input and the session flags.

use std::io::Write;
use std::process::{Command, Output, Stdio};

const MOCK_ANSWER: &str = "Done — I read the project manifest and the workspace looks healthy.";

fn forge(workspace: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_forge"));
    command
        .current_dir(workspace)
        .env("FORGE_DB", workspace.join("forge.db"))
        .env("XDG_CONFIG_HOME", workspace.join("cfg"))
        .env("XDG_DATA_HOME", workspace.join("data"))
        .env("HOME", workspace)
        .stdin(Stdio::null());
    command
}

fn ok(output: Output) -> Output {
    assert!(
        output.status.success(),
        "forge failed ({:?}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn events(output: &Output) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect()
}

#[test]
fn print_mode_stdout_is_exactly_the_final_answer() {
    let ws = tempfile::tempdir().unwrap();
    let out = ok(forge(ws.path())
        .args(["run", "-p", "say hello", "--mock"])
        .output()
        .unwrap());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("{MOCK_ANSWER}\n")
    );
    assert!(
        out.stderr.is_empty(),
        "no trace on a non-terminal stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let verbose = ok(forge(ws.path())
        .args(["run", "-p", "say hello", "--mock", "--verbose"])
        .output()
        .unwrap());
    assert_eq!(
        String::from_utf8_lossy(&verbose.stdout),
        format!("{MOCK_ANSWER}\n")
    );
    let trace = String::from_utf8_lossy(&verbose.stderr);
    assert!(trace.contains("● session"), "{trace}");
    assert!(trace.contains("↳ read_file"), "{trace}");
}

#[test]
fn run_without_p_prints_only_the_answer_when_stdout_is_not_a_terminal() {
    let ws = tempfile::tempdir().unwrap();
    for extra in [&[][..], &["--quiet"][..], &["--output-format", "text"][..]] {
        let out = ok(forge(ws.path())
            .args(["run", "say hello", "--mock"])
            .args(extra)
            .output()
            .unwrap());
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("{MOCK_ANSWER}\n"),
            "{extra:?}"
        );
        assert!(
            out.stderr.is_empty(),
            "{extra:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let verbose = ok(forge(ws.path())
        .args(["run", "say hello", "--mock", "--verbose"])
        .output()
        .unwrap());
    assert_eq!(
        String::from_utf8_lossy(&verbose.stdout),
        format!("{MOCK_ANSWER}\n")
    );
    assert!(String::from_utf8_lossy(&verbose.stderr).contains("↳ read_file"));
}

#[test]
fn quiet_and_verbose_exclude_each_other() {
    let ws = tempfile::tempdir().unwrap();
    let out = forge(ws.path())
        .args(["run", "hi", "--mock", "--quiet", "--verbose"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn verbose_adds_status_events_to_the_stream_and_nothing_to_the_answer() {
    let ws = tempfile::tempdir().unwrap();
    let stream = |verbose: bool| {
        let mut command = forge(ws.path());
        command.args(["run", "hi", "--mock", "--output-format", "stream-json"]);
        if verbose {
            command.arg("--verbose");
        }
        events(&ok(command.output().unwrap()))
    };
    let kind = |e: &serde_json::Value| format!("{}/{}", e["type"], e["subtype"]);
    let (plain, verbose) = (stream(false), stream(true));
    assert!(!plain.iter().any(|e| e["subtype"] == "status"));
    assert!(verbose.iter().any(|e| e["subtype"] == "status"));
    let rest: Vec<String> = verbose
        .iter()
        .filter(|e| e["subtype"] != "status")
        .map(kind)
        .collect();
    assert_eq!(rest, plain.iter().map(kind).collect::<Vec<_>>());
    let usage = plain.iter().filter(|e| e["subtype"] == "usage").count() as u64;
    let calls = plain.last().unwrap()["num_turns"].as_u64().unwrap();
    assert_eq!(
        usage, calls,
        "one running total per model call, none repeated"
    );
}

#[test]
fn json_result_carries_claude_codes_usage_fields() {
    let ws = tempfile::tempdir().unwrap();
    let out = ok(forge(ws.path())
        .args(["run", "hi", "--mock", "--output-format", "json"])
        .output()
        .unwrap());
    let result = &events(&out)[0];
    for field in [
        "uuid",
        "duration_ms",
        "duration_api_ms",
        "num_turns",
        "total_cost_usd",
    ] {
        assert!(!result[field].is_null(), "{field}: {result}");
    }
    assert!(result["permission_denials"].is_array(), "{result}");
    for field in [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
        "output_tokens",
    ] {
        assert!(result["usage"][field].is_u64(), "usage.{field}: {result}");
    }
    let models = result["modelUsage"].as_object().unwrap();
    assert_eq!(models.len(), 1, "{result}");
    let (_, usage) = models.iter().next().unwrap();
    for field in [
        "inputTokens",
        "outputTokens",
        "cacheReadInputTokens",
        "cacheCreationInputTokens",
        "costUSD",
    ] {
        assert!(usage[field].is_number(), "modelUsage.{field}: {result}");
    }
}

#[test]
fn stream_json_matches_claude_code_shapes() {
    let ws = tempfile::tempdir().unwrap();
    let out = ok(forge(ws.path())
        .args([
            "run",
            "-p",
            "say hello",
            "--mock",
            "--output-format",
            "stream-json",
            "--verbose",
        ])
        .output()
        .unwrap());
    let lines = events(&out);
    let init = &lines[0];
    assert_eq!(
        (init["type"].as_str(), init["subtype"].as_str()),
        (Some("system"), Some("init"))
    );
    assert!(
        init["tools"].as_array().is_some_and(|t| !t.is_empty()),
        "{init}"
    );
    assert!(
        init["cwd"].is_string() && init["permissionMode"].is_string(),
        "{init}"
    );
    assert!(
        init["mcp_servers"].is_array() && init["model"].is_string(),
        "{init}"
    );

    let texts: Vec<&str> = lines
        .iter()
        .filter(|e| e["type"] == "assistant")
        .flat_map(|e| e["message"]["content"].as_array().unwrap())
        .filter(|b| b["type"] == "text")
        .map(|b| b["text"].as_str().unwrap())
        .collect();
    assert_eq!(
        texts,
        ["Let me inspect the project manifest.", MOCK_ANSWER],
        "whole messages"
    );
    assert!(
        lines.iter().all(|e| e["type"] != "stream_event"),
        "partials only on request"
    );

    let result = lines.last().unwrap();
    assert_eq!(result["type"], "result");
    assert_eq!(result["subtype"], "success");
    assert_eq!(result["result"], MOCK_ANSWER);
    assert_eq!(result["session_id"], init["session_id"]);
    for field in ["duration_ms", "num_turns"] {
        assert!(result[field].is_u64(), "{field}: {result}");
    }
    assert!(result["total_cost_usd"].is_number() && result["usage"]["input_tokens"].is_u64());

    let partial = ok(forge(ws.path())
        .args([
            "run",
            "-p",
            "say hello",
            "--mock",
            "--output-format",
            "stream-json",
            "--include-partial-messages",
        ])
        .output()
        .unwrap());
    let streamed: String = events(&partial)
        .iter()
        .filter(|e| e["type"] == "stream_event" && e["event"]["delta"]["type"] == "text_delta")
        .map(|e| e["event"]["delta"]["text"].as_str().unwrap().to_string())
        .collect();
    assert!(streamed.ends_with(MOCK_ANSWER), "{streamed}");
}

#[test]
fn stream_json_input_runs_each_user_message_in_one_session() {
    let ws = tempfile::tempdir().unwrap();
    let mut child = forge(ws.path())
        .args([
            "run",
            "-p",
            "--mock",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            concat!(
                r#"{"type":"user","message":{"role":"user","content":"first"}}"#,
                "\n",
                r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"second"}]}}"#,
                "\n"
            )
            .as_bytes(),
        )
        .unwrap();
    let out = ok(child.wait_with_output().unwrap());
    let events = events(&out);
    let results: Vec<_> = events.iter().filter(|e| e["type"] == "result").collect();
    assert_eq!(results.len(), 2, "one result per message");
    assert_eq!(results[0]["session_id"], results[1]["session_id"]);
    assert_eq!(events.iter().filter(|e| e["subtype"] == "init").count(), 1);
}

#[test]
fn session_flags_name_fix_the_id_and_skip_persistence() {
    let ws = tempfile::tempdir().unwrap();
    let id = "550e8400-e29b-41d4-a716-446655440000";
    let out = ok(forge(ws.path())
        .args(["run", "-p", "hi", "--mock", "--output-format", "json"])
        .args(["--session-id", id, "-n", "nightly-review"])
        .output()
        .unwrap());
    assert_eq!(events(&out)[0]["session_id"], id);
    let taken = forge(ws.path())
        .args(["run", "-p", "hi", "--mock", "--session-id", id])
        .output()
        .unwrap();
    assert!(!taken.status.success(), "an id in use is refused");

    let resumed = ok(forge(ws.path())
        .args(["run", "-p", "again", "--mock", "--output-format", "json"])
        .args(["--resume", "nightly-review"])
        .output()
        .unwrap());
    assert_eq!(events(&resumed)[0]["session_id"], id, "resumed by name");

    let unsaved = ok(forge(ws.path())
        .args(["run", "-p", "hi", "--mock", "--output-format", "json"])
        .arg("--no-session-persistence")
        .output()
        .unwrap());
    let gone = events(&unsaved)[0]["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    let missing = forge(ws.path())
        .args(["run", "-p", "x", "--mock", "--resume", &gone])
        .output()
        .unwrap();
    assert!(
        !missing.status.success(),
        "an unsaved session cannot be resumed"
    );
}

#[test]
fn claude_print_flags_are_accepted_together() {
    let ws = tempfile::tempdir().unwrap();
    let extra = ws.path().join("extra");
    std::fs::create_dir(&extra).unwrap();
    let prompt_file = ws.path().join("rules.txt");
    std::fs::write(&prompt_file, "Answer in one line.").unwrap();
    let out = ok(forge(ws.path())
        .args([
            "run",
            "-p",
            "say hello",
            "--mock",
            "--output-format",
            "json",
        ])
        .args(["--system-prompt", "You are a careful reviewer."])
        .args(["--append-system-prompt-file", prompt_file.to_str().unwrap()])
        .args(["--add-dir", extra.to_str().unwrap()])
        .args(["--effort", "max", "--max-budget-usd", "5"])
        .args(["--setting-sources", "user,project,local"])
        .args([
            "--strict-mcp-config",
            "--mcp-config",
            r#"{"mcpServers":{}}"#,
        ])
        .args([
            "--fallback-model",
            "openai::gpt-4o-mini",
            "--permission-mode",
            "dontAsk",
        ])
        .output()
        .unwrap());
    assert_eq!(events(&out)[0]["subtype"], "success");
}
