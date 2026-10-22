//! Where Forge keeps its runtime state for a workspace: undo checkpoints, the evidence ledger and
//! background-job records.
//!
//! None of it belongs in the user's tree. A project directory is the user's: a strict tree check,
//! a `docker COPY`, `rsync` or `grep -r` all see a stray `.forge/checkpoints/` as part of the
//! project, and a benchmark that diffs the tree against an allow-list fails on it. So the state
//! lives under Forge's own data directory, one folder per workspace, and `<workspace>/.forge/`
//! stays reserved for what the project authors (`AGENTS.md`, `mcp.toml`, skills, commands).
//!
//! Root, first match wins:
//! 1. `FORGE_STATE_DIR`, an explicit override (the bridge also hands it to `forge mcp-serve`, so
//!    parent and child never disagree about where a session's files are);
//! 2. `<FORGE_DB>.workspaces`, beside a scratch or operator-selected store, so an isolated run
//!    (tests, benchmarks) never writes into the real data directory;
//! 3. `<data dir>/workspaces`;
//! 4. `<temp dir>/forge-workspaces` when no home directory resolves.
//!
//! State written by earlier versions lives in `<workspace>/.forge/<kind>/`. [`adopt_legacy_state`]
//! moves it across so `/undo` and `/rewind` keep working for sessions that predate the move.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once, OnceLock};
use std::time::{Duration, SystemTime};

/// Env var overriding the root that holds every workspace's state folder.
pub const STATE_DIR_ENV: &str = "FORGE_STATE_DIR";

/// File in a workspace's state folder naming the workspace it belongs to, so the folder of a
/// workspace that no longer exists can be told apart and eventually removed.
const MARKER: &str = "workspace";

/// How long a state folder whose workspace is gone is kept. Generous on purpose: a workspace on an
/// unmounted drive looks gone too, and its checkpoints are worth keeping until it comes back.
const ORPHAN_GRACE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The folder that holds one sub-folder per workspace.
pub fn workspace_state_root() -> PathBuf {
    if let Some(dir) = env_path(STATE_DIR_ENV) {
        return dir;
    }
    if let Some(db) = env_path("FORGE_DB") {
        // Appended, not substituted for the extension: it then sits in the same family as the
        // store's `-wal` and `-shm` files, and a `<store>*` ignore pattern covers it too.
        let mut beside = db.into_os_string();
        beside.push(".workspaces");
        return PathBuf::from(beside);
    }
    match crate::data_dir() {
        Some(data) => data.join("workspaces"),
        None => std::env::temp_dir().join("forge-workspaces"),
    }
}

/// Stable (process- and release-independent) FNV-1a 64-bit hash. Names a folder, not a secret.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The workspace as a path that is the same however it was spelled: symlinks resolved, relative
/// paths made absolute. A path that does not exist (yet) is only made absolute.
fn canonical(workspace: &Path) -> PathBuf {
    std::fs::canonicalize(workspace)
        .or_else(|_| std::path::absolute(workspace))
        .unwrap_or_else(|_| workspace.to_path_buf())
}

/// This workspace's state folder: `<root>/<name>-<hash>`. Pure: nothing is created.
///
/// The folder name is the workspace's base name (so a person browsing the data directory can tell
/// folders apart) plus a hash of its canonical path (so two checkouts called `app` never share).
pub fn workspace_state_dir(workspace: &Path) -> PathBuf {
    let path = canonical(workspace);
    let name: String = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    let name = name.trim_matches('.');
    let name = if name.is_empty() { "root" } else { name };
    let hash = fnv1a(path.to_string_lossy().as_bytes());
    workspace_state_root().join(format!("{name}-{hash:016x}"))
}

/// A kind of state (`checkpoints`, `evidence`, `jobs`) in this workspace's state folder. Pure.
pub fn state_subdir(workspace: &Path, kind: &str) -> PathBuf {
    workspace_state_dir(workspace).join(kind)
}

/// Where earlier versions kept `kind`: inside the workspace, under `.forge/`.
pub fn legacy_state_subdir(workspace: &Path, kind: &str) -> PathBuf {
    workspace.join(".forge").join(kind)
}

/// Create this workspace's state folder and record which workspace it is for. Idempotent. The
/// first call in a process also sweeps the folders of workspaces that are long gone.
pub fn prepare_workspace_state(workspace: &Path) -> std::io::Result<PathBuf> {
    static SWEEP: Once = Once::new();
    let dir = workspace_state_dir(workspace);
    std::fs::create_dir_all(&dir)?;
    let marker = dir.join(MARKER);
    if !marker.exists() {
        std::fs::write(&marker, canonical(workspace).to_string_lossy().as_bytes())?;
    }
    // Off the caller's path: it reads one marker per state folder, and session start is not the
    // place to wait for housekeeping.
    SWEEP.call_once(|| {
        let root = workspace_state_root();
        let _ = std::thread::Builder::new()
            .name("forge-state-sweep".into())
            .spawn(move || sweep_orphans(&root, SystemTime::now()));
    });
    Ok(dir)
}

/// Remove state folders whose workspace is gone and that nothing has touched for [`ORPHAN_GRACE`].
/// Best effort: a folder that cannot be read or removed is left alone.
fn sweep_orphans(root: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let Ok(workspace) = std::fs::read_to_string(dir.join(MARKER)) else {
            continue;
        };
        if Path::new(workspace.trim()).exists() {
            continue;
        }
        let idle = last_touched(&dir).and_then(|touched| now.duration_since(touched).ok());
        if idle.is_some_and(|idle| idle >= ORPHAN_GRACE) {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// The newest modification time of a state folder or of anything directly in it (its `checkpoints`,
/// `evidence` and `jobs` folders change whenever a session adds to them).
fn last_touched(dir: &Path) -> Option<SystemTime> {
    let modified = |path: &Path| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
    };
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| modified(&entry.path()))
        .chain(modified(dir))
        .max()
}

/// Move `kind` state that an earlier version left in `<workspace>/.forge/<kind>/` into this
/// workspace's state folder. Entries already present at the destination win and the stale copy is
/// left in place. Once the old folder is empty (apart from the `*` ignore file Forge wrote there)
/// it is removed, and so is `.forge/` if nothing else is in it. Returns how many entries moved.
///
/// Best effort and cheap: one `stat` per workspace and kind per process, and nothing at all for a
/// workspace that never had legacy state. Callers that must stay correct without it (it can fail
/// on a read-only workspace) read the destination only.
pub fn adopt_legacy_state(workspace: &Path, kind: &str) -> usize {
    static DONE: OnceLock<Mutex<HashSet<(PathBuf, String)>>> = OnceLock::new();
    let legacy = legacy_state_subdir(workspace, kind);
    let key = (canonical(workspace), kind.to_string());
    if let Ok(mut done) = DONE.get_or_init(Default::default).lock() {
        if !done.insert(key) {
            return 0;
        }
    }
    if !legacy.is_dir() {
        return 0;
    }
    let Ok(state) = prepare_workspace_state(workspace) else {
        return 0;
    };
    let destination = state.join(kind);
    if std::fs::create_dir_all(&destination).is_err() {
        return 0;
    }
    adopt_dir(&legacy, &destination)
}

fn adopt_dir(legacy: &Path, destination: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(legacy) else {
        return 0;
    };
    let mut moved = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let target = destination.join(&name);
        if name == ".gitignore" || target.exists() {
            continue;
        }
        if move_entry(&entry.path(), &target).is_ok() {
            moved += 1;
        }
    }
    remove_if_only_ignore_file(legacy);
    if let Some(forge_dir) = legacy.parent() {
        // Succeeds only when empty: a project's own `.forge/` content is never touched.
        let _ = std::fs::remove_dir(forge_dir);
    }
    moved
}

/// Rename, or copy then delete when the two folders are on different file systems.
fn move_entry(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_recursive(from, to)?;
    if from.is_dir() {
        std::fs::remove_dir_all(from)
    } else {
        std::fs::remove_file(from)
    }
}

fn copy_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

fn remove_if_only_ignore_file(dir: &Path) {
    let ignore = dir.join(".gitignore");
    let only_ours = std::fs::read_to_string(&ignore).is_ok_and(|text| text == "*\n");
    let others = std::fs::read_dir(dir)
        .map(|entries| entries.flatten().any(|e| e.file_name() != ".gitignore"))
        .unwrap_or(true);
    if only_ours && !others {
        let _ = std::fs::remove_file(ignore);
    }
    let _ = std::fs::remove_dir(dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests share process env, so the ones that set it serialize and restore it.
    fn with_state_root<T>(root: &Path, body: impl FnOnce() -> T) -> T {
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os(STATE_DIR_ENV);
        std::env::set_var(STATE_DIR_ENV, root);
        let out = body();
        match previous {
            Some(value) => std::env::set_var(STATE_DIR_ENV, value),
            None => std::env::remove_var(STATE_DIR_ENV),
        }
        out
    }

    #[test]
    fn state_folder_is_outside_the_workspace_and_distinct_per_workspace() {
        let state = tempfile::tempdir().unwrap();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        with_state_root(state.path(), || {
            let dir_a = workspace_state_dir(a.path());
            assert!(dir_a.starts_with(state.path()));
            assert!(!dir_a.starts_with(a.path()));
            assert_ne!(dir_a, workspace_state_dir(b.path()));
            assert_eq!(dir_a, workspace_state_dir(a.path()), "stable");
        });
    }

    #[test]
    fn spelling_of_the_workspace_path_does_not_change_the_folder() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        std::fs::create_dir(ws.path().join("sub")).unwrap();
        let roundabout = ws.path().join("sub").join("..");
        with_state_root(state.path(), || {
            assert_eq!(
                workspace_state_dir(ws.path()),
                workspace_state_dir(&roundabout)
            );
        });
    }

    #[test]
    fn folder_name_is_safe_for_awkward_workspace_names() {
        let state = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        let awkward = parent.path().join("my app (v2)/ü");
        std::fs::create_dir_all(&awkward).unwrap();
        with_state_root(state.path(), || {
            let dir = workspace_state_dir(&awkward);
            let name = dir.file_name().unwrap().to_string_lossy().into_owned();
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
                "{name}"
            );
            assert!(dir.parent() == Some(state.path()));
        });
    }

    #[test]
    fn a_scratch_store_gets_its_own_state_root() {
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = (
            std::env::var_os(STATE_DIR_ENV),
            std::env::var_os("FORGE_DB"),
        );
        std::env::remove_var(STATE_DIR_ENV);
        std::env::set_var("FORGE_DB", "/scratch/run-1/forge-bench.db");
        let root = workspace_state_root();
        std::env::set_var(STATE_DIR_ENV, "/explicit");
        let explicit = workspace_state_root();
        for (name, value) in [(STATE_DIR_ENV, saved.0), ("FORGE_DB", saved.1)] {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        assert_eq!(
            root,
            PathBuf::from("/scratch/run-1/forge-bench.db.workspaces")
        );
        assert_eq!(explicit, PathBuf::from("/explicit"), "explicit wins");
    }

    #[test]
    fn legacy_state_moves_into_the_state_folder_and_the_workspace_is_left_clean() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let legacy = ws.path().join(".forge/checkpoints");
        std::fs::create_dir_all(legacy.join("sess1/3")).unwrap();
        std::fs::write(legacy.join("sess1/3/manifest.json"), "{}").unwrap();
        std::fs::write(legacy.join(".gitignore"), "*\n").unwrap();
        with_state_root(state.path(), || {
            assert_eq!(adopt_legacy_state(ws.path(), "checkpoints"), 1);
            let moved = state_subdir(ws.path(), "checkpoints").join("sess1/3/manifest.json");
            assert_eq!(std::fs::read_to_string(moved).unwrap(), "{}");
        });
        assert!(!ws.path().join(".forge").exists(), "nothing left behind");
    }

    #[test]
    fn adopting_never_removes_what_the_project_keeps_in_dot_forge() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(ws.path().join(".forge/evidence/s")).unwrap();
        std::fs::write(ws.path().join(".forge/evidence/s/0001.txt"), "x").unwrap();
        std::fs::write(ws.path().join(".forge/AGENTS.md"), "rules").unwrap();
        with_state_root(state.path(), || {
            assert_eq!(adopt_legacy_state(ws.path(), "evidence"), 1);
        });
        assert!(!ws.path().join(".forge/evidence").exists());
        assert_eq!(
            std::fs::read_to_string(ws.path().join(".forge/AGENTS.md")).unwrap(),
            "rules"
        );
    }

    #[test]
    fn an_existing_destination_entry_wins_over_the_legacy_copy() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(ws.path().join(".forge/checkpoints/s")).unwrap();
        std::fs::write(ws.path().join(".forge/checkpoints/s/old"), "old").unwrap();
        with_state_root(state.path(), || {
            let dest = state_subdir(ws.path(), "checkpoints").join("s");
            std::fs::create_dir_all(&dest).unwrap();
            std::fs::write(dest.join("new"), "new").unwrap();
            assert_eq!(adopt_legacy_state(ws.path(), "checkpoints"), 0);
            assert!(dest.join("new").exists() && !dest.join("old").exists());
        });
        assert!(ws.path().join(".forge/checkpoints/s/old").exists());
    }

    #[test]
    fn a_workspace_without_legacy_state_costs_nothing_and_creates_nothing() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        with_state_root(state.path(), || {
            assert_eq!(adopt_legacy_state(ws.path(), "jobs"), 0);
            assert!(!workspace_state_dir(ws.path()).exists());
        });
    }

    #[test]
    fn sweep_removes_only_long_abandoned_state_of_vanished_workspaces() {
        let root = tempfile::tempdir().unwrap();
        let live_ws = tempfile::tempdir().unwrap();
        let make = |name: &str, workspace: &Path| {
            let dir = root.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(MARKER), workspace.to_string_lossy().as_bytes()).unwrap();
            dir
        };
        let gone = make("gone", Path::new("/definitely/not/here"));
        let live = make("live", live_ws.path());
        let unmarked = root.path().join("unmarked");
        std::fs::create_dir_all(&unmarked).unwrap();

        sweep_orphans(root.path(), SystemTime::now());
        assert!(
            gone.exists(),
            "a recently used folder is kept even if its workspace is gone"
        );

        sweep_orphans(root.path(), SystemTime::now() + ORPHAN_GRACE * 2);
        assert!(
            !gone.exists(),
            "abandoned state of a vanished workspace is swept"
        );
        assert!(
            live.exists(),
            "a workspace that still exists keeps its state"
        );
        assert!(
            unmarked.exists(),
            "a folder Forge did not mark is never touched"
        );
    }
}
