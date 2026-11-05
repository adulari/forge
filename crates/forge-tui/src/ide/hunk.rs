//! Per-hunk stage / unstage / revert for the diff view (`POST /api/git/hunk`), and the undo for a
//! revert. Pure: builds the single-hunk patch the daemon applies, decides which actions a hunk
//! offers, and reverses a patch. The daemon's `revert` is `git apply -R` on the working tree, so
//! applying it to the REVERSED patch puts the original back: no extra route is needed.

use super::wire::{WireDiffFile, WireHunk};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkAction {
    Stage,
    Unstage,
    Revert,
}

impl HunkAction {
    pub fn wire(self) -> &'static str {
        match self {
            HunkAction::Stage => "stage",
            HunkAction::Unstage => "unstage",
            HunkAction::Revert => "revert",
        }
    }

    pub fn done(self) -> &'static str {
        match self {
            HunkAction::Stage => "hunk staged",
            HunkAction::Unstage => "hunk unstaged",
            HunkAction::Revert => "hunk reverted",
        }
    }
}

/// Where a file's diff sits in git, which decides the actions a hunk offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bucket {
    Staged,
    Unstaged,
    Untracked,
    /// A worktree session's whole-branch diff.
    Worktree,
}

/// The actions a hunk in `bucket` offers. A staged hunk can only be unstaged (revert acts on the
/// working tree); a new untracked file is staged as a whole. Nothing is offered where a patch
/// cannot be built faithfully: binary files, renames, and diffs the daemon cut short (a cut hunk
/// is not a valid patch).
pub fn actions_for(bucket: Bucket, file: &WireDiffFile, read_only: bool) -> Vec<HunkAction> {
    if read_only
        || file.binary
        || file.kind == "renamed"
        || file.orig_path.is_some()
        || file.skipped_lines > 0
    {
        return Vec::new();
    }
    match bucket {
        Bucket::Staged => vec![HunkAction::Unstage],
        Bucket::Untracked => vec![HunkAction::Stage],
        Bucket::Unstaged | Bucket::Worktree => vec![HunkAction::Stage, HunkAction::Revert],
    }
}

/// The `--- / +++ / @@` patch for one hunk of `path`, ready for `git apply`.
pub fn build_patch(path: &str, kind: &str, hunk: &WireHunk) -> String {
    let from = if kind == "created" {
        "/dev/null".to_string()
    } else {
        format!("a/{path}")
    };
    let to = if kind == "deleted" {
        "/dev/null".to_string()
    } else {
        format!("b/{path}")
    };
    format!(
        "--- {from}\n+++ {to}\n{}\n{}\n",
        hunk.header,
        hunk.lines.join("\n")
    )
}

/// `@@ -a,b +c,d @@ ctx` -> `@@ -c,d +a,b @@ ctx`; the optional counts are kept verbatim.
pub fn reverse_header(header: &str) -> String {
    let Some(inner) = header.strip_prefix("@@ -") else {
        return header.to_string();
    };
    let Some((old, rest)) = inner.split_once(" +") else {
        return header.to_string();
    };
    let Some((new, tail)) = rest.split_once(" @@") else {
        return header.to_string();
    };
    format!("@@ -{new} +{old} @@{tail}")
}

/// The hunk with additions and deletions swapped. Context and `\ No newline` lines are unchanged.
pub fn reverse_hunk(hunk: &WireHunk) -> WireHunk {
    WireHunk {
        header: reverse_header(&hunk.header),
        lines: hunk
            .lines
            .iter()
            .map(|line| match line.as_bytes().first() {
                Some(b'+') => format!("-{}", &line[1..]),
                Some(b'-') => format!("+{}", &line[1..]),
                _ => line.clone(),
            })
            .collect(),
    }
}

/// What to POST (`action: "revert"`) to put a reverted hunk back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoStep {
    pub hunk_header: String,
    pub patch: String,
}

pub fn undo_step_for_hunk(path: &str, hunk: &WireHunk) -> UndoStep {
    let reversed = reverse_hunk(hunk);
    // The file header stays a/<path> b/<path>: the daemon only accepts that pair.
    UndoStep {
        hunk_header: reversed.header.clone(),
        patch: build_patch(path, "modified", &reversed),
    }
}

/// Steps that restore a whole reverted file, one per hunk, in file order. Empty when the file
/// cannot be rebuilt from its hunks: a cut-short diff, binary, rename, or a file that was created
/// or deleted (those need the file itself, not a patch of lines).
pub fn undo_steps_for_file(file: &WireDiffFile) -> Vec<UndoStep> {
    if file.binary || file.skipped_lines > 0 || file.kind != "modified" || file.orig_path.is_some()
    {
        return Vec::new();
    }
    file.hunks
        .iter()
        .map(|hunk| undo_step_for_hunk(&file.path, hunk))
        .collect()
}

/// A reverted hunk or file the user can bring back with `u`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoRecord {
    pub session_id: String,
    pub path: String,
    pub label: String,
    pub steps: Vec<UndoStep>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk() -> WireHunk {
        WireHunk {
            header: "@@ -3,2 +3,3 @@ fn x".to_string(),
            lines: vec![
                " keep".to_string(),
                "-old".to_string(),
                "+new".to_string(),
                "+extra".to_string(),
            ],
        }
    }

    fn file(kind: &str) -> WireDiffFile {
        WireDiffFile {
            path: "src/a.rs".to_string(),
            kind: kind.to_string(),
            hunks: vec![hunk()],
            ..Default::default()
        }
    }

    #[test]
    fn a_staged_hunk_can_only_be_unstaged_and_a_new_file_only_staged() {
        let f = file("modified");
        assert_eq!(
            actions_for(Bucket::Staged, &f, false),
            [HunkAction::Unstage]
        );
        assert_eq!(
            actions_for(Bucket::Untracked, &f, false),
            [HunkAction::Stage]
        );
        assert_eq!(
            actions_for(Bucket::Unstaged, &f, false),
            [HunkAction::Stage, HunkAction::Revert]
        );
    }

    #[test]
    fn nothing_is_offered_where_a_patch_cannot_be_faithful() {
        let mut f = file("modified");
        assert!(
            actions_for(Bucket::Unstaged, &f, true).is_empty(),
            "read only"
        );
        f.skipped_lines = 3;
        assert!(
            actions_for(Bucket::Unstaged, &f, false).is_empty(),
            "cut short"
        );
        let mut f = file("renamed");
        assert!(actions_for(Bucket::Unstaged, &f, false).is_empty());
        f = file("modified");
        f.binary = true;
        assert!(actions_for(Bucket::Unstaged, &f, false).is_empty());
    }

    #[test]
    fn the_patch_carries_the_a_b_header_and_the_hunk() {
        let patch = build_patch("src/a.rs", "modified", &hunk());
        assert_eq!(
            patch,
            "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -3,2 +3,3 @@ fn x\n keep\n-old\n+new\n+extra\n"
        );
        assert!(build_patch("n.rs", "created", &hunk()).starts_with("--- /dev/null\n+++ b/n.rs\n"));
        assert!(build_patch("n.rs", "deleted", &hunk()).contains("+++ /dev/null"));
    }

    #[test]
    fn reversing_a_header_swaps_the_sides_and_keeps_optional_counts() {
        assert_eq!(
            reverse_header("@@ -3,2 +3,3 @@ fn x"),
            "@@ -3,3 +3,2 @@ fn x"
        );
        assert_eq!(reverse_header("@@ -5 +5,2 @@"), "@@ -5,2 +5 @@");
        assert_eq!(reverse_header("not a header"), "not a header");
    }

    #[test]
    fn reversing_a_hunk_swaps_adds_and_dels_and_leaves_context() {
        let r = reverse_hunk(&hunk());
        assert_eq!(r.lines, [" keep", "+old", "-new", "-extra"]);
        assert_eq!(reverse_hunk(&r), hunk());
    }

    #[test]
    fn the_undo_patch_reverts_into_the_original() {
        let step = undo_step_for_hunk("src/a.rs", &hunk());
        assert_eq!(step.hunk_header, "@@ -3,3 +3,2 @@ fn x");
        assert!(step
            .patch
            .starts_with("--- a/src/a.rs\n+++ b/src/a.rs\n@@ -3,3 +3,2 @@ fn x\n"));
        assert!(step.patch.contains("\n+old\n-new\n-extra\n"));
    }

    #[test]
    fn a_whole_file_is_restorable_only_when_it_can_be_rebuilt_from_its_hunks() {
        assert_eq!(undo_steps_for_file(&file("modified")).len(), 1);
        assert!(undo_steps_for_file(&file("created")).is_empty());
        let mut cut = file("modified");
        cut.skipped_lines = 1;
        assert!(undo_steps_for_file(&cut).is_empty());
    }
}
