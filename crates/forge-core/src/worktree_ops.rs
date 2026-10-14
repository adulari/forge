//! The git half of the `worktree` tool (docs/features/session-worktrees.md): parse a request,
//! run it against the repository, and say where the session should work next. No session state
//! lives here, so the direct path (`session_worktree.rs`) and the CLI bridge (`forge mcp-serve`)
//! run the exact same code and differ only in how they move their workspace afterwards.

use std::path::{Path, PathBuf};
use std::process::Command;

use forge_types::SideEffect;
use serde_json::{json, Value};

pub const WORKTREE_TOOL: &str = "worktree";

/// The `ToolSpec` advertised on both paths, up front (a core tool): while it sat behind
/// `tool_search`, weaker models reached for `git worktree add` + `cd` and guessed paths wrong.
pub fn worktree_spec() -> forge_provider::ToolSpec {
    forge_provider::ToolSpec {
        name: WORKTREE_TOOL.to_string(),
        description: "Move this session into an isolated git worktree and back: `create` \
             (branch + worktree, then switch in), `enter` an existing worktree (path or branch), \
             `list`, `exit` to the main checkout (`remove: true` also deletes it and its merged \
             branch), `remove` another worktree. After a switch, relative paths, file tools and \
             the shell's cwd resolve in the new root. Merge with git after `exit`. Removal refuses \
             dirty worktrees and unmerged branches unless `force: true`."
            .to_string(),
        schema: json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["create", "enter", "list", "exit", "remove"] },
                "name": { "type": "string", "description": "create: short name for the worktree directory (default: the branch's last segment)." },
                "branch": { "type": "string", "description": "create: branch to create (default forge/<name>); an existing branch is checked out instead. enter/remove: the worktree's branch." },
                "base": { "type": "string", "description": "create: commit-ish to branch from (default HEAD of the current checkout)." },
                "path": { "type": "string", "description": "create: where to put the worktree. enter/remove: the worktree's path." },
                "remove": { "type": "boolean", "description": "exit: also remove the worktree being left and delete its branch." },
                "force": { "type": "boolean", "description": "remove/exit: discard uncommitted or untracked files and delete an unmerged branch." },
                "keep_branch": { "type": "boolean", "description": "remove/exit: keep the branch after removing the worktree." }
            },
            "required": ["action"]
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Create,
    Enter,
    List,
    Exit,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub action: Action,
    pub name: Option<String>,
    pub branch: Option<String>,
    pub base: Option<String>,
    pub path: Option<String>,
    pub remove: bool,
    pub force: bool,
    pub keep_branch: bool,
}

impl Request {
    pub fn parse(args: &Value) -> Result<Self, String> {
        let text = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let flag = |key: &str| args.get(key).and_then(Value::as_bool).unwrap_or(false);
        let action = match text("action").as_deref() {
            Some("create") => Action::Create,
            Some("enter") | Some("switch") => Action::Enter,
            Some("list") => Action::List,
            Some("exit") | Some("leave") => Action::Exit,
            Some("remove") | Some("delete") => Action::Remove,
            other => {
                return Err(format!(
                    "worktree: unknown action {other:?}; use create, enter, list, exit or remove"
                ))
            }
        };
        Ok(Self {
            action,
            name: text("name"),
            branch: text("branch"),
            base: text("base"),
            path: text("path"),
            remove: flag("remove"),
            force: flag("force"),
            keep_branch: flag("keep_branch"),
        })
    }

    /// How the permission broker sees this call. Moving between checkouts writes nothing;
    /// creating or deleting a worktree runs git and touches the disk like a shell command.
    pub fn side_effect(&self) -> SideEffect {
        match self.action {
            Action::List | Action::Enter => SideEffect::ReadOnly,
            Action::Exit if !self.remove => SideEffect::ReadOnly,
            _ => SideEffect::Shell,
        }
    }

    fn target(&self) -> Option<&str> {
        self.path
            .as_deref()
            .or(self.branch.as_deref())
            .or(self.name.as_deref())
    }
}

/// Where the call runs from and what it may touch.
pub struct Context<'a> {
    /// The session's current workspace root.
    pub current: &'a Path,
    /// Where `exit` returns to: the root the session left on its first switch. `None` falls back
    /// to the repository's main checkout.
    pub home: Option<&'a Path>,
    /// A worktree a `forge serve` daemon created for this session; its lifecycle belongs to the
    /// daemon's merge/discard flow, so this tool never removes or leaves it.
    pub daemon_owned: Option<&'a Path>,
    /// `Some` when `[tools] confine_to_workspace` is on: the roots a session may move into.
    pub confine: Option<&'a [PathBuf]>,
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub ok: bool,
    pub text: String,
    /// The root the session must switch to, when the call moved it.
    pub switch_to: Option<PathBuf>,
}

impl Outcome {
    fn fail(text: impl Into<String>) -> Self {
        Self {
            ok: false,
            text: text.into(),
            switch_to: None,
        }
    }

    fn done(text: impl Into<String>, switch_to: Option<PathBuf>) -> Self {
        Self {
            ok: true,
            text: text.into(),
            switch_to,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    /// `None` for a detached HEAD.
    pub branch: Option<String>,
    pub head: String,
    /// The repository's main checkout (always the first entry git lists).
    pub main: bool,
}

pub fn execute(request: &Request, ctx: &Context<'_>) -> Outcome {
    let result = match request.action {
        Action::List => list_text(ctx.current).map(|t| Outcome::done(t, None)),
        Action::Create => create(request, ctx),
        Action::Enter => enter(request, ctx),
        Action::Exit => exit(request, ctx),
        Action::Remove => remove_named(request, ctx),
    };
    result.unwrap_or_else(Outcome::fail)
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("running git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Every worktree of the repository `root` belongs to, main checkout first.
pub fn list(root: &Path) -> Result<Vec<Entry>, String> {
    let text = git(root, &["worktree", "list", "--porcelain"]).map_err(|e| {
        format!(
            "worktree: {} is not inside a git repository ({e})",
            root.display()
        )
    })?;
    Ok(parse_porcelain(&text))
}

pub fn parse_porcelain(text: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    for (index, block) in text.split("\n\n").enumerate() {
        // A bare repository lists itself first with no checkout; then nothing is the main one.
        let mut entry = Entry {
            path: PathBuf::new(),
            branch: None,
            head: String::new(),
            main: index == 0,
        };
        let mut bare = false;
        for line in block.lines() {
            if let Some(path) = line.strip_prefix("worktree ") {
                entry.path = canonical(Path::new(path));
            } else if let Some(head) = line.strip_prefix("HEAD ") {
                entry.head = head.chars().take(7).collect();
            } else if let Some(branch) = line.strip_prefix("branch ") {
                entry.branch = Some(branch.trim_start_matches("refs/heads/").to_string());
            } else if line == "bare" {
                bare = true;
            }
        }
        if !entry.path.as_os_str().is_empty() && !bare {
            entries.push(entry);
        }
    }
    entries
}

fn list_text(current: &Path) -> Result<String, String> {
    let current = canonical(current);
    let lines: Vec<String> = list(&current)?
        .iter()
        .map(|e| {
            let branch = e
                .branch
                .clone()
                .unwrap_or_else(|| format!("detached @{}", e.head));
            let mut tags = Vec::new();
            if e.main {
                tags.push("main checkout");
            }
            if e.path == current {
                tags.push("current");
            }
            let tags = if tags.is_empty() {
                String::new()
            } else {
                format!(" ({})", tags.join(", "))
            };
            format!("{} [{branch}]{tags}", e.path.display())
        })
        .collect();
    Ok(lines.join("\n"))
}

fn main_checkout(current: &Path) -> Result<Entry, String> {
    list(current)?
        .into_iter()
        .find(|e| e.main)
        .ok_or_else(|| "worktree: this repository has no main checkout (bare repo)".to_string())
}

fn find(current: &Path, target: &str) -> Result<Entry, String> {
    let entries = list(current)?;
    let as_path = {
        let p = Path::new(target);
        canonical(&if p.is_absolute() {
            p.to_path_buf()
        } else {
            current.join(p)
        })
    };
    entries
        .iter()
        .find(|e| e.path == as_path)
        .or_else(|| entries.iter().find(|e| e.branch.as_deref() == Some(target)))
        .or_else(|| {
            entries
                .iter()
                .find(|e| !e.main && e.path.file_name().and_then(|n| n.to_str()) == Some(target))
        })
        .cloned()
        .ok_or_else(|| {
            let known: Vec<String> = entries
                .iter()
                .map(|e| {
                    format!(
                        "{} [{}]",
                        e.path.display(),
                        e.branch.as_deref().unwrap_or("detached")
                    )
                })
                .collect();
            format!(
                "worktree: no worktree of this repository matches '{target}'. Known: {}",
                known.join("; ")
            )
        })
}

fn confined(ctx: &Context<'_>, target: &Path) -> Result<(), String> {
    let Some(roots) = ctx.confine else {
        return Ok(());
    };
    let target = canonical(target);
    if roots.iter().any(|r| target.starts_with(canonical(r))) {
        return Ok(());
    }
    Err(format!(
        "worktree: {} is outside the workspace and every `[tools] extra_roots` entry, and \
         `confine_to_workspace` is on. Add its parent to `extra_roots` to work there.",
        target.display()
    ))
}

/// Lowercase, `[a-z0-9._-]` only, for a directory name.
pub fn slug(raw: &str) -> String {
    let s: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    s.trim_matches(|c| c == '-' || c == '.').to_string()
}

fn create(req: &Request, ctx: &Context<'_>) -> Result<Outcome, String> {
    let main = main_checkout(ctx.current)?;
    let name = req
        .name
        .clone()
        .or_else(|| {
            req.branch
                .as_deref()
                .map(|b| b.rsplit('/').next().unwrap_or(b).to_string())
        })
        .map(|n| slug(&n))
        .filter(|n| !n.is_empty())
        .ok_or("worktree create: give a `name` or a `branch`")?;
    let branch = req
        .branch
        .clone()
        .unwrap_or_else(|| format!("forge/{name}"));
    let managed = main.path.join(".forge").join("worktrees");
    let path = match req.path.as_deref() {
        Some(p) if Path::new(p).is_absolute() => PathBuf::from(p),
        Some(p) => main.path.join(p),
        None => managed.join(&name),
    };
    if path.exists() && std::fs::read_dir(&path).is_ok_and(|mut d| d.next().is_some()) {
        return Err(format!(
            "worktree create: {} already exists and is not empty; pick another `name`/`path`, or \
             `enter` it if it is already a worktree",
            path.display()
        ));
    }
    confined(ctx, path.parent().unwrap_or(&path))?;
    if path.starts_with(&managed) {
        std::fs::create_dir_all(&managed).map_err(|e| format!("worktree create: {e}"))?;
        // Keeps the nested checkouts out of the main checkout's `git status`, the way the
        // checkpoint and evidence directories already are, without editing the user's ignores.
        let ignore = managed.join(".gitignore");
        if !ignore.exists() {
            let _ = std::fs::write(ignore, "*\n");
        }
    }
    let path_arg = path.display().to_string();
    let exists = git(
        ctx.current,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok();
    let base = req.base.as_deref().unwrap_or("HEAD");
    let added = if exists {
        git(ctx.current, &["worktree", "add", &path_arg, &branch])
    } else {
        git(
            ctx.current,
            &["worktree", "add", "-b", &branch, &path_arg, base],
        )
    };
    added.map_err(|e| format!("worktree create: git worktree add failed: {e}"))?;
    let path = canonical(&path);
    let how = if exists {
        format!("checked out existing branch {branch}")
    } else {
        format!("new branch {branch} from {base}")
    };
    Ok(Outcome::done(
        format!("created worktree {} ({how})", path.display()),
        Some(path),
    ))
}

fn enter(req: &Request, ctx: &Context<'_>) -> Result<Outcome, String> {
    let target = req
        .target()
        .ok_or("worktree enter: give the worktree's `path` or `branch`")?;
    let entry = find(ctx.current, target)?;
    if entry.path == canonical(ctx.current) {
        return Ok(Outcome::done(
            format!("already working in {}", entry.path.display()),
            None,
        ));
    }
    if !entry.path.is_dir() {
        return Err(format!(
            "worktree enter: {} is registered but its directory is gone (`git worktree prune` \
             drops the registration)",
            entry.path.display()
        ));
    }
    confined(ctx, &entry.path)?;
    let branch = entry
        .branch
        .clone()
        .unwrap_or_else(|| format!("detached @{}", entry.head));
    Ok(Outcome::done(
        format!("entered worktree {} [{branch}]", entry.path.display()),
        Some(entry.path),
    ))
}

fn exit(req: &Request, ctx: &Context<'_>) -> Result<Outcome, String> {
    let current = canonical(ctx.current);
    let main = main_checkout(&current)?;
    // A daemon session's own worktree is its home when nothing better is known (a bridge
    // process, or a resumed session), so leaving a second worktree never lands it in main.
    let home = ctx
        .home
        .map(canonical)
        .or_else(|| ctx.daemon_owned.map(canonical).filter(|d| *d != current))
        .unwrap_or(main.path.clone());
    if home == current {
        return Ok(Outcome::done(
            format!("already in {}; nothing to exit", current.display()),
            None,
        ));
    }
    if ctx.home.is_none() && ctx.daemon_owned.is_some_and(|d| canonical(d) == current) {
        return Err(
            "worktree exit: this session was started in this worktree by `forge serve`; the \
             daemon merges or discards it (archive/merge from the app), not this tool"
                .to_string(),
        );
    }
    let mut text = format!(
        "left {} and returned to {}",
        current.display(),
        home.display()
    );
    if req.remove {
        let entry = list(&current)?
            .into_iter()
            .find(|e| e.path == current)
            .ok_or("worktree exit: the current root is not a worktree of this repository")?;
        if entry.main {
            return Err("worktree exit: refusing to remove the main checkout".to_string());
        }
        text.push_str(&format!("; {}", remove_entry(&main.path, &entry, req)?));
    }
    Ok(Outcome::done(text, Some(home)))
}

fn remove_named(req: &Request, ctx: &Context<'_>) -> Result<Outcome, String> {
    let target = req
        .target()
        .ok_or("worktree remove: give the worktree's `path` or `branch`")?;
    let entry = find(ctx.current, target)?;
    if entry.main {
        return Err("worktree remove: refusing to remove the main checkout".to_string());
    }
    if entry.path == canonical(ctx.current) {
        return Err(
            "worktree remove: this session is working in that worktree; use `exit` with \
             `remove: true` to leave and remove it in one step"
                .to_string(),
        );
    }
    if ctx.daemon_owned.is_some_and(|d| canonical(d) == entry.path) {
        return Err(
            "worktree remove: that worktree belongs to a `forge serve` session; merge or \
             discard it from the app"
                .to_string(),
        );
    }
    let main = main_checkout(ctx.current)?;
    Ok(Outcome::done(remove_entry(&main.path, &entry, req)?, None))
}

/// Remove one worktree and (unless kept) its branch. Without `force` it refuses a worktree with
/// uncommitted or untracked files, and keeps a branch that is not merged into the main checkout.
fn remove_entry(main: &Path, entry: &Entry, req: &Request) -> Result<String, String> {
    let path = entry.path.display().to_string();
    if entry.path.is_dir() && !req.force {
        let status = git(&entry.path, &["status", "--porcelain"]).unwrap_or_default();
        let dirty: Vec<&str> = status.lines().map(|l| l.get(3..).unwrap_or(l)).collect();
        if !dirty.is_empty() {
            let shown: Vec<&str> = dirty.iter().take(10).copied().collect();
            return Err(format!(
                "worktree: {path} has uncommitted or untracked files ({}{}); commit them, delete \
                 them, or pass `force: true` to discard them. Nothing was removed.",
                shown.join(", "),
                if dirty.len() > shown.len() {
                    ", ..."
                } else {
                    ""
                }
            ));
        }
    }
    let mut args = vec!["worktree", "remove"];
    if req.force {
        args.push("--force");
    }
    args.push(&path);
    if entry.path.is_dir() {
        git(main, &args).map_err(|e| format!("worktree: git worktree remove failed: {e}"))?;
    } else {
        let _ = git(main, &["worktree", "prune"]);
    }
    let mut text = format!("removed worktree {path}");
    let Some(branch) = entry.branch.as_deref().filter(|_| !req.keep_branch) else {
        return Ok(text);
    };
    match git(main, &["branch", "-d", branch]) {
        Ok(_) => text.push_str(&format!(" and deleted branch {branch}")),
        Err(_) if req.force => match git(main, &["branch", "-D", branch]) {
            Ok(_) => text.push_str(&format!(" and force-deleted unmerged branch {branch}")),
            Err(e) => text.push_str(&format!("; branch {branch} kept: {e}")),
        },
        Err(e) => text.push_str(&format!(
            "; branch {branch} kept because it is not merged into the main checkout's HEAD \
             ({e}). Merge it, or remove again with `force: true` to delete it."
        )),
    }
    Ok(text)
}

#[cfg(test)]
#[path = "tests/worktree_ops.rs"]
mod tests;
