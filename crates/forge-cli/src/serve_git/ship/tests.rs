use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use super::super::test_support::*;
use super::*;

struct Fixture {
    _dir: tempfile::TempDir,
    work: PathBuf,
    remote: PathBuf,
    gh_log: PathBuf,
    body_file: PathBuf,
    gh: PathBuf,
    state: Arc<DaemonState>,
    session: String,
}

fn sh(cwd: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

const NO_PRS: &str = "[]";
const OPEN_PR: &str =
    r#"[{"url":"https://github.com/acme/shop/pull/3","isCrossRepository":false}]"#;

/// A repository whose GitHub URL is rewritten to a bare remote, a fake `gh`, and a session whose
/// worktree is on `feat/cart` (or detached / on `main`, per `mode`) with uncommitted edits.
fn fixture(mode: &str, permission: &str, pr_list: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let remote = root.join("remote.git");
    let main = root.join("main");
    std::fs::create_dir_all(&main).unwrap();
    sh(
        &root,
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            remote.to_str().unwrap(),
        ],
    );
    sh(&main, &["init", "-q", "-b", "main"]);
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
        ("remote.origin.url", "https://github.com/acme/shop.git"),
        ("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"),
    ] {
        sh(&main, &["config", key, value]);
    }
    let rewrite = format!("url.{}.insteadOf", remote.display());
    sh(
        &main,
        &["config", &rewrite, "https://github.com/acme/shop.git"],
    );
    std::fs::write(main.join("README.md"), "shop\n").unwrap();
    sh(&main, &["add", "README.md"]);
    sh(&main, &["commit", "-q", "-m", "init"]);
    sh(&main, &["push", "-q", "-u", "origin", "main"]);

    let work = match mode {
        "main" => main.clone(),
        "shared" => {
            sh(&main, &["checkout", "-q", "-b", "feat/cart"]);
            main.clone()
        }
        _ => {
            let wt = root.join("wt");
            sh(
                &main,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    "feat/cart",
                    wt.to_str().unwrap(),
                ],
            );
            if mode == "detached" {
                sh(&wt, &["checkout", "-q", "--detach"]);
            }
            wt
        }
    };
    std::fs::write(work.join("cart.rs"), "fn cart() {}\n").unwrap();
    std::fs::write(work.join("README.md"), "shop\ncart\n").unwrap();
    std::fs::create_dir_all(work.join(".forge")).unwrap();
    std::fs::write(work.join(".forge/state.json"), "{}").unwrap();

    let gh_log = root.join("gh.log");
    let body_file = root.join("body.md");
    let gh = root.join("gh");
    let script = format!(
        r#"#!/bin/sh
echo "$*" >> '{log}'
case "$1 $2" in
  "repo view") echo '{{"viewerPermission":"{permission}","defaultBranchRef":{{"name":"main"}},"isArchived":false}}' ;;
  "pr list") echo '{pr_list}' ;;
  "pr create") cat > '{body}'; echo "https://github.com/acme/shop/pull/9" ;;
  *) echo "unexpected gh call: $*" >&2; exit 1 ;;
esac
"#,
        log = gh_log.display(),
        body = body_file.display(),
    );
    std::fs::write(&gh, script).unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();

    let store = Arc::new(forge_store::Store::open_in_memory().unwrap());
    let session = store
        .create_session(&main.display().to_string(), "default")
        .unwrap();
    if !matches!(mode, "main" | "shared") {
        store
            .set_session_worktree(&session, &work.display().to_string())
            .unwrap();
    }
    let state = state(store, &main);
    Fixture {
        _dir: dir,
        work,
        remote,
        gh_log,
        body_file,
        gh,
        state,
        session,
    }
}

async fn ship(fx: &Fixture, request: ShipRequest) -> ShipResponse {
    ship_session(&fx.state, &fx.session, &request, &fx.gh)
        .await
        .unwrap()
}

fn step_names(response: &ShipResponse) -> Vec<(&'static str, &'static str)> {
    response.steps.iter().map(|s| (s.step, s.status)).collect()
}

fn gh_calls(fx: &Fixture) -> String {
    std::fs::read_to_string(&fx.gh_log).unwrap_or_default()
}

#[tokio::test]
async fn a_finished_worktree_session_ships_to_a_watched_pr() {
    let fx = fixture("worktree", "WRITE", NO_PRS);
    let response = ship(&fx, ShipRequest::default()).await;

    assert!(response.ok, "{:?}", response.refused.map(|r| r.reason));
    assert_eq!(
        step_names(&response),
        [
            ("branch", "done"),
            ("commit", "done"),
            ("push", "done"),
            ("pr", "done"),
            ("watch", "done")
        ]
    );
    let commit = response.commit.unwrap();
    assert_eq!(sh(&fx.work, &["rev-parse", "HEAD"]), commit.sha);
    assert!(sh(&fx.work, &["status", "--porcelain"]).contains(".forge"));
    let files = sh(&fx.work, &["show", "--name-only", "--format=", "HEAD"]);
    assert!(
        files.contains("cart.rs") && files.contains("README.md"),
        "{files}"
    );
    assert!(!files.contains(".forge"), "{files}");
    assert_eq!(
        sh(&fx.remote, &["rev-parse", "refs/heads/feat/cart"]),
        commit.sha
    );

    let calls = gh_calls(&fx);
    assert!(calls.contains("pr create"), "{calls}");
    assert!(calls.contains("--head feat/cart"), "{calls}");
    let pr = response.pr.unwrap();
    assert_eq!((pr.number, pr.existing), (Some(9), false));
    assert!(calls.contains(&format!("--title {}", pr.title)), "{calls}");
    let body = std::fs::read_to_string(&fx.body_file).unwrap();
    assert!(body.contains("### Provenance"), "{body}");
    assert!(body.contains(&fx.session), "{body}");

    let watch = crate::serve_pr_watch::registry().get(&fx.session).unwrap();
    assert_eq!(watch.mode, Mode::Merge);
    assert_eq!(response.watch.unwrap().method, "squash");
}

#[tokio::test]
async fn given_message_and_title_override_the_generated_ones() {
    let fx = fixture("worktree", "WRITE", NO_PRS);
    let response = ship(
        &fx,
        ShipRequest {
            message: Some("feat(cart): add the cart".into()),
            title: Some("Add the cart".into()),
            method: Some("rebase".into()),
            ..ShipRequest::default()
        },
    )
    .await;
    assert!(response.ok);
    assert_eq!(
        sh(&fx.work, &["log", "-1", "--format=%s"]),
        "feat(cart): add the cart"
    );
    assert!(gh_calls(&fx).contains("--title Add the cart"));
    assert_eq!(response.watch.unwrap().method, "rebase");
}

#[tokio::test]
async fn an_existing_pr_is_updated_by_the_push() {
    let fx = fixture("worktree", "WRITE", OPEN_PR);
    let response = ship(&fx, ShipRequest::default()).await;
    assert!(response.ok);
    let pr = response.pr.unwrap();
    assert_eq!((pr.number, pr.existing), (Some(3), true));
    assert!(!gh_calls(&fx).contains("pr create"));
}

#[tokio::test]
async fn a_detached_head_is_refused_before_anything_is_committed() {
    let fx = fixture("detached", "WRITE", NO_PRS);
    let head = sh(&fx.work, &["rev-parse", "HEAD"]);
    let response = ship(&fx, ShipRequest::default()).await;

    assert!(!response.ok);
    let refused = response.refused.unwrap();
    assert_eq!(refused.step, "branch");
    assert!(refused.reason.contains("detached"), "{}", refused.reason);
    assert_eq!(sh(&fx.work, &["rev-parse", "HEAD"]), head);
    assert!(gh_calls(&fx).is_empty());
    assert!(crate::serve_pr_watch::registry().get(&fx.session).is_none());
}

#[tokio::test]
async fn the_base_branch_is_refused_and_its_edits_stay_uncommitted() {
    let fx = fixture("main", "WRITE", NO_PRS);
    let head = sh(&fx.work, &["rev-parse", "HEAD"]);
    let response = ship(&fx, ShipRequest::default()).await;

    let refused = response.refused.unwrap();
    assert_eq!(refused.step, "branch");
    assert!(
        refused.reason.contains("create a branch first"),
        "{}",
        refused.reason
    );
    assert_eq!(sh(&fx.work, &["rev-parse", "HEAD"]), head);
    assert!(sh(&fx.work, &["status", "--porcelain"]).contains("cart.rs"));
    assert!(gh_calls(&fx).is_empty());
}

#[tokio::test]
async fn a_failing_commit_hook_stops_before_any_push() {
    let fx = fixture("worktree", "WRITE", NO_PRS);
    let hooks = sh(&fx.work, &["rev-parse", "--git-common-dir"]);
    let hook = PathBuf::from(&hooks).join("hooks/pre-commit");
    let hook = if hook.is_absolute() {
        hook
    } else {
        fx.work.join(hook)
    };
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\necho 'lint failed' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    let response = ship(&fx, ShipRequest::default()).await;
    let refused = response.refused.unwrap();
    assert_eq!(refused.step, "commit");
    assert!(refused.reason.contains("lint failed"), "{}", refused.reason);
    assert!(!response.steps.iter().any(|s| s.step == "push"));
    assert!(gh_calls(&fx).is_empty());
    assert!(sh(&fx.remote, &["branch", "--list", "feat/cart"]).is_empty());
}

#[tokio::test]
async fn read_only_access_is_refused_at_the_pr_step_without_pushing() {
    let fx = fixture("worktree", "READ", NO_PRS);
    let response = ship(&fx, ShipRequest::default()).await;
    let refused = response.refused.unwrap();
    assert_eq!(refused.step, "pr");
    assert!(
        refused.reason.contains("not write access"),
        "{}",
        refused.reason
    );
    assert!(response.commit.is_some());
    assert!(sh(&fx.remote, &["branch", "--list", "feat/cart"]).is_empty());
    assert!(crate::serve_pr_watch::registry().get(&fx.session).is_none());
}

#[tokio::test]
async fn a_branch_with_nothing_to_ship_is_refused_by_the_ahead_check() {
    let fx = fixture("worktree", "WRITE", NO_PRS);
    sh(&fx.work, &["checkout", "-q", "--", "README.md"]);
    std::fs::remove_file(fx.work.join("cart.rs")).unwrap();
    let response = ship(&fx, ShipRequest::default()).await;
    assert_eq!(step_names(&response)[1], ("commit", "skipped"));
    let refused = response.refused.unwrap();
    assert_eq!(refused.step, "pr");
    assert!(
        refused.reason.contains("no commits ahead"),
        "{}",
        refused.reason
    );
}

#[tokio::test]
async fn a_bad_method_and_an_unknown_session_do_not_start() {
    let fx = fixture("worktree", "WRITE", NO_PRS);
    let bad = ship_session(
        &fx.state,
        &fx.session,
        &ShipRequest {
            method: Some("fast-forward".into()),
            ..ShipRequest::default()
        },
        &fx.gh,
    )
    .await;
    assert_eq!(bad.err().unwrap().0, axum::http::StatusCode::BAD_REQUEST);
    let gone = ship_session(&fx.state, "nope", &ShipRequest::default(), &fx.gh).await;
    assert_eq!(gone.err().unwrap().0, axum::http::StatusCode::NOT_FOUND);
}

#[test]
fn staging_skips_forge_state_and_untracked_residue_but_refuses_conflicts() {
    let dir = repo_with(&[("a.txt", "a\n")]);
    std::fs::write(dir.path().join("a.txt"), "b\n").unwrap();
    std::fs::create_dir_all(dir.path().join(".forge")).unwrap();
    std::fs::write(dir.path().join(".forge/x"), "x").unwrap();
    std::fs::create_dir_all(dir.path().join("__pycache__")).unwrap();
    std::fs::write(dir.path().join("__pycache__/m.pyc"), "x").unwrap();
    std::fs::write(dir.path().join("new.rs"), "n").unwrap();
    let mut paths: Vec<String> = pending_paths(dir.path())
        .unwrap()
        .into_iter()
        .map(|p| p.path)
        .collect();
    paths.sort();
    assert_eq!(paths, ["a.txt", "new.rs"]);
}

#[test]
fn model_replies_split_into_a_title_and_a_body() {
    let (title, body) =
        split_title_body("\n## Title: \"feat(cart): add cart\"\n\n- adds a cart\n- tests it\n")
            .unwrap();
    assert_eq!(title, "feat(cart): add cart");
    assert_eq!(body, "- adds a cart\n- tests it");
    assert!(split_title_body("  \n").is_none());
    let (title, _) = commit_log_text(&["feat: a".into(), "fix: b".into()]);
    assert_eq!(title, "feat: a");
}

fn install_failing_hook(fx: &Fixture) {
    let hooks = sh(&fx.work, &["rev-parse", "--git-common-dir"]);
    let hook = PathBuf::from(&hooks).join("hooks/pre-commit");
    let hook = if hook.is_absolute() {
        hook
    } else {
        fx.work.join(hook)
    };
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\necho 'lint failed' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn edited(store: &forge_store::Store, session: &str, path: &str) {
    let mid = store
        .add_message(session, 0, forge_types::Role::Assistant, "edit", Some("m"))
        .unwrap();
    let args = serde_json::json!({ "path": path, "content": "x" }).to_string();
    store
        .record_tool_call(&mid, "write_file", &args, "ok", "allowed", "ok")
        .unwrap();
}

fn second_session(fx: &Fixture) -> String {
    let other = fx
        .state
        .store
        .create_session(&fx.work.display().to_string(), "default")
        .unwrap();
    fx.state.store.set_session_local_live(&other, true).unwrap();
    fx.state
        .store
        .touch_session_local_presence(&other, false)
        .unwrap();
    other
}

fn committed_files(fx: &Fixture) -> Vec<String> {
    sh(&fx.work, &["show", "--name-only", "--format=", "HEAD"])
        .lines()
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn a_shared_tree_ships_only_the_shipping_sessions_files() {
    let fx = fixture("shared", "WRITE", NO_PRS);
    let other = second_session(&fx);
    edited(&fx.state.store, &fx.session, "cart.rs");
    edited(&fx.state.store, &other, "README.md");
    std::fs::write(fx.work.join("stray.txt"), "by hand\n").unwrap();

    let response = ship(&fx, ShipRequest::default()).await;
    assert!(response.ok, "{:?}", response.refused.map(|r| r.reason));
    assert_eq!(committed_files(&fx), ["cart.rs"]);
    let status = sh(&fx.work, &["status", "--porcelain", "-uall"]);
    assert!(status.contains("M README.md"), "{status}");
    assert!(status.contains("?? stray.txt"), "{status}");
    assert!(sh(&fx.work, &["diff", "--cached", "--name-only"]).is_empty());
    let plan = response.plan.unwrap();
    assert_eq!(plan.source, "session");
    assert!(plan.left_out.contains(&"README.md".to_string()), "{plan:?}");
}

#[tokio::test]
async fn a_file_two_sessions_touched_refuses_the_ship_and_lists_it() {
    let fx = fixture("shared", "WRITE", NO_PRS);
    let other = second_session(&fx);
    edited(&fx.state.store, &fx.session, "cart.rs");
    edited(&fx.state.store, &fx.session, "README.md");
    edited(&fx.state.store, &other, "README.md");
    let head = sh(&fx.work, &["rev-parse", "HEAD"]);

    let response = ship(&fx, ShipRequest::default()).await;
    let refused = response.refused.unwrap();
    assert_eq!(refused.step, "commit");
    assert!(refused.reason.contains("README.md"), "{}", refused.reason);
    assert_eq!(sh(&fx.work, &["rev-parse", "HEAD"]), head);
    assert!(sh(&fx.work, &["diff", "--cached", "--name-only"]).is_empty());
    assert!(gh_calls(&fx).is_empty());
}

#[tokio::test]
async fn a_partially_staged_index_is_committed_as_it_is() {
    let fx = fixture("worktree", "WRITE", NO_PRS);
    sh(&fx.work, &["add", "cart.rs"]);

    let response = ship(&fx, ShipRequest::default()).await;
    assert!(response.ok, "{:?}", response.refused.map(|r| r.reason));
    assert_eq!(response.plan.unwrap().source, "index");
    assert_eq!(committed_files(&fx), ["cart.rs"]);
    assert!(sh(&fx.work, &["status", "--porcelain"]).contains("M README.md"));
}

#[tokio::test]
async fn an_untracked_env_file_is_refused_and_nothing_is_staged() {
    let fx = fixture("worktree", "WRITE", NO_PRS);
    std::fs::write(fx.work.join(".env"), "TOKEN=1\n").unwrap();
    std::fs::write(fx.work.join(".env.example"), "TOKEN=\n").unwrap();
    let head = sh(&fx.work, &["rev-parse", "HEAD"]);

    let response = ship(&fx, ShipRequest::default()).await;
    let refused = response.refused.unwrap();
    assert_eq!(refused.step, "commit");
    assert!(refused.reason.contains(".env"), "{}", refused.reason);
    assert!(
        !refused.reason.contains(".env.example"),
        "{}",
        refused.reason
    );
    assert_eq!(sh(&fx.work, &["rev-parse", "HEAD"]), head);
    assert!(sh(&fx.work, &["diff", "--cached", "--name-only"]).is_empty());
}

#[test]
fn secret_looking_names_are_recognised() {
    for yes in [
        ".env",
        "a/.env.local",
        "k.pem",
        "x/server.key",
        "id_rsa",
        "id_ed25519",
    ] {
        assert!(looks_like_secret(yes), "{yes}");
    }
    for no in [
        ".env.example",
        "id_rsa.pub",
        "env.rs",
        "keyboard.rs",
        "src/main.rs",
    ] {
        assert!(!looks_like_secret(no), "{no}");
    }
}

#[tokio::test]
async fn a_preview_returns_the_commit_set_and_changes_nothing() {
    let fx = fixture("worktree", "WRITE", NO_PRS);
    let head = sh(&fx.work, &["rev-parse", "HEAD"]);
    let status = sh(&fx.work, &["status", "--porcelain", "-uall"]);

    let response = ship(
        &fx,
        ShipRequest {
            preview: true,
            ..ShipRequest::default()
        },
    )
    .await;
    assert!(response.ok && response.preview && response.commit.is_none());
    let plan = response.plan.unwrap();
    let mut files: Vec<_> = plan.files.iter().map(|f| f.path.as_str()).collect();
    files.sort();
    assert_eq!(files, ["README.md", "cart.rs"]);
    assert_eq!(sh(&fx.work, &["rev-parse", "HEAD"]), head);
    assert_eq!(sh(&fx.work, &["status", "--porcelain", "-uall"]), status);
    assert!(sh(&fx.work, &["diff", "--cached", "--name-only"]).is_empty());
    assert!(gh_calls(&fx).is_empty());
    assert!(crate::serve_pr_watch::registry().get(&fx.session).is_none());
}

#[tokio::test]
async fn a_terminal_session_that_is_mid_turn_is_refused() {
    let fx = fixture("worktree", "WRITE", NO_PRS);
    fx.state
        .store
        .set_session_local_live(&fx.session, true)
        .unwrap();
    fx.state
        .store
        .touch_session_local_presence(&fx.session, true)
        .unwrap();
    let head = sh(&fx.work, &["rev-parse", "HEAD"]);

    let response = ship(&fx, ShipRequest::default()).await;
    let refused = response.refused.unwrap();
    assert_eq!(refused.step, "branch");
    assert!(refused.reason.contains("turn"), "{}", refused.reason);
    assert_eq!(sh(&fx.work, &["rev-parse", "HEAD"]), head);
    assert!(sh(&fx.work, &["diff", "--cached", "--name-only"]).is_empty());

    fx.state
        .store
        .touch_session_local_presence(&fx.session, false)
        .unwrap();
    assert!(ship(&fx, ShipRequest::default()).await.ok);
}

#[tokio::test]
async fn a_refused_commit_hook_leaves_the_index_as_it_was() {
    let fx = fixture("worktree", "WRITE", NO_PRS);
    install_failing_hook(&fx);

    let response = ship(&fx, ShipRequest::default()).await;
    assert!(response.refused.unwrap().reason.contains("lint failed"));
    assert!(
        sh(&fx.work, &["diff", "--cached", "--name-only"]).is_empty(),
        "nothing was staged before, nothing may be after"
    );
    assert!(sh(&fx.work, &["status", "--porcelain", "-uall"]).contains("?? cart.rs"));

    sh(&fx.work, &["add", "README.md"]);
    let response = ship(&fx, ShipRequest::default()).await;
    assert!(response.refused.is_some());
    assert_eq!(
        sh(&fx.work, &["diff", "--cached", "--name-only"]),
        "README.md"
    );
}

#[tokio::test]
async fn a_failing_local_ci_plan_refuses_the_push_and_names_the_step() {
    let fx = fixture("worktree", "WRITE", NO_PRS);
    let wf = fx.work.join(".github/workflows");
    std::fs::create_dir_all(&wf).unwrap();
    std::fs::write(
        wf.join("ci.yml"),
        "on: pull_request\nenv:\n  STRICT: \"1\"\njobs:\n  check:\n    runs-on: ubuntu-latest\n    steps:\n      - name: Strict lint\n        run: echo lint-broke >&2; test \"$STRICT\" = 0\n",
    )
    .unwrap();
    let response = ship(&fx, ShipRequest::default()).await;

    assert!(!response.ok);
    let refused = response.refused.unwrap();
    assert_eq!(refused.step, "push");
    assert!(
        refused.reason.contains("Local CI step failed: Strict lint"),
        "{}",
        refused.reason
    );
    assert!(
        refused.reason.contains("STRICT=1") && refused.reason.contains("lint-broke"),
        "{}",
        refused.reason
    );
    assert!(sh(&fx.remote, &["branch", "--list", "feat/cart"]).is_empty());
    assert!(gh_calls(&fx).is_empty());
}
