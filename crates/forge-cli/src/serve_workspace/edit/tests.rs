use super::*;

fn root() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    (dir, root)
}

#[test]
fn create_makes_files_and_folders_and_missing_parents() {
    let (_guard, root) = root();
    let file = create_entry(&root, "src/deep/new.ts", None).unwrap();
    assert_eq!((file.path.as_str(), file.kind), ("src/deep/new.ts", "file"));
    assert!(root.join("src/deep/new.ts").is_file());
    let folder = create_entry(&root, "docs", Some("directory")).unwrap();
    assert_eq!(folder.kind, "directory");
    assert!(root.join("docs").is_dir());
}

#[test]
fn create_never_overwrites() {
    let (_guard, root) = root();
    fs::write(root.join("a.txt"), "keep").unwrap();
    let error = create_entry(&root, "a.txt", None).unwrap_err();
    assert_eq!(error.status, StatusCode::CONFLICT);
    assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "keep");
    assert_eq!(
        create_entry(&root, "a.txt", Some("directory"))
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
}

#[test]
fn paths_outside_the_root_or_inside_git_are_refused() {
    let (_guard, root) = root();
    for bad in ["../x", "/etc/x", ".git/hooks/pre-commit", "a/../../x", ""] {
        assert!(create_entry(&root, bad, None).is_err(), "{bad:?}");
    }
    assert!(create_entry(&root, "x", Some("symlink")).is_err());
}

#[test]
fn rename_moves_and_refuses_to_clobber_or_nest() {
    let (_guard, root) = root();
    fs::write(root.join("a.txt"), "a").unwrap();
    fs::write(root.join("b.txt"), "b").unwrap();
    fs::create_dir(root.join("dir")).unwrap();
    assert_eq!(
        rename_entry(&root, "a.txt", "b.txt").unwrap_err().status,
        StatusCode::CONFLICT
    );
    assert_eq!(fs::read_to_string(root.join("b.txt")).unwrap(), "b");
    let moved = rename_entry(&root, "a.txt", "dir/sub/c.txt").unwrap();
    assert_eq!(moved.path, "dir/sub/c.txt");
    assert!(!root.join("a.txt").exists());
    assert_eq!(fs::read_to_string(root.join("dir/sub/c.txt")).unwrap(), "a");
    assert!(rename_entry(&root, "dir", "dir/inner").is_err());
    assert!(rename_entry(&root, "dir", "dir").is_err());
    assert_eq!(
        rename_entry(&root, "missing", "x").unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    assert!(rename_entry(&root, "b.txt", "../escaped.txt").is_err());
    assert!(rename_entry(&root, "b.txt", ".git/config").is_err());
}

#[test]
fn delete_removes_files_and_folders_but_not_nested_repositories() {
    let (_guard, root) = root();
    fs::create_dir_all(root.join("tree/inner")).unwrap();
    fs::write(root.join("tree/inner/f.txt"), "x").unwrap();
    fs::write(root.join("solo.txt"), "x").unwrap();
    assert_eq!(delete_entry(&root, "solo.txt").unwrap().kind, "file");
    assert!(!root.join("solo.txt").exists());
    assert_eq!(delete_entry(&root, "tree").unwrap().kind, "directory");
    assert!(!root.join("tree").exists());
    fs::create_dir_all(root.join("vendor/lib/.git")).unwrap();
    assert_eq!(
        delete_entry(&root, "vendor/lib").unwrap_err().status,
        StatusCode::FORBIDDEN
    );
    assert!(root.join("vendor/lib/.git").exists());
    assert!(delete_entry(&root, "../x").is_err());
    assert!(delete_entry(&root, "").is_err());
    assert_eq!(
        delete_entry(&root, "ghost").unwrap_err().status,
        StatusCode::NOT_FOUND
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_directory_cannot_carry_an_operation_outside_the_root() {
    use std::os::unix::fs::symlink;

    let (_guard, root) = root();
    let (_outside_guard, outside) = self::root();
    fs::write(outside.join("secret.txt"), "secret").unwrap();
    symlink(&outside, root.join("link")).unwrap();
    assert_eq!(
        create_entry(&root, "link/new.txt", None)
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
    assert!(!outside.join("new.txt").exists());
    assert_eq!(
        delete_entry(&root, "link/secret.txt").unwrap_err().status,
        StatusCode::FORBIDDEN
    );
    assert!(outside.join("secret.txt").exists());
    fs::write(root.join("mine.txt"), "m").unwrap();
    assert!(rename_entry(&root, "mine.txt", "link/stolen.txt").is_err());
    assert!(root.join("mine.txt").exists());
    // The link itself is the workspace's own entry: deleting it removes the link only.
    assert_eq!(delete_entry(&root, "link").unwrap().kind, "file");
    assert!(outside.join("secret.txt").exists());
}
