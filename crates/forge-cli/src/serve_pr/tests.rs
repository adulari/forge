use std::os::unix::fs::PermissionsExt;

use super::*;

fn fake_gh(dir: &Path, script: &str) -> Gh {
    let path = dir.join("gh");
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    Gh::with_program(path)
}

#[test]
fn checks_roll_up_runs_and_statuses() {
    let rollup = json!([
        {"name": "build", "status": "COMPLETED", "conclusion": "SUCCESS", "detailsUrl": "u1", "workflowName": "CI"},
        {"name": "lint", "status": "COMPLETED", "conclusion": "FAILURE"},
        {"name": "e2e", "status": "IN_PROGRESS", "conclusion": ""},
        {"name": "docs", "status": "COMPLETED", "conclusion": "SKIPPED"},
        {"context": "ci/legacy", "state": "PENDING", "targetUrl": "u2"},
        {"context": "ci/other", "state": "ERROR"}
    ]);
    let summary = summarize_checks(&rollup);
    assert_eq!(
        (
            summary.total,
            summary.passing,
            summary.failing,
            summary.pending
        ),
        (6, 2, 2, 2)
    );
    assert_eq!(summary.items[2].detail, "in progress");
    assert_eq!(summary.items[0].workflow.as_deref(), Some("CI"));
    assert_eq!(summarize_checks(&Value::Null).total, 0);
}

#[test]
fn whitespace_only_rewrites_fold_to_context() {
    let patch = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,3 +1,3 @@\n keep\n-fn  a() {}\n+    fn a() {}\n-real\n+changed\n tail\n";
    let folded = fold_whitespace_changes(patch);
    assert!(folded.contains("\n     fn a() {}\n"));
    assert!(folded.contains("-real\n+changed"));
    let files = parse_patch(&folded, true);
    assert_eq!(files.len(), 1);

    let only_ws =
        "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-x  = 1\n+x = 1\n";
    assert!(parse_patch(&fold_whitespace_changes(only_ws), true).is_empty());
    assert_eq!(parse_patch(only_ws, false).len(), 1);
}

#[test]
fn sha_and_repo_helpers() {
    assert!(valid_sha("abc1234"));
    assert!(!valid_sha("abc12"));
    assert!(!valid_sha("--force-with-lease"));
    assert_eq!(
        repo_of_url("https://github.com/acme/widgets/pull/4"),
        "acme/widgets"
    );
}

#[test]
fn gh_errors_classify() {
    assert_eq!(
        GhError::classify("no pull requests found for branch \"x\"".into()),
        GhError::NoPr
    );
    assert!(matches!(
        GhError::classify("To get started with GitHub CLI, please run:  gh auth login".into()),
        GhError::SignedOut(_)
    ));
    assert!(matches!(
        GhError::classify("boom".into()),
        GhError::Failed(_)
    ));
}

#[tokio::test]
async fn status_reads_the_branch_pr_and_reports_absence() {
    let dir = tempfile::tempdir().unwrap();
    let with_pr = fake_gh(
        dir.path(),
        r#"echo '{"number":7,"url":"https://github.com/a/b/pull/7","title":"T","state":"OPEN","isDraft":false,"headRefName":"f","baseRefName":"main","headRefOid":"abcdef1234","mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","reviewDecision":"","statusCheckRollup":[{"name":"build","status":"COMPLETED","conclusion":"SUCCESS"}],"additions":3,"deletions":1,"changedFiles":2}'"#,
    );
    let status = status_in(&with_pr, dir.path()).await;
    let pr = status.pr.unwrap();
    assert_eq!(
        (pr.number, pr.checks.passing, pr.head_sha.as_str()),
        (7, 1, "abcdef1234")
    );

    let none = fake_gh(
        dir.path(),
        "echo 'no pull requests found for branch \"f\"' >&2; exit 1",
    );
    let status = status_in(&none, dir.path()).await;
    assert!(status.available && status.pr.is_none() && status.error.is_none());

    let out = fake_gh(
        dir.path(),
        "echo 'To get started, run: gh auth login' >&2; exit 4",
    );
    let status = status_in(&out, dir.path()).await;
    assert!(!status.available && status.error.is_some());

    let absent = Gh::with_program(dir.path().join("missing-gh"));
    assert!(!status_in(&absent, dir.path()).await.available);
}

#[tokio::test]
async fn merge_requires_the_seen_sha_and_passes_it_to_gh() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("args.log");
    let gh = fake_gh(
        dir.path(),
        &format!(
            "echo \"$@\" >> {}\nif [ \"$2\" = view ]; then echo '{{\"baseRefName\":\"main\"}}'; fi",
            log.display()
        ),
    );
    let request = |sha: &str, method: Option<&str>| MergeRequest {
        session: "s".into(),
        number: 9,
        head_sha: sha.into(),
        method: method.map(str::to_string),
        delete_branch: true,
    };
    assert!(merge_in(&gh, dir.path(), &request("", None)).await.is_err());
    assert!(
        merge_in(&gh, dir.path(), &request("abcdef1", Some("octopus")))
            .await
            .is_err()
    );
    let response = merge_in(&gh, dir.path(), &request("abcdef1", Some("rebase")))
        .await
        .unwrap();
    assert_eq!(response.base, "main");
    let logged = std::fs::read_to_string(&log).unwrap();
    assert!(logged.contains("pr merge 9 --rebase --match-head-commit abcdef1 --delete-branch"));
}

#[tokio::test]
async fn diff_scopes_to_a_commit_and_rejects_non_hex() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(
        dir.path(),
        "printf 'diff --git a/x b/x\\n--- a/x\\n+++ b/x\\n@@ -1 +1 @@\\n-a\\n+b\\n'",
    );
    let value = diff_in(&gh, dir.path(), 3, Some("abcdef1"), false)
        .await
        .unwrap();
    assert_eq!(value["files"].as_array().unwrap().len(), 1);
    assert!(diff_in(&gh, dir.path(), 3, Some("../x"), false)
        .await
        .is_err());
}

#[tokio::test]
async fn comments_are_sent_on_stdin_and_empty_ones_refused() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("body.txt");
    let gh = fake_gh(dir.path(), &format!("cat > {}", out.display()));
    assert!(comment_in(&gh, dir.path(), 1, "  ").await.is_err());
    comment_in(&gh, dir.path(), 1, "looks good").await.unwrap();
    assert_eq!(std::fs::read_to_string(out).unwrap(), "looks good");
}
