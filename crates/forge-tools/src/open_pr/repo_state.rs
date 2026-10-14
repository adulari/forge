//! Reading `git status --porcelain=v1 -z` for the dirty-tree check.

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Dirty {
    /// Staged or unstaged changes to tracked files (including deletions and renames).
    pub tracked: Vec<String>,
    pub untracked: Vec<String>,
}

/// Split porcelain output into tracked changes and untracked files. Paths under `.forge/` are
/// Forge's own state (evidence, jobs, checkpoints) and never part of a PR, so they are ignored.
pub(super) fn dirty_paths(porcelain_z: &str) -> Dirty {
    let mut dirty = Dirty::default();
    let mut entries = porcelain_z.split('\0').filter(|e| !e.is_empty());
    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let (code, path) = (&entry[..2], &entry[3..]);
        if code.starts_with('R') || code.starts_with('C') {
            entries.next();
        }
        if path == ".forge" || path.starts_with(".forge/") {
            continue;
        }
        if code == "??" {
            dirty.untracked.push(path.to_string());
        } else if code != "!!" {
            dirty.tracked.push(path.to_string());
        }
    }
    dirty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_entries_split_into_tracked_and_untracked() {
        let status =
            " M src/a.rs\0R  new.rs\0old.rs\0?? notes.txt\0?? .forge/\0A  .forge/x\0D  gone.rs\0";
        let dirty = dirty_paths(status);
        assert_eq!(dirty.tracked, vec!["src/a.rs", "new.rs", "gone.rs"]);
        assert_eq!(dirty.untracked, vec!["notes.txt"]);
        assert_eq!(dirty_paths(""), Dirty::default());
    }
}
