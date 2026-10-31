use std::time::Duration;

use super::*;

fn action(id: &str, command: &str) -> ProjectAction {
    ProjectAction {
        id: id.into(),
        name: id.into(),
        command: command.into(),
        cwd: None,
        icon: None,
    }
}

fn driver(dir: &Path, actions: Vec<ProjectAction>) -> (RunMenuDriver, App) {
    let mut d = RunMenuDriver::in_dir(dir.to_path_buf());
    d.defs = actions;
    d.loaded = true;
    (d, App::default())
}

fn row<'a>(app: &'a App, id: &str) -> &'a ActionRow {
    app.run_menu
        .actions
        .iter()
        .find(|r| r.action.id == id)
        .unwrap()
}

async fn until(
    d: &mut RunMenuDriver,
    app: &mut App,
    what: &str,
    done: impl Fn(&RunMenuDriver, &App) -> bool,
) {
    for _ in 0..200 {
        d.tick(app);
        if done(d, app) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

fn gone(pid: i32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Err(_) => true,
        Ok(stat) => stat
            .rsplit(") ")
            .next()
            .is_some_and(|rest| rest.starts_with('Z')),
    }
}

fn read_pid(path: &Path) -> i32 {
    for _ in 0..100 {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(pid) = text.trim().parse() {
                return pid;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("no pid file");
}

#[tokio::test]
async fn output_and_exit_code_are_captured() {
    let dir = tempfile::tempdir().unwrap();
    let (mut d, mut app) = driver(
        dir.path(),
        vec![action("job", "echo hello; echo oops >&2; exit 3")],
    );
    d.start(&mut app, "job");
    until(&mut d, &mut app, "exit", |_, a| {
        matches!(row(a, "job").state, RunState::Exited { code: Some(3), .. })
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    d.tick(&mut app);
    let output = &row(&app, "job").output;
    assert!(output.iter().any(|l| l == "hello"), "{output:?}");
    assert!(output.iter().any(|l| l == "oops"), "{output:?}");
}

#[tokio::test]
async fn the_output_ring_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let (mut d, mut app) = driver(dir.path(), vec![action("noisy", "seq 1 1000")]);
    d.start(&mut app, "noisy");
    until(&mut d, &mut app, "exit", |_, a| {
        matches!(row(a, "noisy").state, RunState::Exited { .. })
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let shared = d.procs["noisy"].shared.clone();
    let out = shared.output.lock().unwrap();
    assert_eq!(out.len(), OUTPUT_CAP);
    assert_eq!(out.back().map(String::as_str), Some("1000"));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn stop_takes_the_whole_process_group_down() {
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("child.pid");
    let command = format!("sleep 30 & echo $! > {}; wait", pidfile.display());
    let (mut d, mut app) = driver(dir.path(), vec![action("dev", &command)]);
    d.start(&mut app, "dev");
    let shell = d.procs["dev"].pid as i32;
    let child = read_pid(&pidfile);
    until(&mut d, &mut app, "running", |_, a| {
        matches!(row(a, "dev").state, RunState::Running { .. })
    })
    .await;
    assert!(!gone(shell) && !gone(child));
    d.stop("dev");
    until(&mut d, &mut app, "stopped", |_, a| {
        matches!(row(a, "dev").state, RunState::Exited { stopped: true, .. })
    })
    .await;
    for _ in 0..100 {
        if gone(child) && gone(shell) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("a process of the action survived stop");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn stop_escalates_when_sigterm_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("pid");
    let command = format!(
        "trap '' TERM; echo $$ > {}; while true; do sleep 1; done",
        pidfile.display()
    );
    let (mut d, mut app) = driver(dir.path(), vec![action("stubborn", &command)]);
    d.start(&mut app, "stubborn");
    let pid = read_pid(&pidfile);
    d.stop("stubborn");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!gone(pid), "SIGTERM alone should not have killed it");
    for _ in 0..100 {
        if gone(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("SIGKILL never landed");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn shutdown_leaves_no_orphans() {
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("child.pid");
    let command = format!("sleep 60 & echo $! > {}; wait", pidfile.display());
    let (mut d, mut app) = driver(dir.path(), vec![action("a", &command)]);
    d.start(&mut app, "a");
    let child = read_pid(&pidfile);
    d.shutdown();
    for _ in 0..40 {
        if gone(child) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the action's child outlived shutdown");
}

#[tokio::test]
async fn restart_runs_the_action_again_after_it_stops() {
    let dir = tempfile::tempdir().unwrap();
    let (mut d, mut app) = driver(dir.path(), vec![action("slow", "sleep 30")]);
    d.start(&mut app, "slow");
    let first = d.procs["slow"].pid;
    app.run_menu.open = true;
    d.tick(&mut app);
    d.key(&mut app, &mut None, KeyKind::Char('r'));
    until(&mut d, &mut app, "a new process", |d, _| {
        d.procs
            .get("slow")
            .is_some_and(|p| p.pid != first && p.exited().is_none())
    })
    .await;
    d.shutdown();
}

#[tokio::test]
async fn a_listening_port_is_attributed_to_its_action() {
    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (mut d, mut app) = driver(
        dir.path(),
        vec![action(
            "srv",
            "exec python3 -m http.server 0 --bind 127.0.0.1",
        )],
    );
    app.run_menu.open();
    d.start(&mut app, "srv");
    until(&mut d, &mut app, "the port", |_, a| {
        a.run_menu
            .ports
            .iter()
            .any(|p| p.action.as_deref() == Some("srv") && p.host == "loopback")
    })
    .await;
    d.shutdown();
}

#[tokio::test]
async fn ports_are_not_scanned_while_the_overlay_is_closed() {
    let dir = tempfile::tempdir().unwrap();
    let (mut d, mut app) = driver(dir.path(), vec![action("a", "true")]);
    d.tick(&mut app);
    assert!(d.ports_at.is_none() && !app.run_menu.ports_scanned);
}

#[test]
fn a_cwd_that_escapes_the_project_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
    std::fs::create_dir(dir.path().join("web")).unwrap();
    let mut a = action("x", "true");
    a.cwd = Some("link".into());
    assert!(action_cwd(dir.path(), &a).unwrap_err().contains("outside"));
    a.cwd = Some("missing".into());
    assert!(action_cwd(dir.path(), &a)
        .unwrap_err()
        .contains("does not exist"));
    a.cwd = Some("web".into());
    assert!(action_cwd(dir.path(), &a).unwrap().ends_with("web"));
}

#[tokio::test]
async fn a_bad_cwd_notifies_instead_of_starting() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = action("x", "true");
    a.cwd = Some("nope".into());
    let (mut d, mut app) = driver(dir.path(), vec![a]);
    d.start(&mut app, "x");
    assert!(d.procs.is_empty());
    assert!(app
        .run_menu
        .notice
        .as_ref()
        .is_some_and(|n| n.text.contains("does not exist")));
}

#[test]
fn trust_hint_follows_the_config_file() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!declares_actions(dir.path()));
    std::fs::create_dir(dir.path().join(".forge")).unwrap();
    std::fs::write(
        dir.path().join(".forge/config.toml"),
        "[[actions]]\nname = \"dev\"\ncommand = \"true\"\n",
    )
    .unwrap();
    assert!(declares_actions(dir.path()));
}
