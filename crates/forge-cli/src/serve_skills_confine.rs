//! A project's own files decide where project-scope skill actions write and delete. A repository
//! can commit `.forge/skills` or `.claude` as a symlink to somewhere else (the user's home, their
//! global `~/.claude`), so every such path must resolve inside the repository before it is used.

use std::path::Path;

const SKILL_PATHS: &[&str] = &[
    ".forge",
    ".forge/skills",
    ".forge/installed-skills.toml",
    ".agents",
    ".agents/skills",
    ".claude",
    ".claude/skills",
];

/// The first skill-related path under `root` that exists but resolves outside it.
pub(crate) fn escaping_path(root: &Path) -> Option<&'static str> {
    let canonical_root = std::fs::canonicalize(root).ok()?;
    SKILL_PATHS.iter().copied().find(|rel| {
        std::fs::canonicalize(root.join(rel)).is_ok_and(|real| !real.starts_with(&canonical_root))
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn symlinks_leaving_the_repository_are_found() {
        let base = std::env::temp_dir().join(format!("forge-skill-confine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let repo = base.join("repo");
        std::fs::create_dir_all(repo.join(".forge")).unwrap();
        std::fs::create_dir_all(base.join("home")).unwrap();
        assert_eq!(escaping_path(&repo), None);
        std::fs::create_dir_all(repo.join("inside")).unwrap();
        std::os::unix::fs::symlink(repo.join("inside"), repo.join(".agents")).unwrap();
        assert_eq!(
            escaping_path(&repo),
            None,
            "a link that stays inside is fine"
        );
        std::os::unix::fs::symlink(base.join("home"), repo.join(".claude")).unwrap();
        assert_eq!(escaping_path(&repo), Some(".claude"));
        std::fs::remove_dir_all(repo.join(".forge")).unwrap();
        std::os::unix::fs::symlink(base.join("home"), repo.join(".forge")).unwrap();
        assert_eq!(escaping_path(&repo), Some(".forge"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
