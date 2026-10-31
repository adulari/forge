use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::json;

use super::discover::discover_in;
use super::output::{self, HeadTail, Rendered};
use super::process::{Kernel, Piece, Splitter};
use super::tool::{parse_args, strip_fence};
use super::PythonTool;
use crate::{Tool, ToolRun, SESSION_WORKSPACE};

fn python3_available() -> bool {
    let ok = std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("python3").is_file()));
    if !ok {
        eprintln!("skipping: no python3 on PATH");
    }
    ok
}

async fn call(tool: &PythonTool, ws: &Path, args: serde_json::Value) -> ToolRun {
    SESSION_WORKSPACE
        .scope(ws.to_path_buf(), tool.run_full(&args))
        .await
        .expect("python call")
}

async fn code(tool: &PythonTool, ws: &Path, src: &str) -> String {
    call(tool, ws, json!({ "code": src })).await.model
}

fn is_gone(pid: i32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Err(_) => true,
        Ok(stat) => stat
            .rsplit(')')
            .next()
            .is_some_and(|r| r.trim_start().starts_with('Z')),
    }
}

async fn gone_within(pid: i32, limit: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if is_gone(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    is_gone(pid)
}

#[test]
fn args_require_code_strip_one_fence_and_clamp_timeout() {
    assert!(parse_args(&json!({})).is_err());
    assert!(parse_args(&json!({ "code": "  \n" })).is_err());
    assert!(parse_args(&json!({ "code": "```python\n```" })).is_err());
    let cell =
        parse_args(&json!({ "code": "```python\nx = 1\n```", "timeout_secs": 9999 })).unwrap();
    assert_eq!(cell.code, "x = 1");
    assert_eq!(cell.timeout_secs, 600);
    assert_eq!(
        parse_args(&json!({ "code": "x", "timeout_secs": 0 }))
            .unwrap()
            .timeout_secs,
        1
    );
    assert_eq!(
        parse_args(&json!({ "code": "x" })).unwrap().timeout_secs,
        60
    );
    assert_eq!(strip_fence("```\nprint(1)\n```"), "print(1)");
    assert_eq!(strip_fence("```js\nx\n```"), "```js\nx\n```");
    assert_eq!(strip_fence("a = '```'"), "a = '```'");
}

fn collect(splitter: &mut Splitter, chunks: &[&[u8]]) -> (Vec<u8>, Vec<u64>) {
    let (mut data, mut ends) = (Vec::new(), Vec::new());
    for chunk in chunks {
        for piece in splitter.feed(chunk) {
            match piece {
                Piece::Data(d) => data.extend(d),
                Piece::CellEnd(n) => ends.push(n),
            }
        }
    }
    (data, ends)
}

#[test]
fn splitter_handles_split_chunks_wrong_nonce_and_trailing_bytes() {
    let mut s = Splitter::new("abc");
    let (data, ends) = collect(&mut s, &[b"out\n\x1eFORGE-", b"END abc 3\x1e", b"\nnext"]);
    assert_eq!((data.as_slice(), ends), (b"out\nnext".as_slice(), vec![3]));

    let mut s = Splitter::new("abc");
    let (data, ends) = collect(&mut s, &[b"a\x1eFORGE-END zzz 4\x1e\nb"]);
    assert!(ends.is_empty());
    assert_eq!(data, b"a\x1eFORGE-END zzz 4\x1e\nb");

    let mut s = Splitter::new("abc");
    let (data, ends) = collect(
        &mut s,
        &[b"\x1eFORGE-END abc 1\x1e\nlater\x1eFORGE-END abc 2\x1e\ntail"],
    );
    assert_eq!(
        (data.as_slice(), ends),
        (b"latertail".as_slice(), vec![1, 2])
    );
}

#[test]
fn output_collapses_progress_strips_ansi_and_truncates_on_char_boundaries() {
    assert_eq!(
        output::collapse_cr("a\r\nb\rc\r  50%\r100%\nend"),
        "a\n100%\nend"
    );
    assert_eq!(output::clean_output("\x1b[31mred\x1b[0m"), "red");

    let text = "é".repeat(20_000);
    let run = output::assemble(Rendered {
        header: "python: ok (cell 1, 0.0s)".into(),
        notice: None,
        output: text.clone(),
        value: None,
        traceback: None,
        capture_truncated: false,
    });
    assert!(run.model.len() <= output::RESULT_MAX_CHARS + 64);
    assert!(run.model.starts_with("python: ok"));
    assert_eq!(run.full.as_deref().map(|f| f.contains(&text)), Some(true));

    let small = output::assemble(Rendered {
        header: "python: ok (cell 1, 0.0s)".into(),
        notice: Some("new kernel".into()),
        output: "hi\n".into(),
        value: Some("42".into()),
        traceback: None,
        capture_truncated: false,
    });
    assert_eq!(
        small.model,
        "python: ok (cell 1, 0.0s)\n[new kernel]\nhi\nOut: 42"
    );
    assert!(small.full.is_none());
}

#[test]
fn headers_have_the_documented_shapes() {
    assert_eq!(output::ok_header(4, 300, None), "python: ok (cell 4, 0.3s)");
    assert_eq!(
        output::ok_header(4, 300, Some(1)),
        "python: ok (cell 4, 0.3s; a ! command exited 1)"
    );
    assert_eq!(
        output::error_header(4, "KeyError"),
        "python: error KeyError (cell 4)"
    );
    assert_eq!(
        output::interrupted_header(60),
        "python: interrupted after 60s; kernel state kept"
    );
    assert_eq!(
        output::KILLED_HEADER,
        "python: kernel killed after ignoring interrupt; state lost"
    );
    assert_eq!(
        output::died_header("SIGKILL, likely out of memory"),
        "python: kernel died (SIGKILL, likely out of memory); state lost"
    );
    assert_eq!(
        output::failed_header(&["a".into(), "b".into()], "boom"),
        "python: failed to start (tried a, b): boom; use shell"
    );
}

#[test]
fn head_tail_keeps_both_ends() {
    let mut ht = HeadTail::default();
    ht.push(&vec![b'a'; 100_000]);
    ht.push(&vec![b'b'; 100_000]);
    let (text, cut) = ht.into_text();
    assert!(cut);
    assert!(text.starts_with("aaa") && text.ends_with("bbb") && text.contains("bytes omitted"));
}

#[test]
fn discovery_orders_venv_path_and_collapses_duplicates() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("ws");
    let bin = dir.path().join("bin");
    let bin2 = dir.path().join("bin2");
    for d in [ws.join(".venv/bin"), bin.clone(), bin2.clone()] {
        std::fs::create_dir_all(d).unwrap();
    }
    let make = |p: &Path| {
        std::fs::write(p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    make(&ws.join(".venv/bin/python"));
    make(&bin.join("python3"));
    make(&bin.join("python"));
    make(&bin2.join("python3"));
    std::os::unix::fs::symlink(bin.join("python3"), bin2.join("python")).unwrap();
    let path = std::env::join_paths([&bin, &bin2]).unwrap();
    let found = discover_in(&ws, Some(&path), Path::new("/nonexistent/conda"));
    assert_eq!(
        found,
        vec![
            ws.join(".venv/bin/python"),
            bin.join("python3"),
            bin.join("python")
        ]
    );

    let none = discover_in(
        &ws.join("nowhere"),
        Some(std::ffi::OsStr::new("/nonexistent")),
        Path::new("/nonexistent/conda"),
    );
    assert!(none.is_empty());
}

#[tokio::test]
async fn state_persists_with_one_new_kernel_notice() {
    if !python3_available() {
        return;
    }
    let ws = tempfile::tempdir().unwrap();
    let tool = PythonTool::new();
    let first = code(&tool, ws.path(), "x = 41\nimport math").await;
    assert!(first.starts_with("python: ok (cell 1,"), "{first}");
    assert!(first.contains("[new kernel: no variables from earlier calls exist]"));
    let second = code(&tool, ws.path(), "x + 1").await;
    assert!(second.starts_with("python: ok (cell 2,"), "{second}");
    assert!(second.contains("Out: 42"));
    assert!(!second.contains("new kernel"));
    assert!(code(&tool, ws.path(), "math.floor(2.5)")
        .await
        .contains("Out: 2"));
}

#[tokio::test]
async fn exceptions_name_the_cell_and_hide_runner_frames() {
    if !python3_available() {
        return;
    }
    let ws = tempfile::tempdir().unwrap();
    let tool = PythonTool::new();
    code(&tool, ws.path(), "1").await;
    let out = code(&tool, ws.path(), "a = 1\nb = a / 0").await;
    assert!(
        out.contains("python: error ZeroDivisionError (cell 2)"),
        "{out}"
    );
    assert!(
        out.contains("<cell 2>") && out.contains("b = a / 0"),
        "{out}"
    );
    assert!(
        !out.contains("runner") && !out.contains("exec(compile"),
        "{out}"
    );
}

#[tokio::test]
async fn bang_commands_interleave_and_report_exit_codes() {
    if !python3_available() {
        return;
    }
    let ws = tempfile::tempdir().unwrap();
    let tool = PythonTool::new();
    let out = code(&tool, ws.path(), "print('one')\n!echo hi\nprint('two')").await;
    let (one, hi, two) = (out.find("one"), out.find("hi\n"), out.find("two"));
    assert!(one < hi && hi < two, "{out}");
    let out = code(&tool, ws.path(), "!false").await;
    assert!(
        out.starts_with("python: ok (cell 2,") && out.contains("; a ! command exited 1)"),
        "{out}"
    );
    assert!(out.contains("[exit 1]"));
    assert!(code(
        &tool,
        ws.path(),
        "s = '''\n!not a command\n'''\nprint(s.strip())"
    )
    .await
    .contains("!not a command"));
}

#[tokio::test]
async fn input_gets_eof_and_reset_clears_state() {
    if !python3_available() {
        return;
    }
    let ws = tempfile::tempdir().unwrap();
    let tool = PythonTool::new();
    assert!(code(&tool, ws.path(), "input()")
        .await
        .contains("python: error EOFError"));
    code(&tool, ws.path(), "keep = 1").await;
    let out = call(&tool, ws.path(), json!({ "code": "keep", "reset": true }))
        .await
        .model;
    assert!(
        out.contains("NameError") && out.contains("fresh kernel (reset)"),
        "{out}"
    );
}

#[tokio::test]
async fn timeout_interrupts_and_keeps_state() {
    if !python3_available() {
        return;
    }
    let ws = tempfile::tempdir().unwrap();
    let tool = PythonTool::new();
    code(&tool, ws.path(), "y = 1").await;
    let out = call(
        &tool,
        ws.path(),
        json!({ "code": "while True: pass", "timeout_secs": 1 }),
    )
    .await
    .model;
    assert!(
        out.starts_with("python: interrupted after 1s; kernel state kept"),
        "{out}"
    );
    assert!(code(&tool, ws.path(), "y").await.contains("Out: 1"));
}

#[tokio::test]
async fn sigint_ignoring_cell_is_killed_and_next_call_says_so() {
    if !python3_available() {
        return;
    }
    let ws = tempfile::tempdir().unwrap();
    let tool = PythonTool::new();
    code(&tool, ws.path(), "y = 1").await;
    let src = "import signal\nsignal.signal(signal.SIGINT, signal.SIG_IGN)\nwhile True: pass";
    let out = call(&tool, ws.path(), json!({ "code": src, "timeout_secs": 1 }))
        .await
        .model;
    assert!(
        out.starts_with("python: kernel killed after ignoring interrupt; state lost"),
        "{out}"
    );
    let next = code(&tool, ws.path(), "y").await;
    assert!(
        next.contains("previous kernel exited") && next.contains("NameError"),
        "{next}"
    );
}

#[tokio::test]
async fn a_dying_kernel_is_reported_then_replaced() {
    if !python3_available() {
        return;
    }
    let ws = tempfile::tempdir().unwrap();
    let tool = PythonTool::new();
    let out = code(&tool, ws.path(), "import os\nos._exit(3)").await;
    assert!(
        out.starts_with("python: kernel died (exit 3); state lost"),
        "{out}"
    );
    let next = code(&tool, ws.path(), "1 + 1").await;
    assert!(
        next.contains("previous kernel exited (exit 3)") && next.contains("Out: 2"),
        "{next}"
    );
}

#[tokio::test]
async fn dropping_the_tool_kills_the_kernel_and_its_background_children() {
    if !python3_available() {
        return;
    }
    let ws = tempfile::tempdir().unwrap();
    let tool = PythonTool::new();
    let out = code(
        &tool,
        ws.path(),
        "import os\nprint('K', os.getpid())\n!sleep 300 & echo \"C $!\"",
    )
    .await;
    let pid = |tag: &str| -> i32 {
        out.lines()
            .find_map(|l| l.strip_prefix(tag))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    };
    let (kernel, child) = (pid("K "), pid("C "));
    assert!(!is_gone(kernel) && !is_gone(child), "{out}");
    drop(tool);
    assert!(gone_within(kernel, Duration::from_secs(1)).await);
    assert!(gone_within(child, Duration::from_secs(1)).await);
}

#[tokio::test]
async fn closing_the_control_pipe_makes_the_kernel_exit() {
    if !python3_available() {
        return;
    }
    let ws = tempfile::tempdir().unwrap();
    let python = super::discover_python(ws.path())
        .into_iter()
        .next()
        .expect("python");
    let mut kernel = Kernel::spawn(&python, ws.path()).await.unwrap();
    kernel.hello(Duration::from_secs(10)).await.unwrap();
    let pid = kernel.pid();
    kernel.close_control();
    assert!(gone_within(pid, Duration::from_secs(1)).await);
}

#[tokio::test]
async fn env_is_scrubbed_and_nonce_hidden() {
    if !python3_available() {
        return;
    }
    forge_types::child_env::scrub("FORGE_TEST_SCRUBBED_KEY");
    std::env::set_var("FORGE_TEST_SCRUBBED_KEY", "secret");
    std::env::set_var("FORGE_DB", "/nonexistent/forge.db");
    let ws = tempfile::tempdir().unwrap();
    let tool = PythonTool::new();
    let out = code(&tool, ws.path(), "import os\n[os.environ.get('FORGE_TEST_SCRUBBED_KEY'), os.environ.get('FORGE_DB'), os.environ.get('FORGE_KERNEL_NONCE'), os.environ.get('MPLBACKEND')]").await;
    assert!(out.contains("Out: [None, None, None, 'Agg']"), "{out}");
    forge_types::child_env::unscrub("FORGE_TEST_SCRUBBED_KEY");
}

#[tokio::test]
async fn huge_output_is_capped_for_the_model_and_kept_in_full() {
    if !python3_available() {
        return;
    }
    let ws = tempfile::tempdir().unwrap();
    let tool = PythonTool::new();
    let run = call(
        &tool,
        ws.path(),
        json!({ "code": "print('H' + 'x' * 1_000_000 + '\\nmiddle\\n' + 'y' * 1_000_000 + 'T')" }),
    )
    .await;
    assert!(
        run.model.len() <= output::RESULT_MAX_CHARS + 128,
        "{}",
        run.model.len()
    );
    assert!(run.model.contains("bytes truncated") && run.model.contains("python: ok"));
    let full = run.full.expect("full output");
    assert!(full.len() > run.model.len() && full.contains("bytes omitted"));
    assert!(full.contains("Hxxx") && full.trim_end().ends_with('T'));
}

#[tokio::test]
async fn an_aborted_cell_is_drained_and_state_survives() {
    if !python3_available() {
        return;
    }
    let ws = tempfile::tempdir().unwrap();
    let tool = PythonTool::new();
    code(&tool, ws.path(), "keep = 5").await;
    let aborted = tokio::time::timeout(
        Duration::from_millis(500),
        call(
            &tool,
            ws.path(),
            json!({ "code": "import time\ntime.sleep(30)" }),
        ),
    )
    .await;
    assert!(aborted.is_err());
    let out = code(&tool, ws.path(), "keep").await;
    assert!(
        out.contains("Out: 5") && !out.contains("new kernel"),
        "{out}"
    );
}

#[tokio::test]
async fn workspace_modules_do_not_shadow_the_runner() {
    if !python3_available() {
        return;
    }
    let ws = tempfile::tempdir().unwrap();
    for name in ["json.py", "signal.py", "types.py", "ast.py", "tokenize.py"] {
        std::fs::write(ws.path().join(name), "raise RuntimeError('shadowed')\n").unwrap();
    }
    std::fs::write(ws.path().join("mymodule.py"), "VALUE = 7\n").unwrap();
    let tool = PythonTool::new();
    let out = code(&tool, ws.path(), "import mymodule\nmymodule.VALUE").await;
    assert!(
        out.starts_with("python: ok") && out.contains("Out: 7"),
        "{out}"
    );
}

#[tokio::test]
async fn privileged_commands_are_stopped_by_the_path_shim() {
    if !python3_available() {
        return;
    }
    let ws = tempfile::tempdir().unwrap();
    let tool = PythonTool::new();
    let out = code(&tool, ws.path(), "import subprocess\nr = subprocess.run(['sudo', 'true'], capture_output=True, text=True)\nprint(r.returncode, r.stderr)").await;
    assert!(
        out.contains("126") && out.contains("Forge does not run privileged commands"),
        "{out}"
    );
}

#[tokio::test]
async fn moving_the_workspace_restarts_the_kernel() {
    if !python3_available() {
        return;
    }
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let tool = PythonTool::new();
    code(&tool, a.path(), "z = 1").await;
    let out = code(&tool, b.path(), "import os\nos.getcwd()").await;
    assert!(
        out.contains("kernel restarted in") && out.contains("Out:"),
        "{out}"
    );
}

#[tokio::test]
async fn a_missing_interpreter_reports_failed_to_start() {
    let ws = tempfile::tempdir().unwrap();
    let python = ws.path().join("fake-python");
    std::fs::write(&python, "#!/bin/sh\necho oops >&2\nexit 1\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut kernel = Kernel::spawn(&python, ws.path()).await.unwrap();
    let err = kernel.hello(Duration::from_secs(5)).await.unwrap_err();
    assert!(err.contains("oops") || err.contains("no output"), "{err}");
}

#[tokio::test]
async fn not_equal_continuation_lines_and_pickling_work() {
    if !python3_available() {
        return;
    }
    let ws = tempfile::tempdir().unwrap();
    let tool = PythonTool::new();
    let out = code(
        &tool,
        ws.path(),
        "a, b = 1, 2\nif (\n    a\n    != b\n):\n    r = 'differs'\nr",
    )
    .await;
    assert!(
        out.starts_with("python: ok") && out.contains("Out: 'differs'"),
        "{out}"
    );
    let out = code(
        &tool,
        ws.path(),
        "import pickle\nclass Model:\n    x = 1\ndef f(v):\n    return v\nm = pickle.loads(pickle.dumps(Model()))\n(pickle.loads(pickle.dumps(f)) is f, type(m).__name__, __name__)",
    )
    .await;
    assert!(out.contains("Out: (True, 'Model', '__main__')"), "{out}");
}
