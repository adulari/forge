//! Behaviour assertions: the QA runner starts the thing, probes it, keeps the proof and leaves
//! nothing running.

use super::goal_contract_tests::harness;
use super::*;
use forge_types::goal_contract::behaviour::Behaviour;
use forge_types::goal_contract::{Assertion, GoalContract};

fn behaviour(run: &str, probe: &str, expect: &str, cleanup: &str) -> Assertion {
    let b = Behaviour {
        run: run.into(),
        probe: probe.into(),
        expect: expect.into(),
        cleanup: cleanup.into(),
        ready: String::new(),
    };
    Assertion {
        id: 1,
        text: "the server answers /health".into(),
        command: b.summary(),
        expect: expect.into(),
        contains: None,
        behaviour: Some(b),
    }
}

fn contract(assertions: Vec<Assertion>) -> GoalContract {
    let mut c = GoalContract::new(assertions);
    c.approved = true;
    c
}

const SERVER: &str = r#"import http.server, sys
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.end_headers(); self.wfile.write(b"ok VERSION")
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
"#;

fn pid_gone(pid: i32) -> bool {
    // SAFETY: signal 0 only checks existence.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

#[tokio::test]
async fn a_server_that_answers_the_wrong_text_fails_then_passes_once_fixed() {
    let mut h = harness(&["x"], "Approve", Config::default());
    let root = h.dir.path().to_path_buf();
    std::fs::write(root.join("server.py"), SERVER.replace("VERSION", "v1")).unwrap();
    let run = "echo $$ > pid.txt; exec python3 server.py $FORGE_QA_PORT";
    let probe = "GET http://127.0.0.1:$FORGE_QA_PORT/health";
    let c = contract(vec![behaviour(run, probe, "ok v2", "touch cleaned.txt")]);

    let first = h.session.validate_goal_contract(&c).await;
    assert!(!first[0].passed, "{:?}", first[0]);
    assert!(first[0].reason.contains("does not contain \"ok v2\""));
    assert!(first[0].proof.contains("ok v1"), "{}", first[0].proof);
    assert!(root.join("cleaned.txt").exists(), "cleanup always runs");
    let pid: i32 = std::fs::read_to_string(root.join("pid.txt"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(pid_gone(pid), "the server must not outlive the check");

    std::fs::write(root.join("server.py"), SERVER.replace("VERSION", "v2")).unwrap();
    let second = h.session.validate_goal_contract(&c).await;
    assert!(second[0].passed, "{:?}", second[0]);
    assert!(second[0].proof.contains("ok v2"));
    let pid: i32 = std::fs::read_to_string(root.join("pid.txt"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(pid_gone(pid));
}

#[tokio::test]
async fn behaviour_waits_for_the_read_only_checks() {
    let mut h = harness(&["x"], "Approve", Config::default());
    let mut read_only = Assertion {
        id: 1,
        text: "marker exists".into(),
        command: "test -f marker".into(),
        expect: String::new(),
        contains: None,
        behaviour: None,
    };
    let mut qa = behaviour("touch ran.txt", "echo hi", "hi", "");
    qa.id = 2;
    read_only.id = 1;
    let results = h
        .session
        .validate_goal_contract(&contract(vec![read_only, qa]))
        .await;
    assert!(!results[0].passed);
    assert!(!results[1].passed);
    assert!(results[1].reason.starts_with("not run"));
    assert!(!h.dir.path().join("ran.txt").exists());
}

#[tokio::test]
async fn a_process_that_dies_fails_with_its_log() {
    let mut h = harness(&["x"], "Approve", Config::default());
    let c = contract(vec![behaviour(
        "echo boom >&2; exit 3",
        "GET http://127.0.0.1:1/",
        "",
        "",
    )]);
    let r = h.session.validate_goal_contract(&c).await;
    assert!(!r[0].passed);
    assert!(r[0].reason.contains("exit 3"), "{}", r[0].reason);
    assert!(r[0].proof.contains("boom"), "{}", r[0].proof);
}

#[tokio::test]
async fn a_shell_probe_runs_a_cli_and_a_deny_rule_still_binds() {
    let mut h = harness(&["x"], "Approve", Config::default());
    std::fs::write(h.dir.path().join("cli.sh"), "echo \"got $1\"\n").unwrap();
    let ok = contract(vec![behaviour(":", "sh cli.sh seven", "got seven", "")]);
    let r = h.session.validate_goal_contract(&ok).await;
    assert!(r[0].passed, "{:?}", r[0]);
    let denied = contract(vec![behaviour(":", "sudo true", "", "")]);
    let r = h.session.validate_goal_contract(&denied).await;
    assert!(!r[0].passed);
    assert!(r[0].reason.starts_with("refused"), "{}", r[0].reason);
}

#[tokio::test]
async fn a_screenshot_probe_without_a_browser_is_skipped_with_the_reason() {
    let mut h = harness(&["x"], "Approve", Config::default());
    if h.session.tools.get("browser").is_some() {
        return;
    }
    let c = contract(vec![behaviour(
        ":",
        "screenshot http://127.0.0.1:1/",
        "",
        "",
    )]);
    let r = h.session.validate_goal_contract(&c).await;
    assert!(r[0].passed);
    assert!(r[0].proof.starts_with("skipped:"), "{}", r[0].proof);
}
