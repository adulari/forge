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

/// One line `GET /api/blame` attributes to an agent turn.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct BlameLine {
    #[serde(default)]
    pub line: usize,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub turn: i64,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub prompt_summary: Option<String>,
    #[serde(default)]
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct BlameResponse {
    #[serde(default)]
    pub lines: Vec<BlameLine>,
}

/// One row of `GET /api/workspace/entries`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct WireEntry {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: String,
    /// `directory` | `file` | `symlink`.
    #[serde(default)]
    pub kind: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct WireEntries {
    #[serde(default)]
    pub entries: Vec<WireEntry>,
    #[serde(default)]
    pub truncated: usize,
}

/// `GET /api/workspace/file`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct WireFile {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub hash: String,
    #[serde(default)]
    pub read_only: bool,
}

/// A background shell as `GET /api/sessions/{id}/jobs` lists it.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct WireJob {
    #[serde(default)]
    pub id: u32,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub started_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct WireJobs {
    #[serde(default)]
    pub jobs: Vec<WireJob>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct WireTail {
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub text: String,
}
