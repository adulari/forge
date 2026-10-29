use super::status::status_response;
use super::*;

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .unwrap()
}

fn git_ok(dir: &Path, args: &[&str]) {
    let out = git(dir, args);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn status_of(dir: &Path) -> GitStatusResponse {
    status_response(dir.to_str().unwrap(), None).unwrap()
}

fn paths(rows: &[GitFileRow]) -> Vec<&str> {
    rows.iter().map(|row| row.path.as_str()).collect()
}

#[test]
fn a_conflicted_merge_lists_the_file_once_and_names_the_merge() {
    let dir = tempfile::tempdir().unwrap();
    git_ok(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("a.txt"), "base\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "other\n").unwrap();
    git_ok(dir.path(), &["add", "."]);
    git_ok(dir.path(), &["commit", "-qm", "base"]);
    git_ok(dir.path(), &["switch", "-qc", "topic"]);
    std::fs::write(dir.path().join("a.txt"), "topic\n").unwrap();
    git_ok(dir.path(), &["commit", "-qam", "topic"]);
    git_ok(dir.path(), &["switch", "-q", "main"]);
    std::fs::write(dir.path().join("a.txt"), "main\n").unwrap();
    git_ok(dir.path(), &["commit", "-qam", "main"]);
    assert!(!git(dir.path(), &["merge", "topic"]).status.success());
    std::fs::write(dir.path().join("b.txt"), "edited later\n").unwrap();

    let status = status_of(dir.path());
    assert_eq!(paths(&status.conflicted), ["a.txt"]);
    assert_eq!(status.conflicted[0].status, "U");
    assert!(!paths(&status.staged).contains(&"a.txt"));
    assert!(!paths(&status.unstaged).contains(&"a.txt"));
    assert_eq!(paths(&status.unstaged), ["b.txt"]);
    assert_eq!(status.operation.as_deref(), Some("merge"));

    std::fs::write(dir.path().join("a.txt"), "resolved\n").unwrap();
    git_ok(dir.path(), &["add", "a.txt"]);
    let resolved = status_of(dir.path());
    assert!(resolved.conflicted.is_empty());
    assert_eq!(paths(&resolved.staged), ["a.txt"]);
    assert_eq!(resolved.operation.as_deref(), Some("merge"));
}

#[test]
fn a_clean_repository_has_no_operation_conflicts_or_upstream() {
    let dir = tempfile::tempdir().unwrap();
    git_ok(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("a.txt"), "x\n").unwrap();
    git_ok(dir.path(), &["add", "."]);
    git_ok(dir.path(), &["commit", "-qm", "init"]);
    let status = status_of(dir.path());
    assert!(status.conflicted.is_empty());
    assert_eq!(status.operation, None);
    assert_eq!(status.upstream, None);
    assert_eq!((status.ahead, status.behind), (0, 0));
    assert!(!status.has_remote);
}

#[test]
fn ahead_and_behind_count_against_the_upstream() {
    let remote = tempfile::tempdir().unwrap();
    git_ok(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
    let url = remote.path().to_str().unwrap();
    let work = tempfile::tempdir().unwrap();
    git_ok(work.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(work.path().join("a.txt"), "1\n").unwrap();
    git_ok(work.path(), &["add", "."]);
    git_ok(work.path(), &["commit", "-qm", "one"]);
    git_ok(work.path(), &["remote", "add", "origin", url]);
    git_ok(work.path(), &["push", "-q", "-u", "origin", "main"]);

    let other = tempfile::tempdir().unwrap();
    git_ok(other.path(), &["clone", "-q", url, "."]);
    std::fs::write(other.path().join("b.txt"), "remote\n").unwrap();
    git_ok(other.path(), &["add", "."]);
    git_ok(other.path(), &["commit", "-qm", "remote work"]);
    git_ok(other.path(), &["push", "-q", "origin", "main"]);

    std::fs::write(work.path().join("a.txt"), "2\n").unwrap();
    git_ok(work.path(), &["commit", "-qam", "local work"]);
    git_ok(work.path(), &["fetch", "-q"]);

    let status = status_of(work.path());
    assert_eq!(status.upstream.as_deref(), Some("origin/main"));
    assert_eq!((status.ahead, status.behind), (1, 1));
    assert!(status.has_remote);
}
