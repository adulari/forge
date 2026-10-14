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
    pub(crate) fn worktree_tool(
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
                PermissionDecision::Ask => !matches!(
                    permission::confirm_call(
                        self.presenter.as_mut(),
                        &call.name,
                        &call.args,
                        side_effect
                    ),
                    forge_types::ConfirmOutcome::Deny
                ),
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
        let outcome = worktree_ops::execute(
            &request,
            &Context {
                current: &current,
                home: home.as_deref(),
                daemon_owned: daemon_owned.as_deref(),
                confine: confine.as_deref(),
            },
        );
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
        }
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

/// The file holding a checkout's `HEAD`: `.git/HEAD`, or for a linked worktree (whose `.git` is
/// a `gitdir:` pointer file) the `HEAD` in its private git directory.
pub(crate) fn git_head_file(root: &std::path::Path) -> std::path::PathBuf {
    let dot_git = root.join(".git");
    if dot_git.is_file() {
        if let Some(gitdir) = std::fs::read_to_string(&dot_git)
            .ok()
            .as_deref()
            .and_then(|t| t.lines().find_map(|l| l.strip_prefix("gitdir:")))
        {
            return root.join(gitdir.trim()).join("HEAD");
        }
    }
    dot_git.join("HEAD")
}
