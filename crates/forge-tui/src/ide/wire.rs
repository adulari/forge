//! Client-side views of the daemon routes IDE mode reads. Every field defaults, so an older or
//! newer daemon still parses; the daemon's own types are `Serialize`-only.

/// One file of `GET /api/projects/changes`.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct WireChangedFile {
    #[serde(default)]
    pub path: String,
    /// `created` | `modified` | `deleted` | `renamed`.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub adds: usize,
    #[serde(default)]
    pub dels: usize,
    #[serde(default)]
    pub content_hash: String,
}

/// One session (or the synthetic `unattributed` group) of `GET /api/projects/changes`.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct WireSessionChanges {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub worktree: Option<String>,
    #[serde(default)]
    pub waiting: bool,
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub files: Vec<WireChangedFile>,
    #[serde(default)]
    pub truncated: usize,
}

/// `GET /api/projects/changes?cwd=`.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct ProjectChanges {
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub sessions: Vec<WireSessionChanges>,
}

/// The id of the synthetic group for files no session is recorded as having edited.
pub const UNATTRIBUTED: &str = "unattributed";

/// One row of `GET /api/git/status`.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct GitRow {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub orig_path: Option<String>,
    #[serde(default)]
    pub adds: usize,
    #[serde(default)]
    pub dels: usize,
    #[serde(default)]
    pub binary: bool,
}

/// `GET /api/git/status?session=`.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct GitStatus {
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub base_branch: Option<String>,
    #[serde(default)]
    pub staged: Vec<GitRow>,
    #[serde(default)]
    pub unstaged: Vec<GitRow>,
    #[serde(default)]
    pub untracked: Vec<GitRow>,
    #[serde(default)]
    pub conflicted: Vec<GitRow>,
    #[serde(default)]
    pub truncated: usize,
    #[serde(default)]
    pub upstream: Option<String>,
    #[serde(default)]
    pub ahead: usize,
    #[serde(default)]
    pub behind: usize,
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub has_remote: bool,
}

/// One `@@` hunk of the diff routes: `lines` keep their gutter character as the first byte.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct WireHunk {
    #[serde(default)]
    pub header: String,
    #[serde(default)]
    pub lines: Vec<String>,
}

/// One file of `GET /api/git/diff` or `GET /api/sessions/{id}/diff`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct WireDiffFile {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub orig_path: Option<String>,
    #[serde(default)]
    pub binary: bool,
    #[serde(default)]
    pub adds: usize,
    #[serde(default)]
    pub dels: usize,
    #[serde(default)]
    pub hunks: Vec<WireHunk>,
    #[serde(default)]
    pub skipped_lines: usize,
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub total_lines: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct WireDiff {
    #[serde(default)]
    pub files: Vec<WireDiffFile>,
}
