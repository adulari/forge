//! The plain-data types the remote-control projection produces: the snapshot itself, its diff
//! card, live tail and workflow view, and the provenance tag on each transcript row.

use super::*;

/// How many landed file diffs a turn retains for the remote diff card. Older files are evicted
/// into a "+N more files" count; the full history is always in the TUI scrollback + tool results.
pub const REMOTE_DIFF_MAX_FILES: usize = 10;

/// One `@@` hunk of a remotely-projected file diff: the unified-diff header (old/new line spans)
/// plus its body lines, each prefixed `+` / `-` / ` ` (gutter as the first character).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiffHunkSnapshot {
    pub header: String,
    pub lines: Vec<String>,
}

/// One file's change, projected for the remote wire (see `render::diff_file_snapshot`): full
/// add/del counts, capped hunks, and how many hunk lines were dropped over the cap.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiffFileSnapshot {
    pub path: String,
    /// "created" | "modified" | "deleted".
    pub kind: String,
    /// Non-UTF-8 target: no textual hunks, the page shows a one-line "binary file" summary.
    pub binary: bool,
    pub adds: usize,
    pub dels: usize,
    pub hunks: Vec<DiffHunkSnapshot>,
    pub skipped_lines: usize,
}

/// The remote diff card: either the ONE proposed change a pending write permission would apply
/// (`pending: true` — "what will this touch" before Allow), or every change that landed this
/// turn (`pending: false`, capped to [`REMOTE_DIFF_MAX_FILES`] files).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiffSnapshot {
    pub pending: bool,
    pub files: Vec<DiffFileSnapshot>,
    pub skipped_files: usize,
}

/// A plain-data view of the live state, produced by [`App::remote_snapshot`] and mapped into the
/// `remote::Snapshot` JSON by the render loop. Defined here (in forge-tui) so the pure render
/// crate owns the projection without depending on the server module.
#[derive(Debug, Clone, Default)]
pub struct RemoteSnapshot {
    pub busy: bool,
    pub done: bool,
    /// How the most recent turn ended. `None` until one has. Remote clients read this to tell a
    /// turn that finished the work from one that failed or produced nothing at all — `busy` and
    /// `done` alone report both as an ordinary idle session.
    pub last_stop_reason: Option<forge_types::StopReason>,
    pub temper: String,
    pub effort: Option<forge_types::EffortLevel>,
    pub tier: Option<String>,
    pub model: String,
    pub cost_usd: f64,
    pub context_tokens: u64,
    pub context_limit: Option<u32>,
    pub streaming: String,
    /// UTF-16 units before `streaming`; the turn's phase label ("" idle); its start (Unix ms).
    pub streaming_offset: u64,
    pub activity: String,
    pub turn_started_at: u64,
    /// The expert-on-call line (`expert: <model> · 1 consult · 2.1% of weekly window`), if the
    /// session has consulted one.
    pub expert: Option<String>,
    pub transcript: Vec<String>,
    /// The same lines as `transcript`, each carrying the provenance its emitter recorded (v9) —
    /// see [`TranscriptRow`]. Same length and order.
    pub transcript_rows: Vec<TranscriptRow>,
    /// Rows ever pushed into the ring (see `App::transcript_total`).
    pub transcript_total: u64,
    pub tasks: Vec<forge_types::TodoItem>,
    pub subagents: Vec<SubagentSnapshot>,
    pub queued: Vec<String>,
    pub permission_prompt: Option<String>,
    /// The standing rule the pending permission prompt offers, if any (approvals learn).
    pub permission_suggestion: Option<crate::RuleSuggestion>,
    /// The repository this Home-scope session keeps working in, offered as a one-tap move. Only
    /// while idle: moving waits for the session lock, which a running turn holds.
    pub project_suggestion: Option<crate::project_offer::Suggestion>,
    pub question: Option<String>,
    pub question_options: Vec<crate::QChoice>,
    pub question_allow_other: bool,
    /// The whole `ask_user` form (v11 additive): every question with its options, multi-select
    /// flag and note allowance. `question`/`question_options` mirror the CURRENT one for older
    /// clients.
    pub question_form: Option<Vec<forge_types::Question>>,
    pub question_index: usize,
    /// The structured diff card (v7) — see [`DiffSnapshot`]. `None` when nothing changed.
    pub diff: Option<DiffSnapshot>,
    /// The most recent plan proposal (v7), for the remote plan-approval card.
    pub plan: Option<forge_types::PlanProposal>,
    /// The predicted next prompt (v8), if any — the page shows it as the composer's placeholder.
    pub suggested_prompt: Option<String>,
    /// The live (or just-finished) workflow run this turn (v8.1) — see [`WorkflowRemote`].
    pub workflow: Option<WorkflowRemote>,
    /// The rolling output of the foreground tool call in flight, if one is reporting.
    pub live_tail: Option<LiveTailRemote>,
}

/// Remote projection of a running call's live output tail (ANSI already stripped).
#[derive(Debug, Clone, Default)]
pub struct LiveTailRemote {
    pub tool: String,
    pub lines: Vec<String>,
    pub elapsed_ms: u64,
}

/// Remote projection of the dedicated workflow view (v8.1): phases + narration + finish state.
/// Agent rows ride in [`RemoteSnapshot::subagents`] with `phase` attached, so this stays a
/// cheap clone per frame (no per-row transcript).
#[derive(Debug, Clone, Default)]
pub struct WorkflowRemote {
    pub active: bool,
    pub name: Option<String>,
    pub phases: Vec<String>,
    /// Tail of the script's `log()` narration (bounded to [`WORKFLOW_REMOTE_LOG_TAIL`]).
    pub logs: Vec<String>,
    pub finished_ok: Option<bool>,
    pub summary: Option<String>,
}

/// Where a finalized scrollback line came from. Scrollback is a flat list of styled lines by the
/// time it reaches the remote ring, so the emitter records this as it pushes — a remote client
/// can't recover "this was a tool result, not the model talking" from the text afterwards.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TranscriptKind {
    /// Anything that is neither a user echo, model prose, nor tool activity: warnings, errors,
    /// command feedback, banners. The default, so an untagged emitter can never masquerade as
    /// the model or a tool.
    #[default]
    System,
    User,
    Assistant,
    Tool,
    /// A background-job completion notice the harness injected as a user-role message. The row's
    /// `meta` is `"ok"` / `"failed"` on a job's header line and `"tail"` on its log lines.
    JobNotice,
    /// A harness-injected user-role message that is not a job exit; `meta` is the subtype
    /// (`wakeup`, `heartbeat`, `peer`).
    HarnessNotice,
}
