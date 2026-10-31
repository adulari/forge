//! Permanent session deletion shared by every surface: the daemon route the apps use, `forge
//! sessions --delete`, and the chat's `/delete` and sessions picker. One set of safety checks, one
//! deletion, one cleanup of the on-disk files a session leaves behind.

use std::path::Path;
use std::sync::Arc;

use forge_store::{DeleteTarget, SessionDeletePreview, Store};

/// Why a delete was refused. `message` says what is wrong, `next_step` what to do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    NotFound,
    OpenInTerminal {
        host: String,
    },
    Running,
    OwnsWorktree(String),
    Frozen,
    Failed(String),
    /// The daemon refused; its own text already says what to do.
    Daemon(String),
}

impl Refusal {
    pub(crate) fn message(&self) -> String {
        match self {
            Self::NotFound => "no such session".into(),
            Self::OpenInTerminal { host } => {
                format!("this session is open in a forge chat terminal on {host}")
            }
            Self::Running => "this session is running in the Forge daemon".into(),
            Self::OwnsWorktree(path) => format!("this session still owns the worktree {path}"),
            Self::Frozen => "this session is frozen by an Anywhere handoff".into(),
            Self::Failed(why) => format!("deleting failed: {why}"),
            Self::Daemon(text) => text.clone(),
        }
    }

    pub(crate) fn next_step(&self) -> &'static str {
        match self {
            Self::NotFound => "run `forge sessions` to list the ids",
            Self::OpenInTerminal { .. } => {
                "run /delete in that terminal, or close the chat and delete it here"
            }
            Self::Running => "stop or archive the running session first, then delete it",
            Self::OwnsWorktree(_) => {
                "resume the session and merge or discard its worktree first, then delete it"
            }
            Self::Frozen => "finish or cancel the Anywhere handoff, then delete it",
            Self::Failed(_) => "nothing was deleted; try again",
            Self::Daemon(_) => "",
        }
    }

    /// Message and next step in one sentence, for toasts and HTTP bodies.
    pub(crate) fn full(&self) -> String {
        match self.next_step() {
            "" => self.message(),
            next => format!("{}; {next}", self.message()),
        }
    }

    pub(crate) fn status(&self) -> axum::http::StatusCode {
        match self {
            Self::NotFound => axum::http::StatusCode::NOT_FOUND,
            Self::Failed(_) | Self::Daemon(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            _ => axum::http::StatusCode::CONFLICT,
        }
    }
}

pub(crate) fn host_name() -> String {
    sysinfo::System::host_name().unwrap_or_else(|| "this machine".into())
}

/// The shared safety checks. `running_in_daemon` is whether the daemon's registry holds a live
/// driver for the session; only the daemon knows, every other caller passes `false`.
pub(crate) async fn check(
    store: &Arc<Store>,
    id: &str,
    running_in_daemon: bool,
) -> Result<SessionDeletePreview, Refusal> {
    check_with(store, id, running_in_daemon, false).await
}

/// [`check`], optionally ignoring the terminal claim: the chat that is about to delete its own
/// session holds that claim itself and releases it just before the delete.
pub(crate) async fn check_with(
    store: &Arc<Store>,
    id: &str,
    running_in_daemon: bool,
    own_terminal: bool,
) -> Result<SessionDeletePreview, Refusal> {
    let preview = {
        let store = Arc::clone(store);
        let id = id.to_string();
        tokio::task::spawn_blocking(move || store.session_delete_preview(&id))
            .await
            .map_err(|e| Refusal::Failed(e.to_string()))?
            .map_err(|e| Refusal::Failed(e.to_string()))?
    }
    .ok_or(Refusal::NotFound)?;
    if running_in_daemon {
        return Err(Refusal::Running);
    }
    if !own_terminal && crate::terminal_owner::terminal_owns_session(store, id).await {
        return Err(Refusal::OpenInTerminal { host: host_name() });
    }
    for target in &preview.targets {
        if let Some(path) = target.worktree.as_deref().filter(|p| Path::new(p).exists()) {
            return Err(Refusal::OwnsWorktree(path.to_string()));
        }
    }
    if preview.frozen {
        return Err(Refusal::Frozen);
    }
    Ok(preview)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Deleted {
    pub(crate) sessions: usize,
    pub(crate) files_removed: usize,
    pub(crate) wal_truncated: bool,
}

/// Check, then delete in `store` and remove the files keyed to the session.
pub(crate) async fn delete_local(
    store: &Arc<Store>,
    id: &str,
    running_in_daemon: bool,
) -> Result<Deleted, Refusal> {
    let preview = check(store, id, running_in_daemon).await?;
    if preview.running_subagents > 0 {
        // Detached subagent tasks live inside a hosted or open session, and the checks above
        // refused both. A row still `running` therefore has no task behind it (its process
        // died); settle it instead of blocking the delete on it forever. The current chat's own
        // live subagents were cancelled by `/delete` before it got here.
        let store = Arc::clone(store);
        let targets = preview.targets.clone();
        let _ = tokio::task::spawn_blocking(move || {
            for target in &targets {
                let _ = store.reconcile_running_detached_children(&target.id);
            }
        })
        .await;
    }
    let report = {
        let store = Arc::clone(store);
        let id = id.to_string();
        tokio::task::spawn_blocking(move || store.delete_session_report(&id))
            .await
            .map_err(|e| Refusal::Failed(e.to_string()))?
            .map_err(|e| Refusal::Failed(e.to_string()))?
    }
    .ok_or(Refusal::NotFound)?;
    Ok(Deleted {
        sessions: report.deleted.len(),
        files_removed: remove_session_files(&report.deleted),
        wal_truncated: report.wal_truncated,
    })
}

/// Delete through the running daemon when there is one, so its registry-aware checks apply;
/// otherwise in this process with the same checks.
pub(crate) async fn delete_anywhere(store: &Arc<Store>, id: &str) -> Result<Deleted, Refusal> {
    match daemon_for_this_store() {
        Some(state) => delete_via_daemon(&state, id).await,
        None => delete_local(store, id, false).await,
    }
}

/// The live daemon, but only when it owns the store this process opened: a daemon on another
/// store would answer 404 for a session it never had, or worse delete a same-id session in its own.
pub(crate) fn daemon_for_this_store() -> Option<crate::serve::ServeState> {
    let state = crate::serve::read_state()
        .ok()
        .flatten()
        .filter(|state| state.process_is_alive())?;
    daemon_owns_store(&state, &crate::cli::commands::replay::resolved_store_path()).then_some(state)
}

fn daemon_owns_store(state: &crate::serve::ServeState, ours: &Path) -> bool {
    let theirs = match state.store_path.as_deref() {
        Some(path) => std::path::PathBuf::from(path),
        // Daemons older than the `store_path` field always opened the release store.
        None => match forge_config::data_dir() {
            Some(dir) => dir.join("forge.db"),
            None => return false,
        },
    };
    std::fs::canonicalize(&theirs).unwrap_or(theirs) == ours
}

async fn delete_via_daemon(state: &crate::serve::ServeState, id: &str) -> Result<Deleted, Refusal> {
    let scheme = if state.exposure == "lan" {
        "https"
    } else {
        "http"
    };
    let url = format!(
        "{scheme}://127.0.0.1:{}/{}/api/sessions/{id}",
        state.port, state.token
    );
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(scheme == "https")
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| Refusal::Failed(e.to_string()))?;
    let response = client.delete(url).send().await.map_err(|e| {
        Refusal::Failed(format!(
            "could not reach the Forge daemon: {}",
            e.without_url()
        ))
    })?;
    let status = response.status();
    let body: serde_json::Value = response.json().await.unwrap_or_default();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(Refusal::NotFound);
    }
    if !status.is_success() {
        let why = body["error"].as_str().unwrap_or("the daemon refused");
        return Err(Refusal::Daemon(format!("the Forge daemon refused: {why}")));
    }
    Ok(Deleted {
        sessions: body["sessions"].as_u64().unwrap_or(1) as usize,
        files_removed: 0,
        wal_truncated: body["wal_truncated"].as_bool().unwrap_or(false),
    })
}

/// The confirmation dialog for a preview: what goes, what stays.
pub(crate) fn dialog_for(
    preview: &SessionDeletePreview,
    current: bool,
    refusal: Option<&Refusal>,
) -> forge_tui::SessionDeleteDialog {
    let mut facts = Vec::new();
    let count = |n: i64, what: &str| (n > 0).then(|| format!("{n} {what}"));
    facts.extend(count(preview.turns, "turn(s) of conversation"));
    facts.extend(count(preview.messages, "message(s)"));
    facts.extend(count(preview.tool_calls, "tool record(s)"));
    if preview.children > 0 {
        facts.push(format!("{} subagent session(s)", preview.children));
    }
    let folders = session_state_dirs(&preview.targets).len();
    if folders > 0 {
        facts.push(format!("{folders} rewind-snapshot / evidence folder(s)"));
    }
    if preview.running_subagents > 0 {
        facts.push(format!(
            "{} detached subagent(s) still marked running (stopped first)",
            preview.running_subagents
        ));
    }
    if preview.synced {
        facts.push("its copy on your other synced devices (deleted there too)".into());
    }
    facts.push(SCRUB_NOTE.into());
    let mut kept = Vec::new();
    if preview.memories_kept > 0 {
        kept.push(format!(
            "{} saved memor(ies) distilled from it are KEPT (/memory deletes them)",
            preview.memories_kept
        ));
    }
    if let Some(path) = preview.worktree.as_deref() {
        kept.push(format!("worktree {path} (merge or discard it yourself)"));
    }
    if current {
        facts.push("then this chat starts a fresh session".into());
    }
    forge_tui::SessionDeleteDialog {
        id: preview.id.clone(),
        title: preview.title.clone().unwrap_or_default(),
        current,
        facts,
        kept,
        refusal: refusal.map(|r| (r.message(), r.next_step().to_string())),
        typed: String::new(),
    }
}

/// Honest scrub wording shared by the dialog and the CLI.
pub(crate) const SCRUB_NOTE: &str = "its rows, scrubbed where SQLite allows; `forge sessions --delete <id> --compact-db` (Forge closed) removes every trace";

/// Per-session directories under a workspace's state folder: shadow snapshots for rewind, and tool
/// evidence. Bridge turns write nothing session-keyed besides these (see docs/features).
const SESSION_STATE_KINDS: &[&str] = &["checkpoints", "evidence"];

fn safe_component(id: &str) -> bool {
    !id.is_empty() && !id.contains(['/', '\\']) && id != "." && id != ".."
}

/// The existing per-session state folders of the deleted tree.
pub(crate) fn session_state_dirs(targets: &[DeleteTarget]) -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    for target in targets.iter().filter(|t| safe_component(&t.id)) {
        let workspaces = std::iter::once(target.cwd.as_str()).chain(target.worktree.as_deref());
        for workspace in workspaces {
            let workspace = Path::new(workspace);
            for kind in SESSION_STATE_KINDS {
                for base in [
                    forge_config::state_subdir(workspace, kind),
                    forge_config::legacy_state_subdir(workspace, kind),
                ] {
                    let dir = base.join(&target.id);
                    if dir.is_dir() && !dirs.contains(&dir) {
                        dirs.push(dir);
                    }
                }
            }
        }
    }
    dirs
}

/// Remove the on-disk state keyed to each deleted session. Returns how many directories went.
pub(crate) fn remove_session_files(targets: &[DeleteTarget]) -> usize {
    session_state_dirs(targets)
        .into_iter()
        .filter(|dir| std::fs::remove_dir_all(dir).is_ok())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::Role;

    fn store_with_session() -> (Arc<Store>, String) {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_session("/proj", "Smith").unwrap();
        store.add_message(&id, 0, Role::User, "hi", None).unwrap();
        (Arc::new(store), id)
    }

    #[tokio::test]
    async fn deletes_an_idle_session_and_its_snapshots() {
        let ws = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let id = store
            .create_session(ws.path().to_str().unwrap(), "Smith")
            .unwrap();
        let store = Arc::new(store);
        let snap = forge_config::state_subdir(ws.path(), "checkpoints").join(&id);
        std::fs::create_dir_all(snap.join("0")).unwrap();
        let done = delete_local(&store, &id, false).await.unwrap();
        assert_eq!(done.sessions, 1);
        assert!(done.files_removed >= 1);
        assert!(!snap.exists());
        assert!(!store.session_exists(&id).unwrap());
    }

    #[tokio::test]
    async fn refuses_a_daemon_running_session() {
        let (store, id) = store_with_session();
        let refusal = delete_local(&store, &id, true).await.unwrap_err();
        assert_eq!(refusal, Refusal::Running);
        assert!(store.session_exists(&id).unwrap());
    }

    #[tokio::test]
    async fn refuses_a_session_a_terminal_holds_and_says_where() {
        let (store, id) = store_with_session();
        store.set_session_local_live(&id, true).unwrap();
        let refusal = delete_local(&store, &id, false).await.unwrap_err();
        assert!(matches!(refusal, Refusal::OpenInTerminal { .. }));
        assert!(refusal.full().contains("run /delete in that terminal"));
        assert!(refusal.message().contains(&host_name()));
        assert!(store.session_exists(&id).unwrap());
    }

    #[tokio::test]
    async fn refuses_while_the_worktree_exists_and_allows_it_once_gone() {
        let (store, id) = store_with_session();
        let wt = tempfile::tempdir().unwrap();
        store
            .set_session_worktree(&id, wt.path().to_str().unwrap())
            .unwrap();
        let refusal = delete_local(&store, &id, false).await.unwrap_err();
        assert!(matches!(refusal, Refusal::OwnsWorktree(_)));
        let gone = wt.path().to_path_buf();
        drop(wt);
        assert!(!gone.exists());
        delete_local(&store, &id, false).await.unwrap();
    }

    #[tokio::test]
    async fn refuses_a_frozen_handoff_and_an_unknown_id() {
        let (store, id) = store_with_session();
        let missing = delete_local(&store, "nope", false).await.unwrap_err();
        assert_eq!(missing, Refusal::NotFound);
        assert_eq!(missing.status(), axum::http::StatusCode::NOT_FOUND);
        store.begin_source_handoff(&id, "capsule").unwrap();
        assert_eq!(
            delete_local(&store, &id, false).await.unwrap_err(),
            Refusal::Frozen
        );
        assert!(store.session_exists(&id).unwrap());
    }

    #[tokio::test]
    async fn a_running_subagent_row_never_blocks_a_delete_nor_the_current_chats_own_delete() {
        let (store, id) = store_with_session();
        let child = store.create_child_session("/proj", "Smith", &id).unwrap();
        store
            .create_detached_child(&child, &id, "worker", "m")
            .unwrap();
        // The chat that owns the session cancels its own subagents and then deletes it: the
        // checks must let it through while the row is still `running`.
        assert!(check_with(&store, &id, false, true).await.is_ok());
        // With no daemon driver and no terminal holding it, the row is stale: delete settles it.
        let done = delete_local(&store, &id, false).await.unwrap();
        assert_eq!(done.sessions, 2);
        assert!(!store.session_exists(&child).unwrap());
    }

    #[tokio::test]
    async fn a_daemon_driven_session_with_running_subagents_is_still_refused() {
        let (store, id) = store_with_session();
        let child = store.create_child_session("/proj", "Smith", &id).unwrap();
        store
            .create_detached_child(&child, &id, "worker", "m")
            .unwrap();
        assert_eq!(
            delete_local(&store, &id, true).await.unwrap_err(),
            Refusal::Running
        );
        assert!(store.session_exists(&id).unwrap());
    }

    fn daemon_state(store_path: Option<&str>) -> crate::serve::ServeState {
        crate::serve::ServeState {
            pid: std::process::id(),
            port: 1,
            exposure: "local".into(),
            base_url: "http://127.0.0.1:1/t".into(),
            token: "t".into(),
            started_at: 0,
            store_path: store_path.map(String::from),
        }
    }

    #[test]
    fn a_daemon_on_another_store_is_not_where_a_delete_goes() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.db");
        let copy = dir.path().join("copy.db");
        std::fs::write(&real, b"").unwrap();
        std::fs::write(&copy, b"").unwrap();
        let real = std::fs::canonicalize(real).unwrap();
        let copy = std::fs::canonicalize(copy).unwrap();
        let daemon = daemon_state(Some(real.to_str().unwrap()));
        assert!(daemon_owns_store(&daemon, &real));
        assert!(!daemon_owns_store(&daemon, &copy));
    }

    #[test]
    fn path_components_are_validated() {
        assert!(!safe_component("../etc"));
        assert!(!safe_component(""));
        assert!(safe_component("0198abcd"));
    }

    #[test]
    fn dialog_lists_only_nonzero_scope_and_says_what_is_kept() {
        let preview = SessionDeletePreview {
            id: "0123456789".into(),
            title: None,
            turns: 2,
            messages: 5,
            tool_calls: 0,
            children: 0,
            worktree: None,
            synced: true,
            frozen: false,
            running_subagents: 0,
            memories_kept: 3,
            targets: Vec::new(),
        };
        let d = dialog_for(&preview, false, None);
        let facts = d.facts.join("\n");
        assert!(facts.contains("2 turn(s)") && facts.contains("5 message(s)"));
        assert!(!facts.contains("tool record") && !facts.contains("subagent"));
        assert!(!facts.contains("folder(s)"));
        assert!(facts.contains("deleted there too"));
        assert!(facts.contains("removes every trace") && !facts.contains("overwritten"));
        assert!(d.kept[0].contains("3 saved memor") && d.kept[0].contains("KEPT"));
    }
}
