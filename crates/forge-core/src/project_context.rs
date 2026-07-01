//! Project-context detection: does the router know what codebase it's operating in?
//!
//! Specifically: is the CURRENT project the same source tree this very binary was itself built
//! from — a genuine, structural self-hosting signal (comparing compile-time package identity
//! against the runtime project's own Cargo.toml), not a keyword match on any one project's name.
//! This generalizes to any agent built the same way, not just this one, and lets the classifier
//! treat otherwise-ordinary infrastructure vocabulary ("mesh", "router", "classifier"...) as
//! higher-stakes only when the session is actually touching the agent's own core routing logic —
//! not every time those words happen to appear in an unrelated project.
//!
//! [`ProjectContext`] itself lives in `forge_types` (shared with forge-mesh, which has no file-I/O
//! dependencies of its own); this module holds the computation, which does need them.

use std::path::{Path, PathBuf};

pub use forge_types::ProjectContext;

const SELF_REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const SELF_NAME: &str = env!("CARGO_PKG_NAME");

/// Compute the [`ProjectContext`] for `cwd`. Cheap, local file reads only (a Cargo.toml at or
/// above `cwd`, plus its workspace root if fields are inherited) — call once per session and
/// cache the result rather than recomputing every turn.
pub fn compute(cwd: &Path) -> ProjectContext {
    let root = find_root(cwd);
    let Some((name, repository)) = read_cargo_identity(&root) else {
        return ProjectContext::default();
    };
    let is_self_hosting = repository
        .as_deref()
        .is_some_and(|r| repos_match(r, SELF_REPOSITORY))
        || name.as_deref().is_some_and(|n| n == SELF_NAME);
    ProjectContext {
        project_name: name,
        is_self_hosting,
    }
}

/// The nearest ancestor of `cwd` (inclusive) containing a recognized project marker, or `cwd`
/// itself if none is found. `forge_lsp::registry::repo_root` walks up from a FILE's parent, so a
/// synthetic filename inside `cwd` makes it check `cwd` itself first, not skip straight past it.
fn find_root(cwd: &Path) -> PathBuf {
    forge_lsp::registry::repo_root(&cwd.join("_")).unwrap_or_else(|| cwd.to_path_buf())
}

/// Read `[package] name`/`repository` from `root/Cargo.toml`, following `repository.workspace =
/// true` inheritance up to the nearest ancestor `Cargo.toml`'s `[workspace.package]` table.
/// `None` when there's no `Cargo.toml` at `root` at all (e.g. a non-Rust project).
///
/// `root` itself may BE a workspace root with no `[package]` table of its own (only
/// `[workspace.package]`) — `cwd` pointing at the workspace root directly, not a member crate's
/// subdirectory, is a real, common case (it's exactly how this binary's own repo is normally
/// operated from). That still counts as a real project identity, just sourced from
/// `[workspace.package]` directly instead of a `[package]` table.
fn read_cargo_identity(root: &Path) -> Option<(Option<String>, Option<String>)> {
    let text = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let doc: toml::Value = toml::from_str(&text).ok()?;

    let Some(pkg) = doc.get("package") else {
        let repository = doc
            .get("workspace")
            .and_then(|w| w.get("package"))
            .and_then(|p| p.get("repository"))
            .and_then(|v| v.as_str())
            .map(String::from);
        return repository.map(|r| (None, Some(r)));
    };

    let name = pkg.get("name").and_then(|v| v.as_str()).map(String::from);
    let repository = match pkg.get("repository") {
        Some(toml::Value::String(s)) => Some(s.clone()),
        Some(toml::Value::Table(t))
            if t.get("workspace").and_then(|v| v.as_bool()) == Some(true) =>
        {
            find_workspace_repository(root)
        }
        _ => None,
    };
    Some((name, repository))
}

/// Walk up from `dir` looking for a `Cargo.toml` with a `[workspace.package] repository`.
fn find_workspace_repository(dir: &Path) -> Option<String> {
    let mut cur = dir.parent();
    while let Some(d) = cur {
        if let Ok(text) = std::fs::read_to_string(d.join("Cargo.toml")) {
            if let Ok(doc) = toml::from_str::<toml::Value>(&text) {
                if let Some(repo) = doc
                    .get("workspace")
                    .and_then(|w| w.get("package"))
                    .and_then(|p| p.get("repository"))
                    .and_then(|v| v.as_str())
                {
                    return Some(repo.to_string());
                }
            }
        }
        cur = d.parent();
    }
    None
}

/// Compare two repository URLs loosely — case-insensitive, ignoring a trailing `/` or `.git` —
/// so e.g. a GitHub org rename doesn't break a binary built before it from recognizing its own
/// still-redirecting repository URL.
fn repos_match(a: &str, b: &str) -> bool {
    fn norm(s: &str) -> String {
        s.trim_end_matches('/')
            .trim_end_matches(".git")
            .to_lowercase()
    }
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn detects_self_hosting_via_matching_repository() {
        let tmp = std::env::temp_dir().join(format!("pc-test-{}", std::process::id()));
        write(
            &tmp,
            "Cargo.toml",
            &format!("[package]\nname = \"unrelated-name\"\nrepository = \"{SELF_REPOSITORY}\"\n"),
        );
        let ctx = compute(&tmp);
        assert!(ctx.is_self_hosting, "matching repository must self-host");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn detects_self_hosting_via_matching_repository_case_and_slash_insensitive() {
        let tmp = std::env::temp_dir().join(format!("pc-test-case-{}", std::process::id()));
        let loud = format!("{}/", SELF_REPOSITORY.to_uppercase());
        write(
            &tmp,
            "Cargo.toml",
            &format!("[package]\nname = \"x\"\nrepository = \"{loud}\"\n"),
        );
        let ctx = compute(&tmp);
        assert!(ctx.is_self_hosting);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn unrelated_project_is_not_self_hosting() {
        let tmp = std::env::temp_dir().join(format!("pc-test-other-{}", std::process::id()));
        write(
            &tmp,
            "Cargo.toml",
            "[package]\nname = \"some-other-app\"\nrepository = \"https://github.com/someone/else\"\n",
        );
        let ctx = compute(&tmp);
        assert!(!ctx.is_self_hosting);
        assert_eq!(ctx.project_name.as_deref(), Some("some-other-app"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn no_cargo_toml_is_a_neutral_default() {
        let tmp = std::env::temp_dir().join(format!("pc-test-none-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let ctx = compute(&tmp);
        assert_eq!(ctx, ProjectContext::default());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn inherited_workspace_repository_is_followed() {
        let tmp = std::env::temp_dir().join(format!("pc-test-ws-{}", std::process::id()));
        write(
            &tmp,
            "Cargo.toml",
            &format!("[workspace]\nmembers = [\"crates/x\"]\n\n[workspace.package]\nrepository = \"{SELF_REPOSITORY}\"\n"),
        );
        write(
            &tmp,
            "crates/x/Cargo.toml",
            "[package]\nname = \"x\"\nrepository.workspace = true\n",
        );
        let ctx = compute(&tmp.join("crates/x"));
        assert!(
            ctx.is_self_hosting,
            "workspace-inherited repository must be followed"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn cwd_at_the_workspace_root_itself_is_detected() {
        // The real, common case: cwd IS the workspace root (no `[package]` table there at all,
        // only `[workspace.package]` — this binary's own repo is exactly this shape). Regression
        // for a real bug: `read_cargo_identity` originally bailed via `doc.get("package")?` before
        // ever checking `[workspace.package]`, so running from the workspace root itself (as
        // opposed to a member crate's subdirectory) silently never detected self-hosting.
        let tmp = std::env::temp_dir().join(format!("pc-test-wsroot-{}", std::process::id()));
        write(
            &tmp,
            "Cargo.toml",
            &format!(
                "[workspace]\nmembers = [\"crates/x\"]\n\n[workspace.package]\nrepository = \"{SELF_REPOSITORY}\"\n"
            ),
        );
        let ctx = compute(&tmp);
        assert!(
            ctx.is_self_hosting,
            "cwd at the workspace root itself must still detect self-hosting"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }
}
