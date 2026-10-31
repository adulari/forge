use super::*;

fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "git {args:?}");
}

#[test]
fn a_subfolder_of_a_repository_is_a_duplicate_of_its_top() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("app");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    git(&repo, &["init", "-q"]);
    let other = dir.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    let paths = vec![
        repo.display().to_string(),
        other.display().to_string(),
        repo.join("src").display().to_string(),
    ];
    let groups = duplicate_groups(&paths);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].0, paths[0]);
    assert_eq!(groups[0].1, vec![paths[2].clone()]);
}

#[test]
fn distinct_projects_are_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    assert!(duplicate_groups(&[a.display().to_string(), b.display().to_string()]).is_empty());
}

#[test]
fn the_repository_top_is_kept_even_when_a_subfolder_is_more_recent() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir
        .path()
        .join("app")
        .canonicalize()
        .unwrap_or_else(|_| dir.path().join("app"));
    std::fs::create_dir_all(repo.join("crates/x")).unwrap();
    git(&repo, &["init", "-q"]);
    let paths = vec![
        repo.join("crates/x").display().to_string(),
        repo.display().to_string(),
    ];
    let groups = duplicate_groups(&paths);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].0, paths[1]);
    assert_eq!(groups[0].1, vec![paths[0].clone()]);
}
