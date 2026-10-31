//! `WorktreeCreate`, `WorktreeRemove` and `CwdChanged` hooks on the `worktree` tool (Claude-Code
//! parity, docs/features/hooks.md).
//!
//! A `WorktreeCreate` hook REPLACES `git worktree add`: it gets `{"name"}`, makes the working copy
//! however it likes (SVN, Perforce, a plain copy) and prints the directory as its last stdout line
//! (an HTTP hook answers `hookSpecificOutput.worktreePath`). The session then switches into it. A
//! worktree made that way is remembered for the life of the process, and removing it (`exit` with
//! `remove: true`, or `remove`) runs the `WorktreeRemove` hooks with `{"worktree_path"}` instead of
//! git; with no such hook Forge tries `git worktree remove --force` and leaves anything git does
//! not know on disk. `CwdChanged` fires whenever the tool moved the session to another directory.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use forge_config::HookEvent;

use crate::hooks_events::{run_event_hooks, EventMatch};
use crate::worktree_ops::{slug, Action, Outcome, Request};
use crate::{PresenterEvent, Session};

fn hook_made() -> &'static Mutex<HashSet<PathBuf>> {
    static MADE: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    MADE.get_or_init(Default::default)
}

fn is_hook_made(path: &Path) -> bool {
    hook_made()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(path)
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn failed(text: impl Into<String>) -> Outcome {
    Outcome {
        ok: false,
        text: text.into(),
        switch_to: None,
        carried: false,
    }
}

fn done(text: impl Into<String>, switch_to: Option<PathBuf>) -> Outcome {
    Outcome {
        ok: true,
        text: text.into(),
        switch_to,
        carried: false,
    }
}

fn inside(confine: Option<&[PathBuf]>, path: &Path) -> bool {
    confine.is_none_or(|roots| roots.iter().any(|r| path.starts_with(canonical(r))))
}

/// Where a request's `path` / `branch` / `name` points, resolved against the current root.
fn target_path(request: &Request, current: &Path) -> Option<PathBuf> {
    let raw = request
        .path
        .as_deref()
        .or(request.branch.as_deref())
        .or(request.name.as_deref())?;
    let path = Path::new(raw);
    Some(canonical(&if path.is_absolute() {
        path.to_path_buf()
    } else {
        current.join(path)
    }))
}

impl Session {
    pub(crate) fn has_hooks_for(&self, event: HookEvent) -> bool {
        self.config.hooks.iter().any(|h| h.event == event)
    }

    /// The `worktree` tool's hook path: `Some(outcome)` when a hook handles this request (a
    /// configured `WorktreeCreate`, or a worktree one made), `None` to use git as usual.
    pub(crate) async fn worktree_via_hooks(
        &mut self,
        request: &Request,
        current: &Path,
        home: Option<&Path>,
        confine: Option<&[PathBuf]>,
    ) -> Option<Outcome> {
        let current = canonical(current);
        match request.action {
            Action::Create if self.has_hooks_for(HookEvent::WorktreeCreate) => {
                Some(self.hook_create(request, &current, confine).await)
            }
            Action::Exit if is_hook_made(&current) => {
                let Some(home) = home.map(canonical) else {
                    return Some(failed(
                        "worktree exit: this session started inside a hook-made worktree, so \
                         there is no checkout to return to",
                    ));
                };
                let mut text = format!(
                    "left {} and returned to {}",
                    current.display(),
                    home.display()
                );
                if request.remove {
                    let removed = self.remove_hook_worktree(&current, &home).await;
                    text.push_str(&format!("; {}", removed.unwrap_or_else(|e| e)));
                }
                Some(done(text, Some(home)))
            }
            Action::Enter | Action::Remove => {
                let path = target_path(request, &current).filter(|p| is_hook_made(p))?;
                if request.action == Action::Enter {
                    return Some(if !inside(confine, &path) {
                        failed(format!(
                            "worktree enter: {} is outside the workspace and `confine_to_workspace` \
                             is on",
                            path.display()
                        ))
                    } else if path == current {
                        done(format!("already working in {}", path.display()), None)
                    } else {
                        done(format!("entered worktree {}", path.display()), Some(path))
                    });
                }
                if path == current {
                    return Some(failed(
                        "worktree remove: this session is working in that worktree; use `exit` \
                         with `remove: true` to leave and remove it in one step",
                    ));
                }
                Some(match self.remove_hook_worktree(&path, &current).await {
                    Ok(text) => done(text, None),
                    Err(text) => failed(text),
                })
            }
            _ => None,
        }
    }

    async fn hook_create(
        &mut self,
        request: &Request,
        current: &Path,
        confine: Option<&[PathBuf]>,
    ) -> Outcome {
        let name = request
            .name
            .clone()
            .or_else(|| {
                request
                    .branch
                    .as_deref()
                    .map(|b| b.rsplit('/').next().unwrap_or(b).to_string())
            })
            .map(|n| slug(&n))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("forge-{}", &forge_types::new_id()[..8]));
        let out = run_event_hooks(
            &self.config.hooks,
            HookEvent::WorktreeCreate,
            &self.id,
            EventMatch::Any,
            serde_json::json!({ "name": name, "cwd": current.display().to_string() }),
        )
        .await;
        if let Some(why) = out.failures.first().or(out.blocked.as_ref()) {
            return failed(format!(
                "worktree create: the WorktreeCreate hook failed: {why}"
            ));
        }
        let Some(raw) = out.worktree_path else {
            return failed(
                "worktree create: the WorktreeCreate hook printed no directory (it must print the \
                 path as its last stdout line)",
            );
        };
        let given = Path::new(&raw);
        if given.is_absolute()
            && given.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::CurDir | std::path::Component::ParentDir
                )
            })
        {
            return failed(format!(
                "worktree create: the hook returned {raw}, which has `.` or `..` segments; return \
                 a normalized absolute path"
            ));
        }
        let path = canonical(&current.join(given));
        if !path.is_dir() {
            return failed(format!(
                "worktree create: the hook returned {}, which is not a directory",
                path.display()
            ));
        }
        if !inside(confine, &path) {
            return failed(format!(
                "worktree create: {} is outside the workspace and `confine_to_workspace` is on",
                path.display()
            ));
        }
        hook_made()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(path.clone());
        done(
            format!(
                "created worktree {} (made by the WorktreeCreate hook)",
                path.display()
            ),
            Some(path),
        )
    }

    /// Remove a worktree a `WorktreeCreate` hook made: through the `WorktreeRemove` hooks when
    /// configured, else as `git worktree remove --force` would. A removal that leaves the directory
    /// behind fails and the worktree stays known.
    async fn remove_hook_worktree(
        &mut self,
        path: &Path,
        run_from: &Path,
    ) -> Result<String, String> {
        let shown = path.display().to_string();
        if self.has_hooks_for(HookEvent::WorktreeRemove) {
            let out = run_event_hooks(
                &self.config.hooks,
                HookEvent::WorktreeRemove,
                &self.id,
                EventMatch::Any,
                serde_json::json!({ "worktree_path": shown, "cwd": run_from.display().to_string() }),
            )
            .await;
            if let Some(why) = out.failures.first().filter(|_| path.exists()) {
                return Err(format!(
                    "the WorktreeRemove hook failed and {shown} is still on disk: {why}"
                ));
            }
        } else {
            let _ = std::process::Command::new("git")
                .arg("-C")
                .arg(run_from)
                .args(["worktree", "remove", "--force"])
                .arg(path)
                .output();
            if path.exists() {
                return Err(format!(
                    "{shown} was made by a WorktreeCreate hook and no WorktreeRemove hook is \
                     configured; git could not remove it, so it was left on disk"
                ));
            }
        }
        hook_made()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(path);
        Ok(format!("removed worktree {shown}"))
    }

    /// `CwdChanged` after the session moved from `old` to its current root. Observe-only: a
    /// `systemMessage` is shown, failures are noted.
    pub(crate) async fn fire_cwd_changed(&mut self, old: &Path) {
        let new = self.workspace.root().to_path_buf();
        if new == old || !self.has_hooks_for(HookEvent::CwdChanged) {
            return;
        }
        let out = run_event_hooks(
            &self.config.hooks,
            HookEvent::CwdChanged,
            &self.id,
            EventMatch::Any,
            serde_json::json!({
                "old_cwd": old.display().to_string(),
                "new_cwd": new.display().to_string(),
                "cwd": new.display().to_string(),
            }),
        )
        .await;
        for note in out.notes {
            self.presenter
                .emit(PresenterEvent::Warning(format!("⎇ hook: {note}")));
        }
        for failure in out.failures {
            self.presenter
                .emit(PresenterEvent::Warning(format!("⎇ hook error: {failure}")));
        }
    }
}
