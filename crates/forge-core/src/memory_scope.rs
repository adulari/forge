//! Auto-memory is keyed by the workspace path, so a workspace that changes path (a host move, a
//! re-clone, a linked worktree) used to start with no memories. Each project also has a stable
//! alias scope per git remote (`repo:<host/owner/repo>[#<path inside the repo>]`); the path scope
//! stays the primary key every reader and writer uses, and the two are kept in step by copying
//! what either lacks. Nothing is deleted from the old key.

use std::path::Path;

use crate::host_move::{repo_remotes, repo_root};

pub(crate) fn memory_alias_scopes(root: &Path) -> Vec<String> {
    let Some(repo) = repo_root(root) else {
        return Vec::new();
    };
    let inside = root
        .strip_prefix(&repo)
        .ok()
        .map(|p| p.display().to_string())
        .filter(|p| !p.is_empty());
    repo_remotes(&repo)
        .into_iter()
        .map(|remote| match &inside {
            Some(inside) => format!("repo:{remote}#{inside}"),
            None => format!("repo:{remote}"),
        })
        .collect()
}

/// Make the memories of every alias scope of `root` available under its path scope, and the path
/// scope's memories available under the aliases, so the next relocation finds them.
pub(crate) fn align_memory_scopes(store: &forge_store::Store, root: &Path) {
    let path_scope = crate::memory_scope_at(root);
    for alias in memory_alias_scopes(root) {
        let _ = store.copy_missing_memories(&alias, &path_scope);
        let _ = store.copy_missing_memories(&path_scope, &alias);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_store::Store;

    fn repo(dir: &Path, remote: &str) {
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(
            dir.join(".git/config"),
            format!("[remote \"origin\"]\n\turl = {remote}\n"),
        )
        .unwrap();
    }

    #[test]
    fn relocated_workspace_recalls_memories_of_the_old_path() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old/proj");
        let new = tmp.path().join("new/moved");
        repo(&old, "git@github.com:acme/proj.git");
        repo(&new, "https://github.com/acme/proj");
        let store = Store::open_in_memory().unwrap();

        store
            .add_memory(
                &crate::memory_scope_at(&old),
                "decision",
                "deploy with blue green",
                "s1",
            )
            .unwrap();
        align_memory_scopes(&store, &old);

        align_memory_scopes(&store, &new);
        let found = store
            .recall_memories(&crate::memory_scope_at(&new), "deploy", 6)
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text, "deploy with blue green");
        assert_eq!(
            store
                .list_memories(&crate::memory_scope_at(&old))
                .unwrap()
                .len(),
            1,
            "the old key keeps its memories"
        );

        align_memory_scopes(&store, &new);
        assert_eq!(
            store.count_memories(&crate::memory_scope_at(&new)).unwrap(),
            1,
            "re-aligning does not duplicate"
        );
    }

    #[test]
    fn memories_written_after_the_last_align_survive_the_next_one() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("a");
        let new = tmp.path().join("b");
        repo(&old, "https://github.com/acme/proj");
        repo(&new, "https://github.com/acme/proj");
        let store = Store::open_in_memory().unwrap();
        store
            .add_memory(
                &crate::memory_scope_at(&old),
                "fact",
                "uses postgres sixteen",
                "s1",
            )
            .unwrap();
        align_memory_scopes(&store, &old);
        store
            .add_memory(
                &crate::memory_scope_at(&old),
                "fact",
                "migrations run nightly",
                "s1",
            )
            .unwrap();
        align_memory_scopes(&store, &old);
        align_memory_scopes(&store, &new);
        assert_eq!(
            store.count_memories(&crate::memory_scope_at(&new)).unwrap(),
            2
        );
    }

    #[test]
    fn different_remote_or_no_remote_shares_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        let bare = tmp.path().join("c");
        repo(&a, "https://github.com/acme/one");
        repo(&b, "https://github.com/acme/two");
        std::fs::create_dir_all(bare.join(".git")).unwrap();
        let store = Store::open_in_memory().unwrap();
        store
            .add_memory(&crate::memory_scope_at(&a), "fact", "alpha only fact", "s1")
            .unwrap();
        align_memory_scopes(&store, &a);
        align_memory_scopes(&store, &b);
        align_memory_scopes(&store, &bare);
        assert_eq!(
            store.count_memories(&crate::memory_scope_at(&b)).unwrap(),
            0
        );
        assert_eq!(
            store
                .count_memories(&crate::memory_scope_at(&bare))
                .unwrap(),
            0
        );
    }
}
