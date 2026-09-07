//! Background file watcher: reindex supported source files as they change on disk (external
//! editor edits), so retrieval stays fresh without a manual `forge lattice update`. Coalesces save
//! bursts; watches only the directories the indexer itself walks (no `target/`, `.git/`,
//! `node_modules/`, worktrees). A watcher must never crash the session, so once running, per-file
//! reindex errors are swallowed.
//!
//! Two properties keep this cheap, and both were learned the hard way:
//!
//! * **Reads are not changes.** notify's inotify backend subscribes to `IN_OPEN`, so every file
//!   the reindexer reads to hash it produced an event for that same file, which queued another
//!   reindex, which read the file again — a self-sustaining loop that pinned one core per Forge
//!   process for as long as the process lived (the initial `update()` walk was enough to seed it).
//!   The handler therefore drops every access-only event before it reaches the worker.
//! * **Watch what you index.** A blanket recursive watch on the project root registered ~55,000
//!   inotify watches on this repository (`target/`, `node_modules/`, every worktree), so every
//!   cargo build or checkout in a sibling worktree woke the watcher thread for nothing. The watch
//!   set is now exactly the directory set the indexer walks, kept current as directories appear.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::thread::JoinHandle;
use std::time::Duration;

use notify::event::{AccessKind, AccessMode, CreateKind, ModifyKind};
use notify::{
    Config as NotifyConfig, Event, EventKind, PollWatcher, RecommendedWatcher, RecursiveMode,
    Watcher,
};

use crate::{is_skippable_dir, lang_for_path, source_walker, Lattice};

/// The OS watcher behind a [`LatticeWatcher`]: native (inotify/FSEvents/ReadDirectoryChanges) or
/// polling, chosen per filesystem. Shared with the reindex worker (weakly) so it can register
/// watches for directories created after startup.
type SharedWatcher = Arc<Mutex<Box<dyn Watcher + Send>>>;

/// Keeps the background watcher alive; dropping it stops watching AND joins the reindex worker so no
/// thread leaks. Holds the OS watcher backend (native or polling) plus the worker thread that drains
/// changed paths and reindexes them OFF the notify thread (so a save burst can't serialize
/// reindexing on the watcher thread or hold the store write lock too long).
pub struct LatticeWatcher {
    // Dropped FIRST (see the explicit `Drop`): the watcher owns the channel `Sender` (it lives in
    // the handler closure), so dropping it disconnects the channel, which is the worker's shutdown
    // signal. `Option` so `Drop` can `take()` it before joining the worker.
    inner: Option<SharedWatcher>,
    worker: Option<JoinHandle<()>>,
    errors: Arc<Mutex<Vec<String>>>,
}

impl LatticeWatcher {
    fn new(inner: SharedWatcher, worker: JoinHandle<()>, errors: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            inner: Some(inner),
            worker: Some(worker),
            errors,
        }
    }

    /// Drain backend errors observed since the last poll. A watcher remains alive after an error,
    /// but callers must surface this state so a dead backend cannot look like a healthy index.
    pub fn take_errors(&self) -> Vec<String> {
        self.errors
            .lock()
            .map(|mut errors| std::mem::take(&mut *errors))
            .unwrap_or_default()
    }
}

impl Drop for LatticeWatcher {
    fn drop(&mut self) {
        // Drop the watcher first: that drops its handler closure, the sole channel `Sender`, so the
        // worker's `recv()` returns `Err` (disconnected) and the loop exits. The worker only ever
        // holds a `Weak` to the watcher, so this strong reference is the last one. Then join the
        // worker so it's torn down deterministically instead of leaked.
        self.inner.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// What the notify handler forwards to the reindex worker. Both are cheap to produce on the
/// notify thread; all filesystem work happens on the worker.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Change {
    /// A supported source file was written, created, moved, or removed.
    Source(PathBuf),
    /// A directory appeared inside the watched tree: register it (and index what it holds).
    NewDir(PathBuf),
}

/// How often the POLLING backend rescans the tree on a filesystem without working inotify (9p/etc.).
/// A balance between reindex latency and the cost of a full stat-walk over a remote/host link.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// After the worker pulls the first change of a batch, it waits at least this long for stragglers
/// from the same save burst (a multi-file save, a `git checkout`) before reindexing — so paths
/// arriving in quick succession coalesce into one reindex pass instead of one pass each. The
/// caller's debounce window extends this when it is longer (see [`spawn_watcher`]).
const COALESCE_WINDOW: Duration = Duration::from_millis(50);

/// Watch the source directories under `root` and reindex changed source files into `lattice`.
/// Returns an error only if the OS watcher can't be set up at all; the returned handle must be
/// kept alive for watching to continue. `debounce` is how long a burst of changes is allowed to
/// settle before it is reindexed.
///
/// Picks the backend by filesystem: native local filesystems use the efficient inotify watcher;
/// a non-native filesystem (WSL2's `/mnt/*` DrvFs/9p, a FUSE mount, an SMB/NFS share) uses a
/// **polling** watcher instead. Recursive inotify registration on 9p RPCs to the host per entry and
/// some calls land in uninterruptible `D` state — which used to hang `forge chat` — whereas polling
/// just stat-walks the tree on a timer (ordinary file ops that work over 9p). The caller runs this
/// off the startup path so the initial registration walk (synchronous, and slow over a remote
/// link) can't gate the UI.
pub fn spawn_watcher(
    lattice: Arc<Lattice>,
    root: &Path,
    debounce: Duration,
) -> Result<LatticeWatcher, String> {
    build_watcher(
        lattice,
        root,
        needs_polling(root),
        POLL_INTERVAL,
        debounce.max(COALESCE_WINDOW),
    )
}

/// Build the backend explicitly (the `poll` decision + interval are parameters so tests can exercise
/// the polling path on a native test filesystem). `poll=false` → native; `poll=true` → stat-walk
/// every `poll_interval`. `coalesce` is the worker's batch window (see [`COALESCE_WINDOW`]).
fn build_watcher(
    lattice: Arc<Lattice>,
    root: &Path,
    poll: bool,
    poll_interval: Duration,
    coalesce: Duration,
) -> Result<LatticeWatcher, String> {
    build_watcher_with(root, poll, poll_interval, coalesce, move |path| {
        if let Err(e) = lattice.reindex_path(path) {
            tracing::warn!("lattice reindex of {path:?} failed: {e}");
        }
    })
}

/// [`build_watcher`] with the per-file action injected, so tests can count reindexes precisely
/// instead of inferring them from index contents.
///
/// The notify thread runs the cheap `handler`: it classifies each event and *enqueues* the
/// affected path onto a channel, then returns immediately, so a burst of saves never serializes
/// reindexing (which takes the store write lock) on the watcher thread. A dedicated worker thread
/// drains the channel, coalesces duplicate paths within `coalesce`, registers newly created
/// directories, and reindexes — off the watcher thread.
fn build_watcher_with<F>(
    root: &Path,
    poll: bool,
    poll_interval: Duration,
    coalesce: Duration,
    reindex: F,
) -> Result<LatticeWatcher, String>
where
    F: FnMut(&Path) + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel::<Change>();
    let errors = Arc::new(Mutex::new(Vec::new()));

    // The handler runs ON the notify thread, so it must stay cheap: classify and forward. A send
    // only fails once the worker has exited (channel closed); that can't normally happen while the
    // watcher is alive, so it's ignored.
    let handler_errors = Arc::clone(&errors);
    let handler = move |res: notify::Result<Event>| match res {
        Ok(event) => forward_event(&event, &tx),
        Err(e) => record_watcher_error(&handler_errors, &format!("notify backend error: {e}")),
    };

    let mut watcher: Box<dyn Watcher + Send> = if poll {
        // compare_contents so a same-SIZE edit (changing a value, a rename of equal length) is still
        // caught — metadata-only polling would miss it if mtime granularity coincides. Costs a content
        // read per file per tick, bounded by the source-directory scope.
        let config = NotifyConfig::default()
            .with_poll_interval(poll_interval)
            .with_compare_contents(true);
        Box::new(PollWatcher::new(handler, config).map_err(|e| e.to_string())?)
    } else {
        Box::new(
            RecommendedWatcher::new(handler, NotifyConfig::default()).map_err(|e| e.to_string())?,
        )
    };

    // Register exactly the directories the indexer walks — one non-recursive watch each — instead
    // of one recursive watch on the root that would pull in every build tree and worktree.
    let mut watched: HashSet<PathBuf> = HashSet::new();
    for dir in source_dirs(root) {
        match watcher.watch(&dir, RecursiveMode::NonRecursive) {
            Ok(()) => {
                watched.insert(dir);
            }
            Err(e) if dir == root => return Err(e.to_string()),
            Err(e) => tracing::debug!("lattice watch skipped {dir:?}: {e}"),
        }
    }

    let shared: SharedWatcher = Arc::new(Mutex::new(watcher));
    let worker_watcher = Arc::downgrade(&shared);
    let worker = std::thread::Builder::new()
        .name("forge-lattice-reindex".into())
        .spawn(move || {
            let mut registrar = DirRegistrar {
                watcher: worker_watcher,
                watched,
            };
            run_reindex_worker(rx, coalesce, &mut registrar, reindex);
        })
        .map_err(|e| e.to_string())?;
    Ok(LatticeWatcher::new(shared, worker, errors))
}

/// Every directory the indexer would walk under `root`, root first. Shares the indexer's walker so
/// the watched set and the indexed set can't drift apart.
fn source_dirs(root: &Path) -> Vec<PathBuf> {
    source_walker(root)
        .flatten()
        .filter(|entry| entry.file_type().is_some_and(|t| t.is_dir()))
        .map(|entry| entry.into_path())
        .collect()
}

/// Turn one notify event into worker messages. Pure apart from a single `is_dir` stat on
/// create/rename events (needed to tell a new directory from a new file); every other event is
/// classified from its kind and path alone.
fn forward_event(event: &Event, tx: &Sender<Change>) {
    if !is_content_event(&event.kind) {
        return;
    }
    let may_be_new_dir = matches!(
        event.kind,
        EventKind::Create(CreateKind::Folder)
            | EventKind::Create(CreateKind::Any)
            | EventKind::Modify(ModifyKind::Name(_))
    );
    for path in &event.paths {
        if should_reindex(path) {
            let _ = tx.send(Change::Source(path.clone()));
        } else if may_be_new_dir && is_watchable_dir(path) {
            let _ = tx.send(Change::NewDir(path.clone()));
        }
    }
}

/// Whether an event kind can reflect a change to file contents or the tree. Access events —
/// open, read, close-without-write — cannot, and the reindexer's own reads produce them, so
/// forwarding them would make the watcher feed itself. `Close(Write)` is the one access event
/// that means "a writer finished" and is kept.
fn is_content_event(kind: &EventKind) -> bool {
    match kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        _ => true,
    }
}

/// A directory the watcher should register when it appears at runtime: exists, is a directory,
/// and no component of its path is a skipped directory (a new `target/debug/…` build dir or a
/// fresh worktree under `.forge/` must not pull the watch set back up to tens of thousands).
fn is_watchable_dir(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    let skipped = path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .any(is_skippable_dir);
    !skipped && !crate::is_toolchain_dir(path) && !path.join(".git").exists()
}

/// Owns the worker-side view of the watch set: which directories are registered, and a weak handle
/// to the watcher to register more. Weak so the worker can never keep the watcher (and so its own
/// `Sender`) alive past the `LatticeWatcher` drop that is meant to stop it.
struct DirRegistrar {
    watcher: Weak<Mutex<Box<dyn Watcher + Send>>>,
    watched: HashSet<PathBuf>,
}

impl DirRegistrar {
    /// Register `dir` and every source directory beneath it, returning the source files already
    /// inside (files can land before the watch does, so they are indexed explicitly).
    fn register(&mut self, dir: &Path) -> Vec<PathBuf> {
        let Some(watcher) = self.watcher.upgrade() else {
            return Vec::new();
        };
        let mut files = Vec::new();
        for entry in source_walker(dir).flatten() {
            let path = entry.path();
            if entry.file_type().is_some_and(|t| t.is_dir()) {
                if self.watched.contains(path) {
                    continue;
                }
                let Ok(mut guard) = watcher.lock() else {
                    return files; // poisoned: the watcher thread panicked; nothing to register into
                };
                match guard.watch(path, RecursiveMode::NonRecursive) {
                    Ok(()) => {
                        self.watched.insert(path.to_path_buf());
                    }
                    Err(e) => tracing::debug!("lattice watch skipped {path:?}: {e}"),
                }
            } else if should_reindex(path) {
                files.push(path.to_path_buf());
            }
        }
        files
    }
}

fn record_watcher_error(errors: &Arc<Mutex<Vec<String>>>, detail: &str) {
    tracing::error!(error = detail, "lattice watcher backend failed");
    if let Ok(mut recorded) = errors.lock() {
        recorded.push(detail.to_string());
    }
}

/// Drain changes off `rx` and apply `reindex` to each changed path, OFF the watcher thread.
/// Coalesces a burst: after the first change arrives it grabs everything already queued, waits
/// `coalesce` for stragglers from the same save, grabs those too, then reindexes each unique path
/// ONCE — so one save that emits the same path several times reindexes it a single time, not N
/// times. New directories are registered before the batch is indexed so the files they already
/// hold are part of the same pass. Exits when the channel disconnects (every `Sender` dropped,
/// i.e. the watcher was dropped), which is the clean-shutdown signal.
fn run_reindex_worker<F: FnMut(&Path)>(
    rx: Receiver<Change>,
    coalesce: Duration,
    registrar: &mut DirRegistrar,
    mut reindex: F,
) {
    while let Ok(first) = rx.recv() {
        let mut batch: HashSet<Change> = HashSet::new();
        batch.insert(first);
        drain_pending(&rx, &mut batch);
        if !coalesce.is_zero() {
            std::thread::sleep(coalesce);
            drain_pending(&rx, &mut batch);
        }
        let mut paths: HashSet<PathBuf> = HashSet::new();
        for change in batch {
            match change {
                Change::Source(path) => {
                    paths.insert(path);
                }
                Change::NewDir(dir) => paths.extend(registrar.register(&dir)),
            }
        }
        for path in &paths {
            reindex(path);
        }
    }
}

/// Move every currently-queued change from `rx` into `batch` (deduping), without blocking. Stops at
/// the first `Empty` (nothing more queued right now) or `Disconnected` (sender gone) — both end the
/// drain.
fn drain_pending(rx: &Receiver<Change>, batch: &mut HashSet<Change>) {
    while let Ok(change) = rx.try_recv() {
        batch.insert(change);
    }
}

/// A changed path is worth reindexing only if it's a supported source file and none of its path
/// components is a skipped directory (build output, `.git`, `node_modules`, …).
fn should_reindex(path: &Path) -> bool {
    // Only the DIRECTORY components are skip-tested — not the filename. `is_skippable_dir` treats any
    // dot-prefixed name as a skipped dir, so checking the final component wrongly excluded dotfile
    // SOURCE files (`.eslintrc.js`, a hidden `.foo.rs`) that the initial `update()` walk DID index
    // (it only applies the skip to directory entries), leaving the watcher unable to refresh them.
    let skipped = path
        .parent()
        .into_iter()
        .flat_map(|p| p.components())
        .filter_map(|c| c.as_os_str().to_str())
        .any(is_skippable_dir);
    if skipped {
        return false;
    }
    // Installed toolchain/SDK trees (Go module cache, Android SDK, site-packages) — never source
    // the user edits, and the single largest contributor to a runaway index (root.rs).
    if path.parent().is_some_and(crate::is_toolchain_dir) {
        return false;
    }
    path.to_str().and_then(lang_for_path).is_some()
}

/// Filesystem types where a recursive inotify watch is unreliable or outright blocking, so the
/// watcher uses the POLLING backend instead. They back onto a remote/host process, so per-entry
/// inotify registration is an RPC that can stall (uninterruptibly, on 9p). `v9fs`/`9p` is WSL2's
/// DrvFs (`/mnt/c`); `fuse*` covers sshfs/rclone/etc.; `cifs`/`smb*` are Windows shares; `nfs*` is
/// NFS. Native local filesystems (ext4, btrfs, xfs, apfs, ntfs3, …) are not listed → inotify.
fn is_poll_only_fstype(fstype: &str) -> bool {
    let fstype = fstype.trim();
    matches!(fstype, "9p" | "v9fs" | "cifs" | "smb3" | "smbfs" | "ncpfs")
        || fstype.starts_with("fuse")
        || fstype.starts_with("nfs")
}

/// Whether `root` lives on a filesystem that needs the polling backend (inotify unreliable/blocking).
/// Linux-only detection via `/proc/self/mountinfo`; other platforms / undetectable fs → `false`
/// (fail toward the efficient inotify backend, which works on every native filesystem).
fn needs_polling(root: &Path) -> bool {
    root_fstype(root)
        .map(|fs| is_poll_only_fstype(&fs))
        .unwrap_or(false)
}

/// The filesystem type backing `root`, from `/proc/self/mountinfo` (Linux only). Returns `None`
/// off-Linux, or when the file can't be read / no mount matches.
fn root_fstype(root: &Path) -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    // Canonicalize so symlinks resolve to the real mount; fall back to the path as given.
    let canon = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let target = canon.to_str()?;
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    fstype_for_path(&mountinfo, target).map(str::to_string)
}

/// Pure parse of `/proc/self/mountinfo`: return the fstype of the mount whose mount point is the
/// LONGEST prefix of `target` (the most specific mount that contains the path). mountinfo format:
/// `<id> <pid> <maj:min> <root> <MOUNTPOINT> <opts> <optfields...> - <FSTYPE> <source> <superopts>`
/// — the mount point is the 5th space-separated field and the fstype is the first token after the
/// ` - ` separator.
fn fstype_for_path<'a>(mountinfo: &'a str, target: &str) -> Option<&'a str> {
    let mut best: Option<(&str, &str)> = None; // (mountpoint, fstype)
    for line in mountinfo.lines() {
        let (pre, post) = match line.split_once(" - ") {
            Some(p) => p,
            None => continue,
        };
        let mountpoint = match pre.split_whitespace().nth(4) {
            Some(m) => m,
            None => continue,
        };
        let fstype = match post.split_whitespace().next() {
            Some(f) => f,
            None => continue,
        };
        // A mount point contains `target` if target == mountpoint or target starts with
        // `mountpoint/` (so `/mnt` doesn't spuriously match `/mntarget`). `/` matches everything.
        let contains = mountpoint == "/"
            || target == mountpoint
            || target
                .strip_prefix(mountpoint)
                .is_some_and(|rest| rest.starts_with('/'));
        if contains && best.is_none_or(|(m, _)| mountpoint.len() > m.len()) {
            best = Some((mountpoint, fstype));
        }
    }
    best.map(|(_, fstype)| fstype)
}

/// Resolve the directory to recursively watch, given the launch `cwd` and the user's `home`.
/// Prefers the nearest enclosing PROJECT ROOT (a dir holding `.git`, `.forge`, or `AGENTS.md`) so
/// the watch covers the codebase rather than whatever happens to sit above it. Returns `None` —
/// "don't watch" — when the resolved root would be the home directory itself: recursively watching
/// all of `$HOME` is pathological (it pulls in `.cargo`, cloned `.git` trees, caches — thousands of
/// inotify watches and a slow initial walk) and is virtually never intended. The upward climb stops
/// at `home`, so we never walk past it into `/` and watch a system root either. When `home` is
/// unknown (`None`), nothing is refused — fail open and watch the discovered root / `cwd`.
pub fn resolve_watch_root(cwd: &Path, home: Option<&Path>) -> Option<PathBuf> {
    const MARKERS: [&str; 3] = [".git", ".forge", "AGENTS.md"];
    let mut dir = cwd;
    let mut found: Option<&Path> = None;
    loop {
        if MARKERS.iter().any(|m| dir.join(m).exists()) {
            found = Some(dir);
            break;
        }
        if Some(dir) == home {
            break; // never climb above $HOME
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => break,
        }
    }
    let root = found.unwrap_or(cwd);
    // Shares [`crate::is_home_or_system_root`] with the indexer's root policy so the two entry
    // points refuse exactly the same set of roots — the asymmetry between them (watcher refused
    // $HOME, indexer did not) is what let an entire home directory into the index.
    if crate::is_home_or_system_root(root, home) {
        return None;
    }
    Some(root.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Lattice;
    use forge_store::Store;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    /// A registrar with no watcher behind it: `NewDir` messages become no-ops.
    fn detached_registrar() -> DirRegistrar {
        DirRegistrar {
            watcher: Weak::new(),
            watched: HashSet::new(),
        }
    }

    fn fresh_root(tag: &str) -> PathBuf {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("forge-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        root
    }

    /// Number of inotify watches this process currently holds, from `/proc/self/fdinfo`. Linux only;
    /// `None` elsewhere or when unreadable.
    fn inotify_watch_count() -> Option<usize> {
        if !cfg!(target_os = "linux") {
            return None;
        }
        let mut total = 0;
        for entry in std::fs::read_dir("/proc/self/fdinfo").ok()?.flatten() {
            if let Ok(info) = std::fs::read_to_string(entry.path()) {
                total += info
                    .lines()
                    .filter(|l| l.starts_with("inotify wd:"))
                    .count();
            }
        }
        Some(total)
    }

    #[test]
    fn access_events_are_not_changes_but_a_finished_write_is() {
        // The reindexer's own reads (open / read / close-without-write) must never come back as
        // work; the loop that pinned a core per Forge process was exactly that.
        assert!(!is_content_event(&EventKind::Access(AccessKind::Open(
            AccessMode::Read
        ))));
        assert!(!is_content_event(&EventKind::Access(AccessKind::Read)));
        assert!(!is_content_event(&EventKind::Access(AccessKind::Close(
            AccessMode::Read
        ))));
        assert!(!is_content_event(&EventKind::Access(AccessKind::Any)));
        // A writer closing its handle is how an editor save shows up on inotify.
        assert!(is_content_event(&EventKind::Access(AccessKind::Close(
            AccessMode::Write
        ))));
        assert!(is_content_event(&EventKind::Modify(ModifyKind::Any)));
        assert!(is_content_event(&EventKind::Create(CreateKind::File)));
        assert!(is_content_event(&EventKind::Remove(
            notify::event::RemoveKind::File
        )));
    }

    #[test]
    fn the_reindexers_own_reads_do_not_feed_the_watcher() {
        // Production shape: the reindex action READS the file it was woken for. With access events
        // forwarded, that read is itself an event for the same file, and the watcher feeds itself
        // forever (one core per Forge process, observed). So: trigger one real write, wait for it
        // to be reindexed, then prove the count stops moving once the tree is quiet.
        let root = fresh_root("read");
        let file = root.join("src/a.rs");
        std::fs::write(&file, "pub fn alpha() {}\n").unwrap();

        let reindexes = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::clone(&reindexes);
        let _w = build_watcher_with(
            &root,
            false,
            POLL_INTERVAL,
            Duration::from_millis(50),
            move |path| {
                let _ = std::fs::read_to_string(path);
                r2.fetch_add(1, Ordering::SeqCst);
            },
        )
        .expect("watcher starts");

        let mut saw_write = false;
        for _ in 0..60 {
            std::fs::write(&file, "pub fn beta() {}\n").unwrap();
            std::thread::sleep(Duration::from_millis(100));
            if reindexes.load(Ordering::SeqCst) > 0 {
                saw_write = true;
                break;
            }
        }
        assert!(saw_write, "a real write must reindex");

        // Let the write's own burst settle, then measure over a quiet window.
        std::thread::sleep(Duration::from_millis(400));
        let settled = reindexes.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(600));
        let later = reindexes.load(Ordering::SeqCst);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(
            later, settled,
            "reindex count kept climbing with no writes: the watcher is feeding itself"
        );
    }

    #[test]
    fn only_source_directories_are_watched() {
        // Build output, VCS, and dependency trees get no watch at all — not merely filtered after
        // the fact. Measured directly on inotify where available: exactly root + src.
        let root = fresh_root("scope");
        std::fs::write(root.join("src/a.rs"), "pub fn alpha() {}\n").unwrap();
        for dir in [
            "target/debug/deps",
            ".git/objects",
            "node_modules/pkg",
            "vendor/x",
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            std::fs::write(root.join(dir).join("gen.rs"), "pub fn noise() {}\n").unwrap();
        }

        let dirs = source_dirs(&root);
        assert_eq!(dirs[0], root, "root first");
        assert!(dirs.contains(&root.join("src")));
        assert_eq!(dirs.len(), 2, "no build/VCS/dependency dirs: {dirs:?}");

        let before = inotify_watch_count();
        let reindexes = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::clone(&reindexes);
        let w = build_watcher_with(
            &root,
            false,
            POLL_INTERVAL,
            Duration::from_millis(50),
            move |_| {
                r2.fetch_add(1, Ordering::SeqCst);
            },
        )
        .expect("watcher starts");
        if let (Some(before), Some(after)) = (before, inotify_watch_count()) {
            assert_eq!(after - before, 2, "one inotify watch per source directory");
        }

        // Churn in target/ (what a cargo build does) never reaches the worker.
        for i in 0..20 {
            std::fs::write(
                root.join("target/debug/deps/gen.rs"),
                format!("pub fn noise{i}() {{}}\n"),
            )
            .unwrap();
        }
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(reindexes.load(Ordering::SeqCst), 0);
        drop(w);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_directory_created_after_startup_is_watched() {
        // A new module directory (mkdir + first file) must be picked up without a restart: the
        // watch set follows the tree, it is not a startup snapshot.
        let root = fresh_root("newdir");
        let store = Arc::new(Store::open_in_memory().unwrap());
        let lattice = Arc::new(Lattice::new(store, &root));
        lattice.update().unwrap();
        let _w = spawn_watcher(Arc::clone(&lattice), &root, Duration::from_millis(100))
            .expect("watcher starts");

        let new_dir = root.join("src/later");
        let file = new_dir.join("b.rs");
        let mut reindexed = false;
        for _ in 0..60 {
            std::fs::create_dir_all(&new_dir).unwrap();
            std::fs::write(&file, "pub fn delta() {}\n").unwrap();
            std::thread::sleep(Duration::from_millis(100));
            if lattice.query("delta", 5).unwrap().len() == 1 {
                reindexed = true;
                break;
            }
        }
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            reindexed,
            "a file in a directory created after startup was not indexed"
        );
    }

    #[test]
    fn should_reindex_allows_dotfile_source_but_skips_dot_dirs() {
        // A dot-prefixed SOURCE file must be reindexed (the initial walk indexes it); only a
        // dot/skip DIRECTORY in the path excludes it.
        assert!(
            should_reindex(Path::new("src/.hidden.rs")),
            "dotfile source"
        );
        assert!(should_reindex(Path::new(".eslintrc.js")), "dotfile at root");
        assert!(!should_reindex(Path::new(".git/config.rs")), "inside .git");
        assert!(
            !should_reindex(Path::new("node_modules/x.js")),
            "vendor dir"
        );
        assert!(!should_reindex(Path::new("src/a.txt")), "unsupported ext");
    }

    #[test]
    fn external_edit_is_reindexed_automatically() {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("forge-watch-{}-{n}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        let file = root.join("src/a.rs");
        std::fs::write(&file, "pub fn alpha() {}\n").unwrap();

        let store = Arc::new(Store::open_in_memory().unwrap());
        let lattice = Arc::new(Lattice::new(store, &root));
        lattice.update().unwrap();
        assert_eq!(lattice.query("alpha", 5).unwrap().len(), 1);

        let _watcher = spawn_watcher(Arc::clone(&lattice), &root, Duration::from_millis(150))
            .expect("watcher starts");

        // The edit is repeated every attempt rather than made once before the loop. `spawn_watcher`
        // returning does not mean the OS watch is registered — registration happens on the watcher's
        // own thread — so a single write can land in that gap, produce no event, and then no amount
        // of waiting can recover it. Alone the gap is too small to notice; run beside the other
        // watch tests, or on a loaded machine, and it is wide enough to fail. Re-writing until the
        // watch exists tests what the test means to test, without weakening the assertion.
        let mut reindexed = false;
        for _ in 0..60 {
            std::fs::write(&file, "pub fn beta() {}\n").unwrap();
            std::thread::sleep(Duration::from_millis(100));
            if lattice.query("beta", 5).unwrap().len() == 1 {
                reindexed = true;
                break;
            }
        }
        let _ = std::fs::remove_dir_all(&root);
        assert!(reindexed, "watcher did not reindex the external edit");
    }

    #[test]
    fn poll_backend_reindexes_external_edit() {
        // Proves the POLLING backend (used on 9p/remote filesystems where inotify is unreliable)
        // actually picks up edits — exercised here on the native test fs via `build_watcher(poll=true)`
        // with a short interval, since CI has no 9p mount.
        let n = N.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("forge-poll-{}-{n}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        let file = root.join("src/a.rs");
        std::fs::write(&file, "pub fn alpha() {}\n").unwrap();

        let store = Arc::new(Store::open_in_memory().unwrap());
        let lattice = Arc::new(Lattice::new(store, &root));
        lattice.update().unwrap();

        let _w = build_watcher(
            Arc::clone(&lattice),
            &root,
            true, // force the polling backend
            Duration::from_millis(150),
            Duration::from_millis(100),
        )
        .expect("poll watcher starts");

        // Repeated per attempt for the same reason as the inotify tests: the polling backend's first
        // scan establishes the baseline it compares against, so an edit made before that scan is
        // simply part of the baseline and is never seen as a change.
        let mut reindexed = false;
        for _ in 0..80 {
            std::fs::write(&file, "pub fn gamma() {}\n").unwrap();
            std::thread::sleep(Duration::from_millis(100));
            if lattice.query("gamma", 5).unwrap().len() == 1 {
                reindexed = true;
                break;
            }
        }
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            reindexed,
            "polling watcher did not reindex the external edit"
        );
    }

    #[test]
    fn watcher_held_only_through_a_channel_still_reindexes() {
        // Production never drains the watcher: it's sent into an mpsc channel and the Session holds
        // the Receiver for keep-alive (so setup is off-thread + the watcher is owned per-session).
        // Prove a watcher sitting UN-received in the channel buffer still runs and reindexes — and
        // that dropping the Receiver tears it down.
        let n = N.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("forge-chan-{}-{n}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        let file = root.join("src/a.rs");
        std::fs::write(&file, "pub fn alpha() {}\n").unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        let lattice = Arc::new(Lattice::new(store, &root));
        lattice.update().unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        let watcher =
            spawn_watcher(Arc::clone(&lattice), &root, Duration::from_millis(100)).expect("starts");
        tx.send(watcher).unwrap(); // hand off to the channel; never received, kept alive by `rx`

        // Repeated per attempt for the registration race described in
        // `external_edit_is_reindexed_automatically`.
        let mut reindexed = false;
        for _ in 0..60 {
            std::fs::write(&file, "pub fn omega() {}\n").unwrap();
            std::thread::sleep(Duration::from_millis(100));
            if lattice.query("omega", 5).unwrap().len() == 1 {
                reindexed = true;
                break;
            }
        }
        drop(rx); // dropping the Receiver drops the buffered watcher → watching stops
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            reindexed,
            "channel-held watcher did not reindex the external edit"
        );
    }

    #[test]
    fn worker_coalesces_a_burst_and_stops_on_sender_drop() {
        // A rapid burst of changes to the SAME path must coalesce into far fewer reindexes than the
        // number of events (don't reindex a file 10× for one save), and dropping the sole `Sender`
        // (what `LatticeWatcher::drop` does to the watcher's handler) must stop the worker so its
        // thread can be joined — no leak.
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (tx, rx) = std::sync::mpsc::channel::<Change>();
        let reindexes = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::clone(&reindexes);
        let worker = std::thread::spawn(move || {
            let mut registrar = detached_registrar();
            run_reindex_worker(rx, Duration::from_millis(80), &mut registrar, move |_p| {
                r2.fetch_add(1, Ordering::SeqCst);
            });
        });

        // Fire 20 events for one path back-to-back; they should land within a single coalesce window.
        let path = PathBuf::from("src/a.rs");
        for _ in 0..20 {
            tx.send(Change::Source(path.clone())).unwrap();
        }

        drop(tx); // sole sender gone → worker must finish the batch and exit
        worker
            .join()
            .expect("worker thread joins after sender drop");

        let n = reindexes.load(Ordering::SeqCst);
        assert!(n >= 1, "the burst must reindex the path at least once");
        assert!(
            n < 20,
            "20 events for one path must coalesce to fewer reindexes, got {n}"
        );
    }

    #[test]
    fn worker_reindexes_distinct_paths_in_a_batch() {
        // Coalescing dedups by path, so distinct paths in one burst are each reindexed once.
        use std::sync::Mutex;

        let (tx, rx) = std::sync::mpsc::channel::<Change>();
        let seen: Arc<Mutex<HashSet<PathBuf>>> = Arc::new(Mutex::new(HashSet::new()));
        let s2 = Arc::clone(&seen);
        let worker = std::thread::spawn(move || {
            let mut registrar = detached_registrar();
            run_reindex_worker(rx, Duration::from_millis(80), &mut registrar, move |p| {
                s2.lock().unwrap().insert(p.to_path_buf());
            });
        });

        for name in ["src/a.rs", "src/b.rs", "src/c.rs"] {
            // each path sent twice — dedup should still reindex each once
            tx.send(Change::Source(PathBuf::from(name))).unwrap();
            tx.send(Change::Source(PathBuf::from(name))).unwrap();
        }
        drop(tx);
        worker.join().expect("worker joins");

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3, "each distinct path reindexed");
        assert!(seen.contains(Path::new("src/b.rs")));
    }

    #[test]
    fn skips_build_dirs_and_unsupported_files() {
        assert!(should_reindex(Path::new("crates/forge-index/src/lib.rs")));
        assert!(!should_reindex(Path::new("target/debug/build.rs")));
        assert!(!should_reindex(Path::new("notes.txt")));
        assert!(!should_reindex(Path::new("node_modules/x/index.js")));
    }

    #[test]
    fn poll_only_fstype_classifies_remote_vs_native() {
        // Remote / host-backed → inotify unreliable, use the polling backend.
        for fs in [
            "9p",
            "v9fs",
            "cifs",
            "smb3",
            "nfs",
            "nfs4",
            "fuse",
            "fuse.sshfs",
        ] {
            assert!(is_poll_only_fstype(fs), "{fs} should use polling");
        }
        // Native local → efficient inotify backend.
        for fs in [
            "ext4", "btrfs", "xfs", "zfs", "apfs", "ntfs3", "tmpfs", "overlay",
        ] {
            assert!(!is_poll_only_fstype(fs), "{fs} should use inotify");
        }
    }

    // A realistic WSL2 mountinfo: `/` is ext4 (Linux home), `/mnt/c` is 9p (DrvFs).
    const WSL_MOUNTINFO: &str = "\
23 30 0:22 / /sys rw,nosuid - sysfs sysfs rw
24 30 0:23 / /proc rw,nosuid - proc proc rw
30 0 8:32 / / rw,relatime - ext4 /dev/sdc rw,discard,errors=remount-ro
70 30 0:55 / /mnt/c rw,noatime - 9p drvfs rw,dirsync,aname=drvfs;path=C:\\,mmap,trans=fd
71 30 0:56 / /mnt/wsl/docker rw - tmpfs tmpfs rw";

    #[test]
    fn fstype_for_path_picks_the_most_specific_mount() {
        // A path under /mnt/c resolves to 9p (DrvFs) — the reported hang case.
        assert_eq!(
            fstype_for_path(WSL_MOUNTINFO, "/mnt/c/Users/Quinn/project"),
            Some("9p")
        );
        // A path on the Linux home falls through to the root ext4 mount.
        assert_eq!(
            fstype_for_path(WSL_MOUNTINFO, "/home/quinn/project"),
            Some("ext4")
        );
        // The mount point itself matches exactly.
        assert_eq!(fstype_for_path(WSL_MOUNTINFO, "/mnt/c"), Some("9p"));
        // A sibling that only shares a name prefix must NOT match /mnt/c (longest-real-prefix).
        assert_eq!(
            fstype_for_path(WSL_MOUNTINFO, "/mnt/computer/x"),
            Some("ext4")
        );
    }

    #[test]
    fn resolve_watch_root_prefers_project_root_and_refuses_home() {
        let tmp = std::env::temp_dir().join(format!(
            "forge-root-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let home = tmp.join("home");
        let proj = home.join("work/myproj");
        let sub = proj.join("crates/x/src");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(proj.join(".git")).unwrap();

        // From deep inside the project, the watch root climbs to the .git project root.
        assert_eq!(
            resolve_watch_root(&sub, Some(&home)),
            Some(proj.clone()),
            "should scope the watch to the nearest project root"
        );
        // From the project root itself, it stays there.
        assert_eq!(resolve_watch_root(&proj, Some(&home)), Some(proj.clone()));
        // Launched in $HOME with no project marker → refuse (don't watch all of home).
        assert_eq!(resolve_watch_root(&home, Some(&home)), None);
        // A marker-less subdir of home that is NOT home → watch that specific dir (not all of home).
        let loose = home.join("scratch");
        std::fs::create_dir_all(&loose).unwrap();
        assert_eq!(resolve_watch_root(&loose, Some(&home)), Some(loose.clone()));
        // Even if $HOME itself holds a .git (dotfiles repo), refuse — the root resolves to home.
        std::fs::create_dir_all(home.join(".git")).unwrap();
        assert_eq!(resolve_watch_root(&loose, Some(&home)), None);
        // Unknown home → never refuse; resolves to the nearest project root (home/.git now exists
        // from the line above), proving home=None can't trigger the refuse branch.
        assert_eq!(resolve_watch_root(&loose, None), Some(home.clone()));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn fstype_drives_poll_decision_9p_polls_native_inotify() {
        // The /mnt/c (9p) case → polling backend; the Linux home → inotify.
        assert_eq!(fstype_for_path(WSL_MOUNTINFO, "/home/quinn"), Some("ext4"));
        assert!(is_poll_only_fstype(
            fstype_for_path(WSL_MOUNTINFO, "/mnt/c/x").unwrap()
        ));
        assert!(!is_poll_only_fstype(
            fstype_for_path(WSL_MOUNTINFO, "/home/quinn").unwrap()
        ));
    }
}
