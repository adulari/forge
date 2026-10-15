//! The files a session read or edited most recently, newest first. A compaction re-reads the
//! newest few from disk (`file_restore.rs`), because the summary keeps what happened to a file
//! but not the text the model was working from.

use std::path::{Path, PathBuf};

use crate::Session;

/// Files remembered.
const CAP: usize = 20;

#[derive(Debug, Default)]
pub(crate) struct RecentFiles {
    paths: Vec<PathBuf>,
}

impl RecentFiles {
    pub(crate) fn touch(&mut self, path: PathBuf) {
        self.paths.retain(|p| *p != path);
        self.paths.insert(0, path);
        self.paths.truncate(CAP);
    }

    pub(crate) fn newest_first(&self) -> &[PathBuf] {
        &self.paths
    }
}

/// `src/lib.rs:120-180` names `src/lib.rs`: a batch read may carry a line range.
fn strip_line_range(entry: &str) -> &str {
    match entry.rsplit_once(':') {
        Some((path, range))
            if !path.is_empty()
                && !range.is_empty()
                && range.chars().all(|c| c.is_ascii_digit() || c == '-') =>
        {
            path
        }
        _ => entry,
    }
}

/// The files a `read_file` call names, in the order it names them.
pub(crate) fn read_paths(args: &serde_json::Value) -> Vec<String> {
    if let Some(paths) = args
        .get("paths")
        .and_then(serde_json::Value::as_array)
        .filter(|paths| !paths.is_empty())
    {
        return paths
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(|entry| strip_line_range(entry).to_string())
            .collect();
    }
    ["path", "file_path"]
        .iter()
        .find_map(|key| args.get(key).and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .into_iter()
        .collect()
}

impl Session {
    /// Remember a file the model just read or edited. A relative path is relative to the workspace.
    pub(crate) fn note_file_use(&mut self, path: &str) {
        if path.trim().is_empty() {
            return;
        }
        let path = Path::new(path);
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace.root().join(path)
        };
        self.recent_files.touch(absolute);
    }

    pub(crate) fn note_read_files(&mut self, tool: &str, args: &serde_json::Value) {
        if tool == "read_file" {
            for path in read_paths(args) {
                self.note_file_use(&path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_file_is_first_and_a_repeat_moves_up() {
        let mut files = RecentFiles::default();
        files.touch("a".into());
        files.touch("b".into());
        files.touch("a".into());
        assert_eq!(
            files.newest_first(),
            [PathBuf::from("a"), PathBuf::from("b")]
        );
    }

    #[test]
    fn only_the_newest_twenty_are_kept() {
        let mut files = RecentFiles::default();
        for i in 0..CAP + 5 {
            files.touch(format!("f{i}").into());
        }
        assert_eq!(files.newest_first().len(), CAP);
        assert_eq!(
            files.newest_first()[0],
            PathBuf::from(format!("f{}", CAP + 4))
        );
    }

    #[test]
    fn a_batch_read_names_its_files_without_the_line_range() {
        let args = serde_json::json!({ "paths": ["src/lib.rs:120-180", "Cargo.toml", "a:b.rs"] });
        assert_eq!(read_paths(&args), ["src/lib.rs", "Cargo.toml", "a:b.rs"]);
        assert_eq!(
            read_paths(&serde_json::json!({ "file_path": "x.rs" })),
            ["x.rs"]
        );
        assert!(read_paths(&serde_json::json!({})).is_empty());
    }
}
