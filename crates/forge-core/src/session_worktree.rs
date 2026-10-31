//! The session half of the `worktree` tool (docs/features/session-worktrees.md): gate the call,
//! run it through [`worktree_ops`], and move every piece of per-session state that is keyed to the
//! workspace root along with the session.
//!
//! What moves on a switch: the tool registry's workspace binding (file tools, relative paths, the
//! shell's default cwd, evidence capture), the Lattice index and watcher, the project context and
//! cached branch, the commit-discipline tracker, the session's stored `cwd` (resume and a
//! daemon restart reopen it where it now works), and the recorded host fingerprint (so a resume
//! inside the worktree is not mistaken for a host move, docs/features/session-host-move.md). What deliberately stays: the checkpoint root, so
//! `/undo` still has the snapshots after the worktree is deleted, the loaded config, and LSP
//! (it keys servers by each file's own project root already).

use super::*;
use crate::worktree_ops::{self, Context, Request};

impl Session {
    pub(crate) async fn worktree_tool(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        let request = match Request::parse(&call.args) {
            Ok(request) => request,
            Err(error) => {
                self.store
                    .record_tool_call(msg_id, &call.name, &args_json, &error, "n/a", "error")?;
                return Ok(error);
            }
        };
        let side_effect = request.side_effect();
        let allowed =
            match permission::decide(self.mode, side_effect, &call.name, &call.args, &self.rules) {
                PermissionDecision::Allow => true,
                PermissionDecision::Deny => false,
                PermissionDecision::Ask => permission::confirm_call(
                    self.presenter.as_mut(),
                    &call.name,
                    &call.args,
                    side_effect,
                )
                .outcome
                .allows(),
            };
        if !allowed {
            let result = format!(
                "permission denied: `worktree {:?}` is not allowed in the current temper",
                request.action
            )
            .to_lowercase();
            self.store
                .record_tool_call(msg_id, &call.name, &args_json, &result, "denied", "error")?;
            return Ok(result);
        }

        let (result, ok) = self.run_worktree_request(&request).await;
        self.store.record_tool_call(
            msg_id,
            &call.name,
            &args_json,
            &result,
            "allowed",
            if ok { "ok" } else { "error" },
        )?;
        Ok(result)
    }

    /// Run a parsed request with no permission check and move the session if it asks to switch.
    /// The model's tool gates first ([`Self::worktree_tool`]); a user's own `/worktree` command
    /// is its own consent and calls this directly. Returns the text and whether it succeeded.
    pub async fn run_worktree_request(&mut self, request: &Request) -> (String, bool) {
        let current = self.workspace.root().to_path_buf();
        let home = self.worktree_home.clone();
        let daemon_owned = self
            .store
            .session_worktree(&self.id)
            .ok()
            .flatten()
            .map(std::path::PathBuf::from);
        let confine = self.config.tools.confine_to_workspace.then(|| {
            let mut roots = vec![home.clone().unwrap_or_else(|| current.clone())];
            roots.extend(
                self.config
                    .tools
                    .extra_roots
                    .iter()
                    .map(std::path::PathBuf::from)
                    .filter(|p| p.is_absolute()),
            );
            roots
        });
        // A `WorktreeCreate` hook replaces git, and a worktree one made is removed by hooks.
        let hooked = self
            .worktree_via_hooks(request, &current, home.as_deref(), confine.as_deref())
            .await;
        let outcome = match hooked {
            Some(outcome) => outcome,
            None => {
                // Git, the `setup`/`cleanup` commands and the ignored-file copy can run for
                // minutes; keep them off the async workers (and out of the session's way).
                let settings = self.config.worktree.clone();
                run_off_thread(
                    request.clone(),
                    current.clone(),
                    home.clone(),
                    daemon_owned,
                    confine,
                    settings,
                )
                .await
            }
        };
        let mut result = outcome.text;
        let mut ok = outcome.ok;
        if let Some(root) = outcome.switch_to {
            match self.switch_workspace(&root) {
                Ok(()) => result.push_str(&format!(
                    "\nThis session now works in {} — relative paths, file tools and the shell's \
                     default cwd resolve there; do not `cd` elsewhere.",
                    self.workspace.root().display()
                )),
                Err(error) => {
                    ok = false;
                    result.push_str(&format!("\nbut switching the session failed: {error}"));
                }
            }
            self.fire_cwd_changed(&current).await;
        }
        (result, ok)
    }

    /// Move the session to a different project directory (`/home`, `/project`). Unlike a
    /// worktree switch there is no home checkout to come back to, and the checkpoint root
    /// follows the new project.
    pub fn switch_project(&mut self, root: &std::path::Path) -> Result<(), CoreError> {
        self.transition_workspace(WorkspaceContext::new(root)?)?;
        let display = self.workspace.display();
        if let Err(error) = self.store.set_session_cwd(&self.id, &display) {
            tracing::warn!(session_id = %self.id, %error, "session cwd not persisted after switch");
        }
        crate::host_move::record_workspace_switch(&self.store, &self.id, self.workspace.root());
        self.presenter.emit(PresenterEvent::Progress(format!(
            "now working in {display}"
        )));
        Ok(())
    }

    /// Move this live session to another root of the same repository: every root-keyed piece of
    /// session state follows, the checkpoint root does not (see the module docs).
    pub fn switch_workspace(&mut self, root: &std::path::Path) -> Result<(), CoreError> {
        let previous = self.workspace.root().to_path_buf();
        let home = self.worktree_home.take().unwrap_or(previous);
        let checkpoint_root = self.checkpoint_root.clone();
        self.transition_workspace(WorkspaceContext::new(root)?)?;
        self.checkpoint_root = checkpoint_root;
        self.worktree_home = (home != self.workspace.root()).then_some(home);
        let display = self.workspace.display();
        if let Err(error) = self.store.set_session_cwd(&self.id, &display) {
            tracing::warn!(session_id = %self.id, %error, "session cwd not persisted after switch");
        }
        crate::host_move::record_workspace_switch(&self.store, &self.id, self.workspace.root());
        let branch = self
            .cached_git_branch
            .as_deref()
            .map(|b| format!(" [{b}]"))
            .unwrap_or_default();
        self.presenter.emit(PresenterEvent::Progress(format!(
            "⎇ now working in {display}{branch}"
        )));
        Ok(())
    }

    /// The session's `cwd` as the store has it right now.
    pub(crate) fn stored_cwd(&self) -> Option<String> {
        self.store.session_cwd(&self.id).ok().flatten()
    }

    /// A CLI bridge runs the `worktree` tool inside `forge mcp-serve`, which can only record the
    /// new root in the store. Follow it once the bridge yields — only when the stored `cwd`
    /// changed during that call (`before`) — so the next request, and the next bridge spawn,
    /// run where the model now works. True when the session moved.
    pub(crate) fn adopt_bridge_workspace(&mut self, before: Option<String>) -> bool {
        let after = self.stored_cwd();
        let Some(moved) = after.filter(|cwd| Some(cwd) != before.as_ref()) else {
            return false;
        };
        let Ok(root) = std::path::Path::new(&moved).canonicalize() else {
            return false;
        };
        if root == self.workspace.root() || !root.is_dir() {
            return false;
        }
        self.switch_workspace(&root).is_ok()
    }

    /// Where a runtime "always allow" answer is persisted: the session's home checkout, never a
    /// worktree it is visiting (that config file would dirty the worktree and die with it).
    pub(crate) fn rules_root(&self) -> &std::path::Path {
        self.worktree_home
            .as_deref()
            .unwrap_or_else(|| self.workspace.root())
    }
}

/// The file holding a checkout's `HEAD`: `.git/HEAD`, or for a linked worktree or submodule
/// (whose `.git` is a `gitdir:` pointer file) the `HEAD` in its private git directory. Walks up
/// from `root`, so a workspace that is a subdirectory of the repository resolves it too; with no
/// repository above, the path under `root` (which does not exist) is returned.
pub(crate) fn git_head_file(root: &std::path::Path) -> std::path::PathBuf {
    for top in root.ancestors() {
        let dot_git = top.join(".git");
        if dot_git.is_file() {
            if let Some(gitdir) = std::fs::read_to_string(&dot_git)
                .ok()
                .as_deref()
                .and_then(|t| t.lines().find_map(|l| l.strip_prefix("gitdir:")))
            {
                return top.join(gitdir.trim()).join("HEAD");
            }
        } else if dot_git.is_dir() {
            return dot_git.join("HEAD");
        }
    }
    root.join(".git").join("HEAD")
}

#[derive(Default)]
struct Handoff {
    outcome: Option<worktree_ops::Outcome>,
    abandoned: bool,
}

/// What to do with a worktree created for a caller that no longer exists. Only a clean, freshly
/// created one with nothing carried into it is removed, and only with `branch -d`: anything the
/// user's changes, or the agent, put there is theirs, so it stays and the log says where.
pub(crate) fn discard_orphan(created: &worktree_ops::Outcome, current: &std::path::Path) {
    let Some(path) = created.switch_to.as_deref() else {
        return;
    };
    let dirty = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["status", "--porcelain"])
        .output()
        .map(|o| !o.status.success() || !o.stdout.is_empty())
        .unwrap_or(true);
    if created.carried || dirty {
        tracing::warn!(
            worktree = %path.display(),
            "a cancelled worktree request left {} in place: it holds your uncommitted changes",
            path.display()
        );
        return;
    }
    let request = Request {
        action: worktree_ops::Action::Remove,
        name: None,
        branch: None,
        base: None,
        path: Some(path.display().to_string()),
        remove: false,
        force: false,
        keep_branch: false,
        keep_unmerged: true,
        carry_dirty: false,
    };
    let _ = worktree_ops::execute(
        &request,
        &Context {
            current,
            home: None,
            daemon_owned: None,
            confine: None,
            settings: None,
        },
    );
}

/// Dropped while the blocking operation is still pending (the turn was cancelled): whatever it
/// creates would be a worktree nobody switched to, so it is removed instead of left behind.
struct OrphanGuard {
    handoff: std::sync::Arc<std::sync::Mutex<Handoff>>,
    current: std::path::PathBuf,
    creates: bool,
    armed: bool,
}

impl Drop for OrphanGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut handoff = self.handoff.lock().unwrap_or_else(|e| e.into_inner());
        handoff.abandoned = true;
        if let Some(created) = handoff.outcome.take().filter(|_| self.creates) {
            let current = self.current.clone();
            std::thread::spawn(move || discard_orphan(&created, &current));
        }
    }
}

pub(crate) async fn run_off_thread(
    request: Request,
    current: std::path::PathBuf,
    home: Option<std::path::PathBuf>,
    daemon_owned: Option<std::path::PathBuf>,
    confine: Option<Vec<std::path::PathBuf>>,
    settings: forge_config::worktree::WorktreeConfig,
) -> worktree_ops::Outcome {
    let handoff = std::sync::Arc::new(std::sync::Mutex::new(Handoff::default()));
    let creates = request.action == worktree_ops::Action::Create;
    let mut guard = OrphanGuard {
        handoff: handoff.clone(),
        current: current.clone(),
        creates,
        armed: true,
    };
    let task_handoff = handoff.clone();
    let task_current = current.clone();
    let joined = tokio::task::spawn_blocking(move || {
        let outcome = worktree_ops::execute(
            &request,
            &Context {
                current: &current,
                home: home.as_deref(),
                daemon_owned: daemon_owned.as_deref(),
                confine: confine.as_deref(),
                settings: Some(&settings),
            },
        );
        let mut handoff = task_handoff.lock().unwrap_or_else(|e| e.into_inner());
        if handoff.abandoned {
            if creates {
                discard_orphan(&outcome, &task_current);
            }
        } else {
            handoff.outcome = Some(outcome);
        }
    })
    .await;
    let outcome = handoff
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .outcome
        .take();
    guard.armed = false;
    match (joined, outcome) {
        (Ok(()), Some(outcome)) => outcome,
        (Err(e), _) => worktree_ops::Outcome {
            ok: false,
            text: format!("worktree: the operation task failed: {e}"),
            switch_to: None,
            carried: false,
        },
        (Ok(()), None) => worktree_ops::Outcome {
            ok: false,
            text: "worktree: the operation produced no result".into(),
            switch_to: None,
            carried: false,
        },
    }
}
