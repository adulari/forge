//! Unit tests that start sessions or restore test edits resolve Forge's per-workspace state
//! folder; this points it at a throwaway directory once per process, so a test run never writes
//! into the real data directory.

pub(crate) fn isolate() {
    static STATE: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    STATE.get_or_init(|| {
        let dir = tempfile::tempdir().expect("state dir");
        std::env::set_var(forge_config::STATE_DIR_ENV, dir.path());
        dir
    });
}

/// The guard restored the originals without touching the repo's stash or refs, and kept the
/// model's edits as a patch in Forge's state folder for the workspace.
pub(crate) fn assert_test_edits_kept_as_patch(dir: &std::path::Path) {
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert!(git(&["stash", "list"]).is_empty(), "no stash entry");
    assert!(!git(&["for-each-ref"]).contains("refs/stash"));
    let patches = forge_config::state_subdir(dir, "test-edits");
    let kept = std::fs::read_dir(&patches).map_or(0, |entries| entries.flatten().count());
    assert!(
        kept > 0,
        "the test edits must be kept as a patch in {patches:?}"
    );
}
