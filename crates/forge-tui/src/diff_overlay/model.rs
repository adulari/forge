//! The data the diff overlay shows, and the parsers that fill it: `git diff` output, the diff
//! cards of the current turn, and an untracked file (which git does not diff at all).

use std::hash::{Hash, Hasher};

use crate::DiffFileSnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Add,
    Del,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
}

impl FileStatus {
    pub fn letter(self) -> &'static str {
        match self {
            FileStatus::Modified => "M",
            FileStatus::Added => "A",
            FileStatus::Deleted => "D",
            FileStatus::Renamed => "R",
            FileStatus::Untracked => "?",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayFile {
    pub path: String,
    pub status: FileStatus,
    pub binary: bool,
    pub adds: usize,
    pub dels: usize,
    pub hunks: Vec<OverlayHunk>,
    /// A line under the file name: a rename's origin, or why no hunks are shown.
    pub note: Option<String>,
    /// Content identity that the hunks do not carry: git's `index a..b` blobs for a binary file,
    /// size and mtime for an untracked file shown without its content.
    pub identity: String,
}

impl OverlayFile {
    fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            status: FileStatus::Modified,
            binary: false,
            adds: 0,
            dels: 0,
            hunks: Vec::new(),
            note: None,
            identity: String::new(),
        }
    }

    /// Identifies this file's content, so a "reviewed" mark is dropped when the file changes
    /// again after it was marked.
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.path.hash(&mut hasher);
        for hunk in &self.hunks {
            hunk.header.hash(&mut hasher);
            for line in &hunk.lines {
                (line.kind as u8).hash(&mut hasher);
                line.text.hash(&mut hasher);
            }
        }
        self.note.hash(&mut hasher);
        self.identity.hash(&mut hasher);
        hasher.finish()
    }
}

/// A named list of files: where the changes come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffSource {
    pub label: String,
    pub files: Vec<OverlayFile>,
}

fn hunk_spans(header: &str) -> Option<(u32, u32, u32, u32)> {
    let inner = header.strip_prefix("@@ -")?;
    let (old, rest) = inner.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    let span = |s: &str| -> Option<(u32, u32)> {
        match s.split_once(',') {
            Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (os, oc) = span(old)?;
    let (ns, nc) = span(new)?;
    Some((os, oc, ns, nc))
}

fn finish_hunk(file: &mut Option<OverlayFile>, hunk: &mut Option<OverlayHunk>) {
    if let (Some(f), Some(h)) = (file.as_mut(), hunk.take()) {
        f.hunks.push(h);
    }
}

fn finish_file(
    out: &mut Vec<OverlayFile>,
    file: &mut Option<OverlayFile>,
    hunk: &mut Option<OverlayHunk>,
) {
    finish_hunk(file, hunk);
    if let Some(f) = file.take() {
        out.push(f);
    }
}

/// Parse `git diff --no-color` output into files and hunks.
pub fn parse_unified(text: &str) -> Vec<OverlayFile> {
    let mut out = Vec::new();
    let mut file: Option<OverlayFile> = None;
    let mut hunk: Option<OverlayHunk> = None;
    let (mut old_no, mut new_no) = (0u32, 0u32);
    let (mut old_left, mut new_left) = (0u32, 0u32);
    for raw in text.lines() {
        let in_body = hunk.is_some() && (old_left > 0 || new_left > 0);
        if in_body && !raw.starts_with('\\') {
            let (sigil, rest) = match raw.chars().next() {
                Some(c @ ('+' | '-' | ' ')) => (c, &raw[1..]),
                _ => (' ', raw),
            };
            let (kind, old, new) = match sigil {
                '+' => {
                    new_left = new_left.saturating_sub(1);
                    new_no += 1;
                    (LineKind::Add, None, Some(new_no))
                }
                '-' => {
                    old_left = old_left.saturating_sub(1);
                    old_no += 1;
                    (LineKind::Del, Some(old_no), None)
                }
                _ => {
                    old_left = old_left.saturating_sub(1);
                    new_left = new_left.saturating_sub(1);
                    old_no += 1;
                    new_no += 1;
                    (LineKind::Context, Some(old_no), Some(new_no))
                }
            };
            if let (Some(f), Some(h)) = (file.as_mut(), hunk.as_mut()) {
                match kind {
                    LineKind::Add => f.adds += 1,
                    LineKind::Del => f.dels += 1,
                    LineKind::Context => {}
                }
                h.lines.push(DiffLine {
                    kind,
                    text: rest.to_string(),
                    old_no: old,
                    new_no: new,
                });
            }
            continue;
        }
        if raw.starts_with('\\') {
            continue;
        }
        if let Some(rest) = raw.strip_prefix("diff --git ") {
            finish_file(&mut out, &mut file, &mut hunk);
            let path = rest.rsplit_once(" b/").map_or(rest, |(_, b)| b);
            file = Some(OverlayFile::new(path));
        } else if let Some(f) = file.as_mut() {
            if let Some(blobs) = raw.strip_prefix("index ") {
                f.identity = blobs.to_string();
            } else if raw.starts_with("new file mode") {
                f.status = FileStatus::Added;
            } else if raw.starts_with("deleted file mode") {
                f.status = FileStatus::Deleted;
            } else if let Some(from) = raw.strip_prefix("rename from ") {
                f.status = FileStatus::Renamed;
                f.note = Some(format!("renamed from {from}"));
            } else if let Some(to) = raw.strip_prefix("rename to ") {
                f.path = to.to_string();
            } else if raw.starts_with("Binary files") || raw.starts_with("GIT binary patch") {
                f.binary = true;
                f.note = Some("binary file".to_string());
            } else if let Some(path) = raw.strip_prefix("+++ b/") {
                f.path = path.to_string();
            } else if raw.starts_with("@@ ") {
                finish_hunk(&mut file, &mut hunk);
                if let Some((os, oc, ns, nc)) = hunk_spans(raw) {
                    old_no = os.saturating_sub(1);
                    new_no = ns.saturating_sub(1);
                    // A zero-length side starts after the line the header names.
                    if oc == 0 {
                        old_no = os;
                    }
                    if nc == 0 {
                        new_no = ns;
                    }
                    old_left = oc;
                    new_left = nc;
                    hunk = Some(OverlayHunk {
                        header: raw.to_string(),
                        lines: Vec::new(),
                    });
                }
            }
        }
    }
    finish_file(&mut out, &mut file, &mut hunk);
    out
}

/// A file git does not track: shown as one hunk of additions.
pub fn untracked_file(path: &str, content: Option<&str>) -> OverlayFile {
    let mut file = OverlayFile::new(path);
    file.status = FileStatus::Untracked;
    let Some(content) = content else {
        file.binary = true;
        file.note = Some("untracked, not shown (binary or too large)".to_string());
        return file;
    };
    let lines: Vec<DiffLine> = content
        .lines()
        .enumerate()
        .map(|(i, l)| DiffLine {
            kind: LineKind::Add,
            text: l.to_string(),
            old_no: None,
            new_no: Some(i as u32 + 1),
        })
        .collect();
    file.adds = lines.len();
    if !lines.is_empty() {
        file.hunks.push(OverlayHunk {
            header: format!("@@ -0,0 +1,{} @@", lines.len()),
            lines,
        });
    }
    file
}

/// A turn's diff card as an overlay file. The card keeps a bounded number of lines per hunk, so
/// a longer change says how many it left out.
pub fn from_snapshot(snap: &DiffFileSnapshot) -> OverlayFile {
    let mut file = OverlayFile::new(&snap.path);
    file.status = match snap.kind.as_str() {
        "created" => FileStatus::Added,
        "deleted" => FileStatus::Deleted,
        _ => FileStatus::Modified,
    };
    file.binary = snap.binary;
    file.adds = snap.adds;
    file.dels = snap.dels;
    if snap.binary {
        file.note = Some("binary file".to_string());
        return file;
    }
    for h in &snap.hunks {
        let (mut old_no, mut new_no) = hunk_spans(&h.header).map_or((0, 0), |(os, _, ns, _)| {
            (os.saturating_sub(1), ns.saturating_sub(1))
        });
        let lines = h
            .lines
            .iter()
            .map(|l| {
                let (sigil, rest) = l.split_at(l.chars().next().map_or(0, char::len_utf8));
                match sigil {
                    "+" => {
                        new_no += 1;
                        DiffLine {
                            kind: LineKind::Add,
                            text: rest.to_string(),
                            old_no: None,
                            new_no: Some(new_no),
                        }
                    }
                    "-" => {
                        old_no += 1;
                        DiffLine {
                            kind: LineKind::Del,
                            text: rest.to_string(),
                            old_no: Some(old_no),
                            new_no: None,
                        }
                    }
                    _ => {
                        old_no += 1;
                        new_no += 1;
                        DiffLine {
                            kind: LineKind::Context,
                            text: rest.to_string(),
                            old_no: Some(old_no),
                            new_no: Some(new_no),
                        }
                    }
                }
            })
            .collect();
        file.hunks.push(OverlayHunk {
            header: h.header.clone(),
            lines,
        });
    }
    if snap.skipped_lines > 0 {
        file.note = Some(format!(
            "{} more changed lines are not kept for this view; the working-tree source shows them all",
            snap.skipped_lines
        ));
    }
    file
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,4 +1,3 @@ fn main\n keep\n-old line\n+new line\n--- a dashed deletion\n tail\ndiff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+one\n+two\ndiff --git a/gone.txt b/gone.txt\ndeleted file mode 100644\n--- a/gone.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n";

    #[test]
    fn files_hunks_and_numbers_are_read() {
        let files = parse_unified(SAMPLE);
        assert_eq!(files.len(), 3);
        let a = &files[0];
        assert_eq!(
            (a.path.as_str(), a.status),
            ("src/a.rs", FileStatus::Modified)
        );
        assert_eq!((a.adds, a.dels), (1, 2));
        let lines = &a.hunks[0].lines;
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[1].old_no, Some(2));
        assert_eq!(lines[2].new_no, Some(2));
        assert_eq!(lines[3].text, "-- a dashed deletion");
        assert_eq!(lines[3].kind, LineKind::Del);
        assert_eq!(lines[4].new_no, Some(3));
        assert_eq!(files[1].status, FileStatus::Added);
        assert_eq!(files[1].hunks[0].lines[1].new_no, Some(2));
        assert_eq!(files[2].status, FileStatus::Deleted);
        assert_eq!(files[2].dels, 1);
    }

    #[test]
    fn renames_and_binaries_carry_a_note() {
        let files = parse_unified(
            "diff --git a/x b/y\nsimilarity index 90%\nrename from x\nrename to y\ndiff --git a/i.png b/i.png\nBinary files a/i.png and b/i.png differ\n",
        );
        assert_eq!(files[0].status, FileStatus::Renamed);
        assert_eq!(files[0].path, "y");
        assert_eq!(files[0].note.as_deref(), Some("renamed from x"));
        assert!(files[1].binary);
    }

    #[test]
    fn an_untracked_file_is_all_additions() {
        let f = untracked_file("n.txt", Some("a\nb\nc\n"));
        assert_eq!((f.adds, f.dels), (3, 0));
        assert_eq!(f.hunks[0].header, "@@ -0,0 +1,3 @@");
        assert!(untracked_file("b", None).binary);
    }

    #[test]
    fn the_fingerprint_changes_with_the_content() {
        let a = parse_unified(SAMPLE);
        let b = parse_unified(&SAMPLE.replace("new line", "newer line"));
        assert_ne!(a[0].fingerprint(), b[0].fingerprint());
        assert_eq!(a[1].fingerprint(), b[1].fingerprint());
    }

    #[test]
    fn a_turn_card_keeps_line_numbers_and_flags_what_it_dropped() {
        let snap = DiffFileSnapshot {
            path: "z.rs".into(),
            kind: "modified".into(),
            binary: false,
            adds: 1,
            dels: 1,
            hunks: vec![crate::DiffHunkSnapshot {
                header: "@@ -10,2 +10,2 @@".into(),
                lines: vec![" ctx".into(), "-gone".into(), "+here".into()],
            }],
            skipped_lines: 4,
        };
        let f = from_snapshot(&snap);
        let l = &f.hunks[0].lines;
        assert_eq!((l[0].old_no, l[0].new_no), (Some(10), Some(10)));
        assert_eq!(l[1].old_no, Some(11));
        assert_eq!(l[2].new_no, Some(11));
        assert!(f.note.unwrap().contains("4 more"));
    }
}
