//! `worktree` on the CLI bridge. The git work is `forge_core::worktree_ops`, shared with the
//! direct path. This process can only move ITSELF: it rebinds its own tool registry and cwd so
//! the rest of the bridge turn works at the new root, and records that root as the parent
//! session's `cwd` in the store; the parent follows it once the bridge yields
//! (`Session::adopt_bridge_workspace`), and its next bridge spawn starts there.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use forge_core::worktree_ops::{self, Context, Request};
use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool};
use serde_json::Value;

use super::{gate, permission, ForgeMcp};

pub(super) fn tool() -> Tool {
    let spec = worktree_ops::worktree_spec();
    let schema: JsonObject = spec.schema.as_object().cloned().unwrap_or_default();
    Tool::new(spec.name, spec.description, Arc::new(schema))
}

impl ForgeMcp {
    pub(super) async fn handle_worktree(&self, args: &Value) -> CallToolResult {
        let request = match Request::parse(args) {
            Ok(request) => request,
            Err(error) => return CallToolResult::error(vec![ContentBlock::text(error)]),
        };
        let mode = self.effective_mode();
        let side_effect = request.side_effect();
        let decision = permission::decide(
            mode,
            side_effect,
            worktree_ops::WORKTREE_TOOL,
            args,
            &self.rules,
        );
        if let Err(refusal) = gate(decision, worktree_ops::WORKTREE_TOOL, side_effect, mode).await {
            return CallToolResult::error(vec![ContentBlock::text(refusal)]);
        }
        let Ok(current) = std::env::current_dir().and_then(|d| d.canonicalize()) else {
            return CallToolResult::error(vec![ContentBlock::text(
                "worktree: the bridge workspace is not readable",
            )]);
        };
        let session = std::env::var(forge_core::snapshot::ENV_SESSION).ok();
        let daemon_owned = session
            .as_deref()
            .and_then(|id| self.tasks_store.session_worktree(id).ok().flatten())
            .map(PathBuf::from);
        let confine = self.config.tools.confine_to_workspace.then(|| {
            let spawned = std::env::var(forge_provider::BRIDGE_WORKSPACE_ENV)
                .map(PathBuf::from)
                .unwrap_or_else(|_| current.clone());
            let mut roots = vec![spawned];
            roots.extend(
                self.config
                    .tools
                    .extra_roots
                    .iter()
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute()),
            );
            roots
        });
        let outcome = worktree_ops::execute(
            &request,
            &Context {
                current: &current,
                home: None,
                daemon_owned: daemon_owned.as_deref(),
                confine: confine.as_deref(),
            },
        );
        let mut text = outcome.text;
        let mut ok = outcome.ok;
        if let Some(root) = outcome.switch_to {
            match self.move_bridge(&root, session.as_deref()) {
                Ok(()) => text.push_str(&format!(
                    "\nThis session now works in {} — relative paths, file tools and the shell's \
                     default cwd resolve there; do not `cd` elsewhere.",
                    root.display()
                )),
                Err(error) => {
                    ok = false;
                    text.push_str(&format!("\nbut switching the session failed: {error}"));
                }
            }
        }
        let content = vec![ContentBlock::text(text)];
        if ok {
            CallToolResult::success(content)
        } else {
            CallToolResult::error(content)
        }
    }

    fn move_bridge(&self, root: &Path, session: Option<&str>) -> Result<(), String> {
        self.registry
            .rebind_workspace(root)
            .map_err(|e| e.to_string())?;
        std::env::set_current_dir(root).map_err(|e| e.to_string())?;
        if let Some(id) = session {
            self.tasks_store
                .set_session_cwd(id, &root.display().to_string())
                .map_err(|e| format!("recording the new root for the parent session: {e}"))?;
        }
        Ok(())
    }
}
