//! Mirrors of `forge serve`'s wire types, deserialized leniently (every field defaults) so the
//! board tolerates older and newer daemons. The daemon's own types are `Serialize`-only; these are
//! the client-side views, exactly as `forge attach` keeps its own.

/// One row of `GET /api/sessions`. Field names are the daemon's; every field defaults so a row
/// from an older daemon still parses.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct FleetRow {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub worktree: Option<String>,
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub waiting: bool,
    #[serde(default)]
    pub last_turn_outcome: Option<String>,
    #[serde(default)]
    pub last_stop_reason: Option<String>,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub context_tokens: u64,
    #[serde(default)]
    pub context_limit: Option<u32>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub last_activity: i64,
    /// No input path at all (a terminal session that published no control channel).
    #[serde(default)]
    pub read_only: bool,
    /// Runs in a terminal rather than hosted by the daemon; archive/mode are unavailable.
    #[serde(default)]
    pub terminal: bool,
}

/// One row of `GET /api/sessions/past` — persisted, not running.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct PastRow {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub worktree: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub message_count: i64,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub last_activity: i64,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub preview: Option<String>,
}

/// One finalized transcript line with its provenance (`user` | `assistant` | `tool` | `system`).
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct TranscriptRow {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub tool: Option<String>,
    /// `"ok"` / `"failed"` on a tool result row.
    #[serde(default)]
    pub meta: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct Task {
    #[serde(default)]
    pub title: String,
    /// `pending` | `in_progress` | `done`.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub assignee: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct Subagent {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub last: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default = "default_true")]
    pub ok: bool,
    #[serde(default)]
    pub cost: f64,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct QOption {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct DiffFile {
    #[serde(default)]
    pub path: String,
    /// `created` | `modified` | `deleted`.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub binary: bool,
    #[serde(default)]
    pub adds: usize,
    #[serde(default)]
    pub dels: usize,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct DiffCard {
    #[serde(default)]
    pub pending: bool,
    #[serde(default)]
    pub files: Vec<DiffFile>,
    #[serde(default)]
    pub skipped_files: usize,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct PlanStep {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct PlanCard {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub steps: Vec<PlanStep>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct WorkflowCard {
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub phases: Vec<String>,
    #[serde(default)]
    pub logs: Vec<String>,
    #[serde(default)]
    pub finished_ok: Option<bool>,
    #[serde(default)]
    pub summary: Option<String>,
}

/// The subset of the daemon's per-session `Snapshot` the board renders. A separate
/// `Deserialize` view (the daemon's type is `Serialize`-only) so the board tolerates fields
/// coming and going across versions.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct LiveSnapshot {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub worktree: Option<String>,
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub last_turn_outcome: Option<String>,
    #[serde(default)]
    pub last_stop_reason: Option<String>,
    #[serde(default)]
    pub temper: String,
    #[serde(default)]
    pub permission_mode: String,
    #[serde(default)]
    pub effort: String,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub context_tokens: u64,
    #[serde(default)]
    pub context_limit: Option<u32>,
    /// Trailing edge of the in-flight reply.
    #[serde(default)]
    pub streaming: String,
    #[serde(default)]
    pub transcript: Vec<String>,
    #[serde(default)]
    pub transcript_rows: Vec<TranscriptRow>,
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub subagents: Vec<Subagent>,
    #[serde(default)]
    pub queued: Vec<String>,
    #[serde(default)]
    pub permission_prompt: Option<String>,
    #[serde(default)]
    pub question: Option<String>,
    #[serde(default)]
    pub question_options: Vec<QOption>,
    #[serde(default)]
    pub question_allow_other: bool,
    #[serde(default)]
    pub diff: Option<DiffCard>,
    #[serde(default)]
    pub plan: Option<PlanCard>,
    #[serde(default)]
    pub workflow: Option<WorkflowCard>,
    #[serde(default)]
    pub prompt_seq: u64,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub revision: Option<u64>,
    #[serde(default)]
    pub closed: bool,
}

impl LiveSnapshot {
    /// Transcript rows with provenance; synthesized from the plain transcript for a pre-v9 host.
    pub fn rows(&self) -> Vec<TranscriptRow> {
        if !self.transcript_rows.is_empty() {
            return self.transcript_rows.clone();
        }
        self.transcript
            .iter()
            .map(|t| TranscriptRow {
                kind: "assistant".into(),
                text: t.clone(),
                ..Default::default()
            })
            .collect()
    }

    /// A permission prompt or question is blocking the turn.
    pub fn waiting(&self) -> bool {
        self.permission_prompt.is_some() || self.question.is_some()
    }
}

/// One file of `GET /api/git/status`.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct GitFile {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub adds: usize,
    #[serde(default)]
    pub dels: usize,
}

/// `GET /api/git/status?session=<id>`.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct GitInfo {
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub base_branch: Option<String>,
    #[serde(default)]
    pub staged: Vec<GitFile>,
    #[serde(default)]
    pub unstaged: Vec<GitFile>,
    #[serde(default)]
    pub untracked: Vec<GitFile>,
    #[serde(default)]
    pub truncated: usize,
}

impl GitInfo {
    pub fn dirty_count(&self) -> usize {
        self.staged.len() + self.unstaged.len() + self.untracked.len() + self.truncated
    }
}

/// One row of `GET /api/history?include_tools=1` (newest first on the wire).
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct HistoryRow {
    #[serde(default)]
    pub seq: i64,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub tool: Option<String>,
    /// `"call"` | `"result"` on tool rows.
    #[serde(default)]
    pub tool_phase: Option<String>,
}
