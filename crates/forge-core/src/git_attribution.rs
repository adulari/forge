//! The `prepare-commit-msg` hook that stamps `Co-Authored-By: Forge (<model>)` on commits, and the
//! `forge-model` file it reads. Lives in core (not the CLI) so every surface that tells the model
//! the trailer is added automatically (TUI, headless `forge run`, daemon sessions) actually
//! installs it: the system prompt makes that promise whenever `[git] coauthor = true`.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{PresenterEvent, Session};

const HOOK_NAME: &str = "prepare-commit-msg";
/// A hook that was in the way is moved here and run first by the Forge hook, never discarded.
const CHAINED_NAME: &str = "prepare-commit-msg.pre-forge";
const MARKER: &str = "Installed by Forge";

/// The hook script. Strips Claude/Codex/Anthropic co-author lines in any casing (and Forge's own,
/// so `--amend` does not stack trailers), runs a chained pre-existing hook first, and leaves an
/// empty message empty so git aborts instead of committing a bare trailer.
pub const COMMIT_HOOK: &str = r#"#!/bin/sh
# Installed by Forge — rewrites commit co-author attribution.
# Strips Claude/Codex/Anthropic co-author lines; adds Forge (with the active model) as co-author.
COMMIT_MSG_FILE="$1"
HOOK_DIR=$(dirname "$0")
if [ -x "$HOOK_DIR/prepare-commit-msg.pre-forge" ]; then
  "$HOOK_DIR/prepare-commit-msg.pre-forge" "$@" || exit $?
fi
if ! grep -Eqv '^[[:space:]]*(#([[:space:]]|$)|$)' "$COMMIT_MSG_FILE"; then
  exit 0
fi
GIT_DIR=$(git rev-parse --git-dir 2>/dev/null || echo .git)
MODEL=$(cat "$GIT_DIR/forge-model" 2>/dev/null)
filtered=$(grep -Eiv '^[[:space:]]*co-authored-by:.*(claude|codex|anthrop|forge@adulari\.dev)' "$COMMIT_MSG_FILE" || true)
if [ -n "$MODEL" ]; then
  printf '%s\n\nCo-Authored-By: Forge (%s) <forge@adulari.dev>\n' "$filtered" "$MODEL" > "$COMMIT_MSG_FILE"
else
  printf '%s\n\nCo-Authored-By: Forge <forge@adulari.dev>\n' "$filtered" > "$COMMIT_MSG_FILE"
fi
"#;

/// What [`install_commit_hook`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum HookInstall {
    Installed(PathBuf),
    /// Installed, and the hook that was already there is kept and runs first.
    Chained {
        hook: PathBuf,
        kept: PathBuf,
    },
    /// Deliberately not installed; the string says why and what to do instead.
    Skipped(String),
    NotARepo,
}

fn git_output(cwd: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// This checkout's own git dir: the real directory for a plain repo, `.git/worktrees/<name>` for
/// a linked worktree, `.git/modules/<name>` for a submodule (their `.git` is a pointer FILE, so
/// joining `.git` onto the checkout root is wrong there).
pub fn git_dir(cwd: &Path) -> Option<PathBuf> {
    let dir = PathBuf::from(git_output(cwd, &["rev-parse", "--absolute-git-dir"])?);
    Some(dir)
}

/// Where git actually looks for hooks (`--git-path hooks` follows linked worktrees, which share
/// the main repo's hooks, and `core.hooksPath`).
fn hooks_dir(cwd: &Path) -> Option<PathBuf> {
    let raw = PathBuf::from(git_output(cwd, &["rev-parse", "--git-path", "hooks"])?);
    Some(if raw.is_absolute() {
        raw
    } else {
        cwd.join(raw)
    })
}

fn is_forge_hook(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|body| body.contains(MARKER))
}

/// Install (or update) the model-aware hook for the repository containing `cwd`.
pub fn install_commit_hook(cwd: &Path) -> Result<HookInstall, String> {
    let Some(_) = git_dir(cwd) else {
        return Ok(HookInstall::NotARepo);
    };
    if let Some(path) = git_output(cwd, &["config", "--get", "core.hooksPath"]) {
        return Ok(HookInstall::Skipped(format!(
            "core.hooksPath is set to '{path}', so git ignores .git/hooks and Forge will not write \
             into a shared hooks directory. Call the Forge hook from your own prepare-commit-msg \
             (`forge git hook` prints it), or unset core.hooksPath."
        )));
    }
    let hooks = hooks_dir(cwd).ok_or("could not resolve the git hooks directory")?;
    std::fs::create_dir_all(&hooks).map_err(|e| format!("creating {}: {e}", hooks.display()))?;
    let hook = hooks.join(HOOK_NAME);
    let mut kept = None;
    if hook.exists() && !is_forge_hook(&hook) {
        let backup = hooks.join(CHAINED_NAME);
        if backup.exists() {
            return Ok(HookInstall::Skipped(format!(
                "{} is a custom hook and {} already exists; Forge left both alone",
                hook.display(),
                backup.display()
            )));
        }
        std::fs::rename(&hook, &backup)
            .map_err(|e| format!("keeping the existing hook at {}: {e}", backup.display()))?;
        kept = Some(backup);
    }
    std::fs::write(&hook, COMMIT_HOOK).map_err(|e| format!("writing {}: {e}", hook.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&hook)
            .map_err(|e| e.to_string())?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&hook, perms).map_err(|e| e.to_string())?;
    }
    Ok(match kept {
        Some(kept) => HookInstall::Chained { hook, kept },
        None => HookInstall::Installed(hook),
    })
}

/// Record `model` where the hook reads it (`<git dir>/forge-model`). Best-effort.
pub fn write_active_model(git_dir: &Path, model: &str) {
    let _ = std::fs::write(git_dir.join("forge-model"), model);
}

/// Per-session bookkeeping so the hook is installed once per workspace and the model file is
/// rewritten only when the model changes.
#[derive(Default)]
pub(crate) struct AttributionState {
    workspace: Option<PathBuf>,
    git_dir: Option<PathBuf>,
    model: String,
}

impl AttributionState {
    /// Make attribution current for `workspace` running `model`. Returns the install outcome only
    /// when this call (re)installed the hook, so the caller can surface a problem once.
    pub(crate) fn sync(&mut self, workspace: &Path, model: &str) -> Option<HookInstall> {
        let mut installed = None;
        if self.workspace.as_deref() != Some(workspace) {
            self.workspace = Some(workspace.to_path_buf());
            self.git_dir = git_dir(workspace);
            self.model.clear();
            installed = Some(install_commit_hook(workspace).unwrap_or_else(|e| {
                HookInstall::Skipped(format!("commit hook not installed: {e}"))
            }));
        }
        if let Some(dir) = &self.git_dir {
            if !model.is_empty() && self.model != model {
                write_active_model(dir, model);
                self.model = model.to_string();
            }
        }
        installed
    }
}

impl Session {
    /// Before a model request: with `[git] coauthor = true`, make sure the commit hook is installed
    /// for the workspace the session is bound to and that `forge-model` names the model about to
    /// run, so a commit made this turn is attributed to it. Cheap when nothing changed.
    pub(crate) fn sync_commit_attribution(&mut self, model: &str) {
        if !self.config.git.coauthor {
            return;
        }
        let root = self.workspace.root().to_path_buf();
        let warning = match self.attribution.sync(&root, model) {
            Some(HookInstall::Skipped(why)) => Some(why),
            Some(HookInstall::Chained { kept, .. }) => Some(format!(
                "kept your existing prepare-commit-msg hook as {} and run it before Forge's",
                kept.display()
            )),
            _ => None,
        };
        if let Some(warning) = warning {
            self.presenter.emit(PresenterEvent::Warning(warning));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(dir.path().join("a.txt"), "a\n").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-qm", "init"]);
        dir
    }

    fn message_of_head(dir: &Path) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["log", "-1", "--format=%B"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    }

    fn commit(dir: &Path, message: &str) {
        std::fs::write(dir.join("a.txt"), format!("{message}\n")).unwrap();
        git(dir, &["commit", "-qam", message]);
    }

    #[test]
    fn hook_strips_any_casing_and_does_not_stack_trailers_on_amend() {
        let dir = repo();
        assert!(matches!(
            install_commit_hook(dir.path()).unwrap(),
            HookInstall::Installed(_)
        ));
        write_active_model(&git_dir(dir.path()).unwrap(), "m-1");
        commit(
            dir.path(),
            "feat: x\n\nCo-authored-by: Claude <noreply@anthropic.com>\nCo-authored-by: ChatGPT Codex Bot <x@y>",
        );
        let msg = message_of_head(dir.path());
        assert!(!msg.to_lowercase().contains("claude"), "{msg}");
        assert!(!msg.to_lowercase().contains("codex"), "{msg}");
        git(dir.path(), &["commit", "--amend", "--no-edit", "-q"]);
        git(dir.path(), &["commit", "--amend", "--no-edit", "-q"]);
        let msg = message_of_head(dir.path());
        assert_eq!(
            msg.matches("Co-Authored-By: Forge (m-1)").count(),
            1,
            "{msg}"
        );
    }

    #[test]
    fn an_empty_message_stays_empty_instead_of_becoming_a_trailer() {
        let dir = repo();
        install_commit_hook(dir.path()).unwrap();
        std::fs::write(dir.path().join("msg"), "\n").unwrap();
        let status = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["commit", "--allow-empty", "-q", "-F", "msg"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .unwrap();
        assert!(!status.success(), "an empty message must abort the commit");
        assert!(!message_of_head(dir.path()).contains("Forge"));
    }

    #[test]
    fn an_existing_hook_is_kept_and_still_runs() {
        let dir = repo();
        let hooks = dir.path().join(".git/hooks");
        let custom = hooks.join(HOOK_NAME);
        std::fs::write(
            &custom,
            "#!/bin/sh\necho custom-ran >> \"$(dirname \"$0\")/ran\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&custom, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let outcome = install_commit_hook(dir.path()).unwrap();
        assert!(
            matches!(outcome, HookInstall::Chained { .. }),
            "{outcome:?}"
        );
        assert!(hooks.join(CHAINED_NAME).exists());
        commit(dir.path(), "feat: y");
        assert!(hooks.join("ran").exists(), "the chained hook must run");
        assert!(message_of_head(dir.path()).contains("Co-Authored-By: Forge"));
        // Re-installing is idempotent: the Forge hook is recognized and refreshed in place.
        assert!(matches!(
            install_commit_hook(dir.path()).unwrap(),
            HookInstall::Installed(_)
        ));
    }

    #[test]
    fn linked_worktrees_and_submodule_style_git_files_resolve_to_the_real_hooks_dir() {
        let dir = repo();
        let wt = dir.path().join("wt");
        git(
            dir.path(),
            &["worktree", "add", "-q", "-b", "b", wt.to_str().unwrap()],
        );
        assert!(wt.join(".git").is_file());
        let outcome = install_commit_hook(&wt).unwrap();
        let HookInstall::Installed(hook) = outcome else {
            panic!("worktree install failed: {outcome:?}");
        };
        assert!(hook.starts_with(dir.path().join(".git/hooks")), "{hook:?}");
        assert!(git_dir(&wt)
            .unwrap()
            .to_string_lossy()
            .contains("worktrees"));
    }

    #[test]
    fn core_hooks_path_is_reported_not_silently_ignored() {
        let dir = repo();
        git(dir.path(), &["config", "core.hooksPath", ".husky"]);
        match install_commit_hook(dir.path()).unwrap() {
            HookInstall::Skipped(why) => assert!(why.contains("core.hooksPath"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert!(!dir.path().join(".husky").exists());
    }
}
