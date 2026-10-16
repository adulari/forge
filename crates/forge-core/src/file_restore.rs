//! Put the files the session was working on back in front of the model after a compaction.
//!
//! The summary says which files were read and changed; it does not hold their text, and the
//! model had been working from that text. The newest few are re-read from disk, so they are
//! current, and handed over as one system note. A bridged claude or codex reads files with its own
//! tools and keeps its own context, so it only gets the list.

use forge_types::MessageKind;

use super::*;

/// Files whose contents are restored.
pub(crate) const RESTORE_FILES: usize = 5;
/// A file bigger than this many tokens is left out rather than cut.
pub(crate) const RESTORE_FILE_MAX_TOKENS: usize = 25_000;
/// Tokens all restored files may add together.
pub(crate) const RESTORE_TOTAL_TOKENS: usize = 40_000;
/// Paths a bridge is shown.
const BRIDGE_LISTED_FILES: usize = 10;
/// A file past this size is not even read to be counted.
const READ_CEILING_BYTES: u64 = 2 * 1024 * 1024;

pub(crate) const RESTORED_HEADER: &str = "[files restored after compaction]";

/// One restored file's text, or `None` for a missing file, a binary one, or one past `max_tokens`.
fn readable_text(path: &std::path::Path, max_tokens: usize) -> Option<String> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    if meta.len() > READ_CEILING_BYTES {
        return None;
    }
    let text = String::from_utf8(std::fs::read(path).ok()?).ok()?;
    if text.contains('\0') || tokens::count_text(&text) > max_tokens {
        return None;
    }
    Some(text)
}

/// The newest files of `candidates` that fit the limits, with their current text. `total_budget`
/// caps all of them together and also bounds a single file.
pub(crate) fn pick_restorable(
    candidates: &[std::path::PathBuf],
    root: &std::path::Path,
    total_budget: usize,
) -> Vec<(String, String)> {
    let file_max = RESTORE_FILE_MAX_TOKENS.min(total_budget);
    let mut picked = Vec::new();
    let mut total = 0usize;
    for path in candidates {
        if picked.len() == RESTORE_FILES {
            break;
        }
        let Some(text) = readable_text(path, file_max) else {
            continue;
        };
        let cost = tokens::count_text(&text) + tokens::count_text(&path.to_string_lossy());
        if total + cost > total_budget {
            continue;
        }
        total += cost;
        let shown = path.strip_prefix(root).unwrap_or(path);
        picked.push((shown.display().to_string(), text));
    }
    picked
}

pub(crate) fn render_restored(files: &[(String, String)]) -> String {
    let mut out = format!(
        "{RESTORED_HEADER}\nThe conversation was compacted. These are the files you most \
         recently read or edited, as they are on disk now:\n"
    );
    for (path, text) in files {
        out.push_str(&format!(
            "\n===== {path} =====\n{}\n",
            text.trim_end_matches('\n')
        ));
    }
    out
}

pub(crate) fn render_restored_paths(paths: &[String]) -> String {
    let mut out = format!(
        "{RESTORED_HEADER}\nThe conversation was compacted. These are the files you most \
         recently read or edited; read them again if you need their contents:\n"
    );
    for path in paths {
        out.push_str(&format!("- {path}\n"));
    }
    out
}

impl Session {
    /// The model the session is on, for the decisions that depend on it outside a request.
    pub(crate) fn session_model_hint(&self) -> Option<String> {
        self.pinned_model()
            .and_then(|set| set.first().cloned())
            .or_else(|| self.route_affinity.as_ref().map(|a| a.model.clone()))
            .or_else(|| self.store.session_last_model(&self.id).ok().flatten())
    }

    /// Tokens the restored files may add: [`RESTORE_TOTAL_TOKENS`], but never more than a quarter
    /// of the window the session runs in (the note stays in every later request) and never more
    /// than what is left under the compaction's target, so restoring cannot put the context back
    /// over the trigger the compaction just got it under.
    fn restore_budget(&self, budget: &crate::compaction_budget::CompactionBudget) -> usize {
        let window = self.session_model_hint().map_or(
            forge_mesh::pricing::CONSERVATIVE_CONTEXT_WINDOW as usize,
            |model| self.effective_context_window(&model) as usize,
        );
        budget.restore_room(
            self.context_pressure_tokens(),
            RESTORE_TOTAL_TOKENS.min(window / 4),
        )
    }

    /// Append the restored-files note after a compaction. Nothing is added when no recent file is
    /// left on disk.
    pub(crate) fn restore_recent_files(
        &mut self,
        budget: &crate::compaction_budget::CompactionBudget,
    ) -> Result<(), CoreError> {
        let root = self.workspace.root().to_path_buf();
        let bridged = self
            .session_model_hint()
            .is_some_and(|model| forge_provider::is_cli_bridge(&model));
        let candidates = self.recent_files.newest_first().to_vec();
        let note = if bridged {
            let paths: Vec<String> = candidates
                .iter()
                .filter(|p| p.is_file())
                .take(BRIDGE_LISTED_FILES)
                .map(|p| p.strip_prefix(&root).unwrap_or(p).display().to_string())
                .collect();
            (!paths.is_empty()).then(|| render_restored_paths(&paths))
        } else {
            let files = pick_restorable(&candidates, &root, self.restore_budget(budget));
            (!files.is_empty()).then(|| render_restored(&files))
        };
        match note {
            Some(note) => self.push_note(MessageKind::FilesRestored, note),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &std::path::Path, name: &str, text: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn the_newest_five_readable_files_are_restored_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<_> = (0..7)
            .map(|i| write(dir.path(), &format!("f{i}.rs"), &format!("fn f{i}() {{}}")))
            .collect();
        let newest_first: Vec<_> = paths.iter().rev().cloned().collect();
        let picked = pick_restorable(&newest_first, dir.path(), RESTORE_TOTAL_TOKENS);
        assert_eq!(picked.len(), RESTORE_FILES);
        assert_eq!(picked[0].0, "f6.rs");
        assert_eq!(picked[0].1, "fn f6() {}");
        assert_eq!(picked[4].0, "f2.rs");
    }

    #[test]
    fn missing_binary_and_oversized_files_are_skipped_not_cut() {
        let dir = tempfile::tempdir().unwrap();
        let good = write(dir.path(), "good.rs", "fn good() {}");
        let binary = dir.path().join("blob.bin");
        std::fs::write(&binary, [0u8, 159, 146, 150, 0]).unwrap();
        let nul = write(dir.path(), "nul.txt", "text\0more");
        let huge = write(
            dir.path(),
            "huge.txt",
            &"word ".repeat(RESTORE_FILE_MAX_TOKENS + 100),
        );
        let missing = dir.path().join("gone.rs");
        let picked = pick_restorable(
            &[missing, binary, nul, huge, good],
            dir.path(),
            RESTORE_TOTAL_TOKENS,
        );
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].0, "good.rs");
    }

    #[test]
    fn the_total_budget_skips_a_file_that_would_overrun_it_and_keeps_looking() {
        let dir = tempfile::tempdir().unwrap();
        let big = "word ".repeat(RESTORE_FILE_MAX_TOKENS - 2_000);
        let first = write(dir.path(), "a.txt", &big);
        let second = write(dir.path(), "b.txt", &big);
        let small = write(dir.path(), "c.txt", "tiny");
        let picked = pick_restorable(&[first, second, small], dir.path(), RESTORE_TOTAL_TOKENS);
        let names: Vec<_> = picked.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(names, ["a.txt", "c.txt"], "b.txt would pass 40k tokens");
    }

    #[test]
    fn a_small_window_shrinks_both_limits() {
        let dir = tempfile::tempdir().unwrap();
        let mid = write(dir.path(), "mid.txt", &"word ".repeat(3_000));
        let small = write(dir.path(), "small.txt", "tiny");
        let picked = pick_restorable(&[mid, small], dir.path(), 2_000);
        let names: Vec<_> = picked.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            names,
            ["small.txt"],
            "a file bigger than the whole budget is skipped"
        );
    }

    #[test]
    fn the_note_carries_each_path_and_its_text() {
        let note = render_restored(&[("src/a.rs".into(), "fn a() {}\n".into())]);
        assert!(note.starts_with(RESTORED_HEADER));
        assert!(note.contains("===== src/a.rs =====\nfn a() {}\n"));
    }
}
