//! Pure, testable TUI state and rendering for the inline-scrollback model.
//!
//! `App` folds [`PresenterEvent`]s into two kinds of state: *transient* state rendered
//! every frame in the small pinned live region (input, statusline, the in-flight reply,
//! the permission bar), and a *flush* outbox of finalized scrollback lines that the I/O
//! shell drains and pushes into the terminal's native scrollback (`insert_before`). The
//! line builders and `render_live` are free of terminal I/O so they stay TestBackend-able.

use crate::theme::styles::{self, NoticeKind, RailKind};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line as TextLine, Span};
use ratatui::widgets::{Block, Padding, Paragraph, Wrap};
use ratatui::Frame;
use serde::{Deserialize, Serialize};

use crate::app_remote::{DiffFileSnapshot, TranscriptKind};
use crate::{surface, PresenterEvent, QChoice, ReplayItem};

pub use crate::overlays::{
    MeshCandRow, MeshOverlay, MeshQuotaRow, QuotaPaceInfo, RoutingView, UsageOverlay, UsagePaceNote,
};

pub use crate::voice::{insert_voice_transcript, VoiceOverlay, VoicePhase};

// ── Palette ──────────────────────────────────────────────────────────────────
// Kept as imports here for readability; values and surface treatment are shared by every view.

pub(crate) const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The text a scrollback line contributes to the remote transcript ring. A tool-card row is laid
/// out for a terminal width — the status is pushed to the right margin with padding, and a call
/// that has not finished ends in `running`. The phone renders rows in its own width and
/// its ring is append-only (the in-place redraw the terminal gets cannot reach it), so the padded
/// running row wrapped into a stray "running" line under every call and then stayed there next to
/// the finished row. Collapse the padding and drop the transient mark; the finished row still
/// arrives as its own line, exactly like the pre-card `↳ call` / `✓ result` pair did.
fn remote_transcript_text(plain: &str, origin: &LineOrigin) -> String {
    if origin.kind != TranscriptKind::Tool {
        return plain.to_string();
    }
    let collapsed = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    // The disclosure triangle at the end of a card row is for the terminal's mouse.
    let collapsed = collapsed
        .strip_suffix(" ▸")
        .or_else(|| collapsed.strip_suffix(" ▾"))
        .map(str::to_string)
        .unwrap_or(collapsed);
    // Only the call row carries the running marker; a result row's own text is left alone.
    if origin.meta.is_some() {
        return collapsed;
    }
    collapsed
        .strip_suffix(" running")
        .map(str::to_string)
        .unwrap_or(collapsed)
}

/// The text of an assistant label row on the remote wire (a stable word, not what the terminal
/// shows: the supervisor strips it from a worker's final reply).
const REMOTE_ASSISTANT_HEADER: &str = "⚒ forge";

/// Two idle Esc presses closer together than this open the rewind picker.
pub const ESC_DOUBLE_TAP: std::time::Duration = std::time::Duration::from_millis(700);

/// Pinned live-region geometry. Fixed at terminal creation (ratatui inline viewports do
/// not resize at runtime), so kept small: only the in-flight reply edge, the permission
/// bar, the input box, and the statusline are pinned — finalized lines flow to scrollback.
pub const STREAM_PREVIEW_H: u16 = 3;
/// A permission prompt gets a compact summary plus an untruncated decision row.  Keeping the
/// controls on their own row means a long tool name or a narrow terminal can never hide `deny`.
pub const PERMISSION_H: u16 = 2;
/// Minimum input box height (1 text row + 2 border rows). The box grows up to [`INPUT_MAX_H`] as
/// the (wrapped/multiline) input gets taller, then scrolls internally.
pub const INPUT_H: u16 = 3;
/// Max input box height: up to [`INPUT_MAX_ROWS`] visible text rows + 2 border rows.
pub const INPUT_MAX_ROWS: u16 = 6;
pub const INPUT_MAX_H: u16 = INPUT_MAX_ROWS + 2;
pub const STATUS_H: u16 = 1;
/// Fixed inline-viewport height. Detailed work stays available on demand rather than permanently
/// occupying the basic chat flow.
/// Cannot be resized at runtime — recreating the inline viewport pollutes the terminal scrollback.
pub const LIVE_H: u16 = 14;
/// Max task / running-subagent rows shown in their sticky panels before summarizing the overflow.
const TASKS_PANEL_MAX: usize = 6;
/// Max entry rows shown in the sticky activity panel before summarizing the overflow.
const ACTIVITY_PANEL_MAX: usize = 8;
/// Max styled lines retained for the "main chat" full-screen view (older lines drop off the front;
/// the terminal's native scrollback still holds the complete history).
const MAIN_LOG_MAX: usize = 5000;

/// What a paste-block placeholder stands in for: a chunk of pasted text (substituted back into the
/// prompt on submit) or an attached image (sent out-of-band as vision input, the placeholder
/// stripped from the text).
#[derive(Debug, Clone)]
enum PasteKind {
    Text(String),
    Image(forge_types::ImageAttachment),
}

/// An attachment shown inline in the input as a one-line placeholder (e.g. `[pasted text (3 lines)]`
/// or `[image (PNG 800x600)]`). The placeholder is deletable as a single unit and is resolved on
/// submit: text is substituted back in, images are pulled out as vision input.
#[derive(Debug, Clone)]
struct PasteBlock {
    /// The exact placeholder string inserted into `input`.
    placeholder: String,
    kind: PasteKind,
}

/// Live state for the compaction progress band. Progress is indeterminate (a single summarizer
/// call with no measurable fraction), so it's eased toward ~95% over an expected duration and
/// snapped to 100% when [`CompactionFinished`](crate::PresenterEvent::CompactionFinished) clears
/// it — the standard "honest indeterminate" progress UX.
#[derive(Debug, Clone, Default)]
pub struct CompactionState {
    /// `App::tick` value when compaction started; elapsed ≈ (tick - start_tick) frames.
    pub start_tick: usize,
    /// Whether this was a silent auto-compact (vs an explicit `/compact`).
    pub auto: bool,
}

/// The most specific phase the TUI can prove from presenter events during a turn. This stays
/// intentionally surface-local: providers already emit routing, reasoning, text, and tool events;
/// the TUI turns those facts into a useful live heartbeat without persisting UI-only noise in the
/// conversation transcript.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum TurnPhase {
    #[default]
    Preparing,
    ContactingModel,
    ReceivingProvider,
    Reasoning,
    Responding,
    RunningTool,
    ProcessingToolResult,
    Auxiliary,
    Recovering,
    Coordinating,
    Compacting,
    Finalizing,
}

impl TurnPhase {
    const fn label(self) -> &'static str {
        match self {
            Self::Preparing => "preparing turn",
            Self::ContactingModel => "waiting for model",
            Self::ReceivingProvider => "receiving provider stream",
            Self::Reasoning => "model reasoning",
            Self::Responding => "streaming response",
            Self::RunningTool => "running tool",
            Self::ProcessingToolResult => "processing tool result",
            Self::Auxiliary => "auxiliary model work",
            Self::Recovering => "recovering provider",
            Self::Coordinating => "coordinating work",
            Self::Compacting => "compacting context",
            Self::Finalizing => "finalizing turn",
        }
    }
}

/// Live, per-turn observability derived from real presenter events. The counters make hidden model
/// reasoning visibly alive, while `last_event_tick` distinguishes an active stream from a request
/// that has produced no UI events for a long time.
#[derive(Debug, Clone, Default)]
struct TurnActivity {
    phase: TurnPhase,
    detail: String,
    /// The model at the actual provider-call boundary. This normally matches `routing`, but
    /// keeping the request model here makes retries/planner calls accurate even if a routing
    /// surface is delayed or omitted.
    model: Option<String>,
    last_event_tick: usize,
    reasoning_chars: usize,
    auxiliary_chars: usize,
    response_chars: usize,
    provider_events: usize,
    tool_calls: usize,
    /// Tool call whose arguments are streaming right now: (display name, cumulative chars).
    composing_tool: Option<(String, usize)>,
    /// Estimated tokens of hidden model thinking (providers that stream no thinking text).
    thinking_tokens: u64,
}

/// A bridge tool name as the user knows it: the `mcp__<server>__` namespace is plumbing.
fn tool_display_name(name: &str) -> &str {
    name.strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"))
        .map_or(name, |(_, tool)| tool)
}

/// What a remote `Answer` text did to the open form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteAnswer {
    /// The whole form is answered; deliver these.
    Complete(Vec<forge_types::Answer>),
    /// Answered the current question of a multi-question form; the next one is now current.
    Partial,
    /// Not a valid answer for the current question.
    Invalid,
}

/// All state the TUI needs to render the pinned live region, plus the scrollback outbox.
#[derive(Debug, Clone, Default)]
pub struct App {
    pub session_id: String,
    pub routing: Option<RoutingView>,
    /// The rung mesh chose for the routed model, when it had a measured ladder. Falls back to
    /// `effort` (the session pin) for display, mirroring exactly what the request sends.
    pub routed_effort: Option<forge_types::EffortLevel>,
    /// While `Some`, the mesh is recovering from a failed model attempt — drives the animated
    /// status indicator. The bool is `retrying`: true = same-model retry (backoff), false =
    /// searching for a replacement. Set on a `ModelSearch` event, cleared the moment real output
    /// (assistant text / a tool call) arrives, so it shows only during the recovery, never after.
    pub model_search: Option<(String, bool)>,
    pub cost_usd: f64,
    /// Live token counter (tui-token-counter.md): session totals + current context fill.
    pub session_in: u64,
    /// Prompt tokens served from provider cache (subset of `session_in`). `None` when no call in
    /// the session came from a provider that reports cache hits — distinct from `Some(0)`.
    pub session_cached_in: Option<u64>,
    pub session_out: u64,
    pub context_tokens: u64,
    pub context_limit: Option<u32>,
    /// Wall-clock seconds the current turn has been running; updated live by the I/O shell each
    /// frame while busy and left frozen at the final value once the turn ends (reset on the next
    /// turn). Drives the `⧖ Ns` turn timer in the statusline.
    pub turn_elapsed_secs: u64,
    /// Input/output tokens attributed to the current/last turn (session totals minus the snapshot
    /// taken at turn start). Shown alongside the timer; the session totals stay on their own.
    pub turn_in: u64,
    pub turn_cached_in: Option<u64>,
    pub turn_out: u64,
    /// Live output-token throughput for the statusline.
    pub throughput: crate::throughput::ThroughputMeter,
    /// Session in/out totals captured at turn start, so `turn_in/out` are deltas from here.
    turn_base_in: u64,
    turn_base_cached_in: Option<u64>,
    turn_base_out: u64,
    /// True once at least one turn has started this session — so the turn timer/token segment shows
    /// the last turn's frozen stats even for a sub-second turn (where `turn_elapsed_secs` is 0).
    turn_ran: bool,
    /// Current phase, recent-event heartbeat, and streamed-work counters for the live progress row.
    turn_activity: TurnActivity,
    /// Set while compaction is running, driving the animated progress band. `None` otherwise.
    pub compaction: Option<CompactionState>,
    pub done: bool,
    /// Why the last turn ended; `None` before any turn completes or for lifecycle-only Done events.
    pub last_stop_reason: Option<forge_types::StopReason>,
    /// Turns finished so far, bumped exactly where `last_stop_reason` is set. A daemon watching the
    /// snapshot compares counts, so a turn that starts and ends between two polls is still seen.
    pub turns_finished: u64,
    /// The active operating temper label (e.g. "Guarded"), shown in the statusline.
    pub temper: String,
    /// The active reasoning-effort pin, when set by config or `/effort`.
    pub effort: Option<forge_types::EffortLevel>,
    /// True while the effort slider popup is visible above the input bar (Ctrl+R).
    pub effort_slider: bool,
    /// An in-flight `ask_user` form (one or more questions), answered in the live region with
    /// arrow keys / digits / Space; see `question_form.rs`.
    pub form: Option<crate::QuestionForm>,
    /// A pending permission question shown while the loop blocks on the user's y/n.
    pub prompt: Option<String>,
    /// The text of a pending AskUserQuestion (set by `set_question`, cleared on resolve), so a
    /// remote-control snapshot can tell the phone what's being asked without the private options.
    pub question_prompt: Option<String>,
    /// The current input-line buffer (shown in the input box).
    pub input: String,
    /// Byte offset of the text cursor within `input`. Always on a char boundary.
    pub input_cursor: usize,
    /// An AI-predicted next prompt (`SuggestionReady`), shown as dim ghost text when `input` is
    /// empty and idle; Tab moves it into `input` (editable, never auto-sent). Cleared at the start
    /// of every new turn (`on_turn_start`) and on session switch (`clear_transcript`) so a stale
    /// suggestion from a prior turn/session can never linger.
    pub suggested_prompt: Option<String>,
    /// The current *partial* (un-flushed, newline-free) line of the streaming reply.
    pub streaming: String,
    /// Accumulated reasoning/thinking text, flushed as a dim block before the answer.
    reasoning: String,
    /// True once the `⚒ forge` header for the in-flight reply has been flushed.
    streaming_active: bool,
    /// True while a turn is running (drives the thinking spinner).
    pub busy: bool,
    /// When an idle Esc was last pressed: a second press inside [`ESC_DOUBLE_TAP`] opens the
    /// rewind picker. The statusline hint says so while it is armed.
    pub esc_armed: Option<std::time::Instant>,
    /// When an idle Ctrl-C on an empty input armed the exit; a second press inside
    /// [`idle_interrupt::QUIT_DOUBLE_TAP`] quits.
    pub quit_armed: Option<std::time::Instant>,
    /// A startup phase label ("starting session…") shown with the spinner before the session
    /// exists, so the terminal is never blank while the store and providers come up.
    pub loading: Option<&'static str>,
    /// Prompts the user typed while busy, queued to run after the current turn (shown in the
    /// statusline as "⏳ N queued"). Maintained by the I/O shell.
    pub queued: Vec<String>,
    /// Animation tick, advanced by the render loop while busy.
    pub tick: usize,
    /// Finalized scrollback lines, in arrival order; drained by the I/O shell.
    flush: Vec<TextLine<'static>>,
    /// Provenance for `flush`, index-aligned and never longer than it. Emitters that know where
    /// their lines came from record it through [`App::tag_flush`]; anything pushed directly is
    /// sealed as [`TranscriptKind::System`] when the next tagged emitter runs or on drain, so a
    /// missing tag degrades to "system note" instead of misattributing a line.
    flush_origins: Vec<LineOrigin>,
    /// Subagents in the current `spawn_agents` batch (RFC subagent-orchestration). Running rows
    /// animate with a spinner in the live preview; on completion each becomes a scrollback
    /// branch line, and the whole group folds (header + branches + footer) when all finish.
    /// Rows are retained across batches within a turn (see the `SubagentStart` handler) so a
    /// multi-phase workflow's earlier phases stay reachable via Ctrl+O — `subagent_batch_start`
    /// is the index where the *currently open* batch begins, so its scrollback footer sums only
    /// this batch's rows, not every retained row from earlier phases too.
    pub(crate) subagents: Vec<SubRow>,
    subagent_batch_start: usize,
    /// Per-critic rows for the live assay panel. Populated from AssayCriticRow events; cleared
    /// when the AssayReport arrives (the full report lands in scrollback instead).
    assay_critics: Vec<forge_types::AssayCriticRow>,
    /// Number of candidates being adversarially verified — shown in the panel header; `None`
    /// before the verifier phase starts or after the report arrives.
    assay_verifying: Option<usize>,
    /// The inline slash-command palette (RFC session-management-and-commands). Open while the
    /// input line starts with `/`.
    pub palette: crate::commands::Palette,
    /// The global command center, opened with Ctrl+K.
    pub command_center: crate::commands::CommandCenter,
    /// The interactive session/checkpoint picker (RFC session-management-and-commands). Modal
    /// while open; reused for `/sessions`, `/resume`, and `/checkpoints`.
    pub picker: crate::commands::Picker,
    /// Whether the sessions picker is showing archived sessions as well as active history.
    pub show_archived: bool,
    /// Candidates for the `/copy` block picker (`PickerKind::CopyBlocks`) as `(lang, text)`: index 0
    /// is the full response (lang `""`), 1.. are the fenced code blocks (lang = the fence info). A
    /// picker row's `id` is the index here; Enter copies the text to the clipboard, `w` writes it to
    /// a file (extension derived from `lang`) — both handled in the render loop.
    pub copy_candidates: Vec<(String, String)>,
    /// For the `/models` browser only: `Some(provider)` when drilled into a provider's models,
    /// `None` at the top-level provider list. Lets Esc step back a level instead of closing.
    pub models_drilled: Option<String>,
    /// When true the models picker was opened via bare `/model` (pin-mode): selecting a leaf model
    /// row pins it instead of just browsing. Reset to false when the picker closes.
    pub models_pin_mode: bool,
    /// The live task list (`update_tasks`). Kept so the sticky tasks panel stays visible during
    /// the turn (the inline scrollback copy scrolls away); cleared when the model empties the list.
    pub(crate) tasks: Vec<forge_types::TodoItem>,
    /// File-path picker for `@path` inline completion. Opens when the input contains `@…` at cursor.
    pub at_picker: crate::commands::AtPathPicker,
    /// When true, the sticky activity panel has keyboard focus: ↑↓ move the selection, Enter opens
    /// the selected entry's full-screen transcript, Esc unfocuses. Toggled by Ctrl+O.
    pub activity_focused: bool,
    /// The highlighted row in the activity panel (0 = main chat, then subagents, then critics).
    pub activity_idx: usize,
    /// Full styled transcript of the main conversation, mirrored from the scrollback outbox so the
    /// activity viewer can show "main chat" full-screen. Bounded to [`MAIN_LOG_MAX`] lines.
    main_log: Vec<TextLine<'static>>,
    /// True when the chat renders on the alternate screen (full-screen): the transcript is drawn
    /// from [`main_log`] into a scrollable region and the panels are pinned at the bottom. When
    /// false (inline mode), finalized lines flow into the terminal's native scrollback instead and
    /// only the small pinned live region is drawn. Set once at startup.
    pub fullscreen: bool,
    /// Full-screen transcript scroll offset, in wrapped rows from the top. Only meaningful when
    /// [`fullscreen`] is set.
    pub transcript_scroll: usize,
    /// While true, the full-screen transcript auto-scrolls to the tail as new lines arrive (a
    /// normal terminal). Paging up pauses it; paging to the bottom (or End) resumes it.
    pub transcript_follow: bool,
    /// The in-loop activity transcript viewer, open while `Some` (full-screen mode only). It renders
    /// over the whole frame using the MAIN terminal — no nested alternate screen — so it can never
    /// collide with the chat. Inline mode uses the separate-alt-screen `run_transcript_viewer`.
    pub viewer: Option<ViewerState>,
    /// The dynamic `/config` settings editor overlay (modal while `open`).
    pub config_editor: crate::config_editor::ConfigEditor,
    /// The `/usage` overlay state.
    pub usage_overlay: UsageOverlay,
    /// The `/btw` answer card (`app_btw.rs`); `None` when no side question is on screen.
    pub btw: Option<crate::app_btw::BtwOverlay>,
    /// Question of a `/btw` card dismissed before its answer came back; that late answer is
    /// dropped instead of reopening a card that would swallow the next Esc.
    pub(crate) btw_dismissed: Option<String>,
    /// The `/mesh` routing-inspector overlay state.
    pub mesh_overlay: MeshOverlay,
    /// The `/voice` recording overlay (also opened by Ctrl+V), while open. The real system
    /// resources (recorder thread, whisper model) live in the render loop, not here — this is
    /// pure rendering data so `App` stays `Clone + Default` (see `VoiceOverlay`).
    pub voice: Option<VoiceOverlay>,
    /// The dedicated full-screen workflow view (docs/rfcs/forge-workflow.md). Auto-opens on
    /// `WorkflowStarted`; while a workflow is active it owns every `Subagent*` event — those rows
    /// render here, never in the sticky subagent activity panel.
    pub workflow: crate::workflow_view::WorkflowView,
    /// True while remote control is active (a browser can drive the session via `/remote`). Shown
    /// as a `◉ remote` segment in the statusline so it's visible at a glance that the session is
    /// remotely controllable.
    pub remote_active: bool,
    /// Current git branch, kept fresh by `refresh_git_location`. Shown by the `GitBranch` widget.
    pub git_branch: Option<String>,
    /// Repository name (the directory outside git), kept fresh the same way. Shown by `RepoName`.
    pub repo_name: Option<String>,
    /// The workspace whose repository and branch the statusline follows.
    pub(crate) git_workspace: Option<std::path::PathBuf>,
    /// What the statusline's location chip shows; `None` outside a git checkout.
    pub(crate) git_location: Option<crate::git_location::GitLocation>,
    pub(crate) git_checked_at: Option<std::time::Instant>,
    /// Number of connected MCP servers (from the last `McpStatus` event). Shown by `McpStatus` widget.
    pub mcp_count: usize,
    /// The subscription window currently projected closest to exhaustion (from the latest
    /// `QuotaPace` event), if any history-derived projection is available yet. Shown by the
    /// `QuotaPace` statusline widget; `None` degrades that widget to render nothing (no panic,
    /// no fake reading) when there isn't enough quota history for the active provider(s).
    pub quota_pace: Option<QuotaPaceInfo>,
    /// Statusline layout loaded from config at startup. Drives `render_statusline`.
    pub statusline_config: forge_config::StatuslineConfig,
    /// Latest stdout from each shell-backed `Custom` widget, keyed by its `shell` command string
    /// (the command IS the id — two widgets configured with the same command share a cache entry,
    /// which is harmless). Populated by a periodic background refresh in the render loop, never
    /// blocking it: rendering only ever reads this cache.
    pub custom_widget_cache: std::collections::HashMap<String, String>,
    /// A bounded ring buffer of the most recent finalized scrollback lines (text + provenance),
    /// so a remote-control snapshot can show the phone the tail of the conversation. Kept small
    /// (the full transcript lives in the terminal's native scrollback); newest is last.
    pub recent_transcript: std::collections::VecDeque<TranscriptRow>,
    /// How many rows have ever entered [`Self::recent_transcript`]. The ring only keeps the last
    /// few, so this is what lets a remote client tell which of them it has already seen in the
    /// persisted history it paged in.
    pub transcript_total: u64,
    /// When true, model reasoning/thinking blocks are shown in scrollback. Default false (hidden).
    /// Toggled by `/thinking`.
    pub show_thinking: bool,
    /// Expandable tool cards, oldest first — full-screen only (inline mode prints into the
    /// terminal's own scrollback, which cannot be rewritten once printed).
    pub(crate) tool_cards: Vec<ToolCard>,
    /// Next card id. Monotonic, never reused, so a card is identifiable after the log ring has
    /// dropped older ones and shifted every index.
    next_card_id: u64,
    /// Rows that exist only for the remote-control transcript: a finished card is redrawn IN
    /// PLACE in `main_log`, which the phone's tail-of-conversation ring never sees, so the
    /// outcome row is handed to it separately. Drained by `drain_flush_remote`.
    remote_only: Vec<(TextLine<'static>, LineOrigin)>,
    /// Cards whose lines are queued in `flush` but not yet folded into `main_log`:
    /// `(offset within this flush, card id, line count)`. Drained by `drain_flush`, which is the
    /// only place the final `main_log` position is known.
    pending_card_marks: Vec<(usize, u64, usize)>,
    /// The card the latest `ToolResult` closed, which a following `ToolOutput` belongs to.
    last_closed_card: Option<u64>,
    /// The newest kept output, for `/output` where there is no card to open (inline mode).
    last_tool_output: Option<(String, forge_types::ToolOutputRef)>,
    /// The full-output viewer, while open (`/output`, or a card's "view full output" row).
    pub output_view: Option<OutputView>,
    /// Attachment blocks shown inline as placeholders (pasted text or images): the placeholder
    /// lives in `input`, the backing content here. On submit, `resolve_paste_blocks()` substitutes
    /// text back inline and pulls images out as vision input.
    paste_blocks: Vec<PasteBlock>,
    /// Images resolved from the last submitted prompt, stashed so the user-turn echo can show a
    /// marker line per image (the placeholder was stripped from the text). Cleared by `submit_user`.
    last_submit_images: Vec<forge_types::ImageAttachment>,
    /// Images typed with a prompt that was queued mid-turn, keyed by the queued text; they ride
    /// with that prompt when it is finally dequeued ([`Self::promote_queued_images`]).
    queued_images: Vec<(String, Vec<forge_types::ImageAttachment>)>,
    /// Images of the prompt just dequeued, waiting for the turn spawn to attach them to the session.
    ready_images: Vec<forge_types::ImageAttachment>,
    /// The write-tool preview diff awaiting its tool's outcome. `PresenterEvent::Diff` is emitted
    /// BEFORE the permission gate, so while `prompt` is armed this is exactly "what the pending
    /// Allow would touch" — the remote diff card. The following `ToolResult` resolves it: ok →
    /// moves into [`Self::turn_diffs`] (the change landed), failed/denied → dropped.
    pub(crate) pending_diff: Option<DiffFileSnapshot>,
    /// Capped projections of the file changes that actually LANDED this turn (latest edit per
    /// path, at most [`REMOTE_DIFF_MAX_FILES`] files — older ones fold into
    /// [`Self::turn_diffs_skipped`]). Cleared by `submit_user` (a new prompt starts a new turn).
    pub(crate) turn_diffs: Vec<DiffFileSnapshot>,
    /// How many landed file diffs were evicted over the [`REMOTE_DIFF_MAX_FILES`] cap this turn.
    pub(crate) turn_diffs_skipped: usize,
    /// The most recent plan proposal (`present_plan`), retained for the remote plan card — the
    /// TUI renders its card straight into scrollback, but the phone needs it as live state.
    /// Replaced by a newer proposal; cleared by `submit_user`.
    pub plan: Option<forge_types::PlanProposal>,
    /// True when the terminal window has lost focus (FocusLost). Inverted sense so the derived
    /// `Default` (false) means *focused* — terminals don't always emit an initial FocusGained.
    /// Drives a hollow/dim input cursor while another window is in front.
    pub unfocused: bool,
    /// Blink phase for the input cursor: when true the block is suppressed this frame (the "off"
    /// half of the blink). Inverted sense so `Default` (false) shows the solid block. Toggled by
    /// the render loop ~every 530ms while focused.
    pub cursor_hidden: bool,
    /// Monotonic revision of [`main_log`], bumped on every append/clear. Keys the wrap cache so a
    /// fold that *replaces* lines at the [`MAIN_LOG_MAX`] cap (len unchanged) still invalidates it.
    main_log_rev: u64,
    /// Memoized wrap of [`main_log`] for the full-screen transcript. Re-wrapping the whole log
    /// char-by-char every frame is O(transcript) and was the full-screen input/scroll lag on long
    /// conversations; this caches it so a streaming frame only re-wraps the one in-flight edge line.
    wrap_cache: std::cell::RefCell<WrapCache>,
    /// Screen geometry of the transcript area (col0, row0, width, height) captured each render, plus
    /// the scroll offset used — so a mouse event can be mapped to a wrapped-row/col in the log. `Cell`
    /// because it's written from the (immutable-`&self`) render path.
    transcript_geom: std::cell::Cell<Option<TranscriptGeom>>,
    /// Screen geometry of the floating "jump to bottom" bar (row, col0, width) when shown, for
    /// click hit-testing. `None` when at the bottom (bar hidden).
    jump_bar_geom: std::cell::Cell<Option<(u16, u16, u16)>>,
    /// Screen geometry of the floating "previous message" bar at the top of the transcript
    /// (row, col0, width) when shown, for click hit-testing.
    prev_bar_geom: std::cell::Cell<Option<(u16, u16, u16)>>,
    /// Full-screen viewer scroll geometry `(wrapped_len, body_h)` of the selected entry, written by
    /// the (immutable-`&self`) render path so `viewer_key` can re-enable follow when the user scrolls
    /// back to the bottom (it has no other way to know the wrapped length at keypress time).
    viewer_geom: std::cell::Cell<Option<(usize, u16)>>,
    /// Memoized wrap of the in-loop activity viewer's selected entry. The viewer redraws at the
    /// render loop's full rate, and rebuilding it per frame meant cloning the whole transcript and
    /// re-wrapping it char-by-char ~60×/s on long sessions.
    viewer_wrap: std::cell::RefCell<ViewerWrapCache>,
    /// Active text selection in transcript coords: (wrapped_row, col) anchor + cursor. `None` when
    /// nothing is selected. Highlighted in the transcript and copied to the clipboard on release.
    selection: Option<(TextPos, TextPos)>,
    /// Drag selection in screen cells for popups and viewers; see [`screen_select`].
    screen_sel: std::cell::Cell<Option<screen_select::ScreenSel>>,
    screen_snap: std::cell::RefCell<screen_select::ScreenSnapshot>,
    /// Framed-surface rectangles from the last draw, so a press can find the popup it landed in.
    panel_rects: std::cell::RefCell<Vec<Rect>>,
    /// Monotonic revision of the in-flight assistant reply. Unlike a byte length, this also
    /// invalidates the preview when text is replaced at an equal length.
    streaming_rev: u64,
    /// Memoized markdown render of the in-flight streaming reply. keyed on `(len, width)`. The live
    /// preview renders the partial reply as markdown (so it matches the finalized block instead of a
    /// raw unwrapped blob); re-parsing every frame would be O(reply) lag, so it's only rebuilt when
    /// new tokens arrive or the width changes.
    stream_cache: std::cell::RefCell<crate::render::StreamCache>,
    /// Configurable keybinds loaded from config at startup. Synced into `Tui.keybinds` for
    /// `poll_event` matching; modified by the keybind configurator overlay.
    pub keybinds: forge_config::KeybindsConfig,
    /// Last terminal width seen at render time, so scrollback-line builders (in `apply`, which has
    /// no `Rect`) can size width-aware truncation. `Cell` because it's written from `&self` render.
    last_width: std::cell::Cell<u16>,
    /// Whether the `/status` inspector overlay is open.
    pub status_open: bool,
    /// The tick the app started waiting for the user (permission or question), noted by the
    /// first frame that draws the wait so the border can pulse for its first few seconds.
    pub(crate) waiting_since: std::cell::Cell<Option<usize>>,
    /// Goal progress chip text (`2/5`), shown on the composer border while a `/goal` runs.
    pub goal_chip: Option<String>,
    /// Loop chip text (`5m`), shown on the composer border while a `/loop` runs.
    pub loop_chip: Option<String>,
    /// Fold each run of tool calls into one summary row (`[tui] fold_tool_groups`, full-screen).
    pub fold_tool_groups: bool,
    /// Which tool groups are open and which has keyboard focus.
    pub(crate) fold_ui: state::fold_ui::FoldUi,
    #[allow(dead_code)] // filled in by the transcript surface
    pub(crate) transcript_ui: state::transcript_ui::TranscriptUi,
    #[allow(dead_code)] // filled in by the interaction surface
    pub(crate) interaction_ui: state::interaction_ui::InteractionUi,
    pub(crate) panels_ui: state::panels_ui::PanelsUi,
}

/// A position in the wrapped transcript: `row` is the wrapped-row index in the cache, `col` the
/// 0-based column within that row.
type TextPos = (usize, u16);

/// Transcript area geometry captured at render time, used to map mouse cells → transcript text.
#[derive(Debug, Clone, Copy)]
struct TranscriptGeom {
    col0: u16,
    row0: u16,
    width: u16,
    height: u16,
    /// The scroll offset (top wrapped-row index) the last frame rendered with.
    scroll: usize,
}

/// Cached result of wrapping [`App::main_log`] to a width. Valid while `(width, rev)` are unchanged.
#[derive(Debug, Clone, Default)]
struct WrapCache {
    width: u16,
    rev: u64,
    rows: Vec<TextLine<'static>>,
    /// For each wrapped row, the `main_log` line it came from. A click lands on a wrapped row;
    /// tool cards own a RANGE of source lines, so a card can only be identified from a click
    /// through this map.
    origins: Vec<usize>,
    /// The fold state the rows were built with, and the rows the fold added (sorted by row) and
    /// the blocks a click toggles a group from. Empty while groups do not fold.
    fold_key: group_flow::FoldKey,
    fold_rows: Vec<group_flow::FoldRow>,
    group_hits: Vec<group_flow::GroupHit>,
}

/// Cached wrap of the entry the in-loop activity viewer has selected, keyed on
/// `(selected, wrap_width, revision)` — the same key the standalone viewer's own cache uses
/// (`transcript.rs`). `None` while the viewer is closed so a session that once looked at a
/// 5000-line main chat doesn't keep that copy alive afterwards.
#[derive(Debug, Clone, Default)]
struct ViewerWrapCache {
    key: Option<(usize, u16, u64)>,
    rows: Vec<TextLine<'static>>,
}

/// How many recent scrollback lines the remote snapshot keeps (a phone screen shows ~6–8).
const REMOTE_TRANSCRIPT_MAX: usize = 12;

impl TranscriptKind {
    /// The stable wire word (see `remote::SnapTranscriptRow::kind`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::JobNotice => "job_notice",
            Self::HarnessNotice => "harness_notice",
        }
    }
}

/// The provenance of one flushed line, recorded next to it as it is emitted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LineOrigin {
    kind: TranscriptKind,
    /// The tool name, on `Tool` lines.
    tool: Option<String>,
    /// `"ok"` / `"failed"` on a tool RESULT line; `None` on the call line that precedes it.
    meta: Option<String>,
}

impl LineOrigin {
    fn user() -> Self {
        Self {
            kind: TranscriptKind::User,
            ..Self::default()
        }
    }

    fn job(meta: Option<&str>) -> Self {
        Self {
            kind: TranscriptKind::JobNotice,
            tool: None,
            meta: meta.map(str::to_string),
        }
    }

    fn harness(subtype: &str) -> Self {
        Self {
            kind: TranscriptKind::HarnessNotice,
            tool: None,
            meta: Some(subtype.to_string()),
        }
    }

    /// A row that is for the terminal only (a turn's footer): the remote transcript skips it.
    fn local() -> Self {
        Self {
            kind: TranscriptKind::System,
            tool: None,
            meta: Some("local".to_string()),
        }
    }

    fn assistant() -> Self {
        Self {
            kind: TranscriptKind::Assistant,
            ..Self::default()
        }
    }

    fn tool_call(name: &str) -> Self {
        Self {
            kind: TranscriptKind::Tool,
            tool: Some(name.to_string()),
            meta: None,
        }
    }

    fn tool_result(name: &str, ok: bool) -> Self {
        Self {
            kind: TranscriptKind::Tool,
            tool: Some(name.to_string()),
            meta: Some(if ok { "ok" } else { "failed" }.to_string()),
        }
    }
}

/// One finalized scrollback line with its provenance, for the remote transcript ring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptRow {
    pub kind: TranscriptKind,
    pub text: String,
    pub tool: Option<String>,
    pub meta: Option<String>,
}

/// One subagent's live row in the TUI.
#[derive(Debug, Clone)]
pub(crate) struct SubRow {
    pub(crate) id: String,
    pub(crate) agent: String,
    pub(crate) task: String,
    /// The model this child routed to (shown in the activity panel). `None` until known.
    pub(crate) model: Option<String>,
    /// A workflow-script `phase()` label, if any (docs/rfcs/forge-workflow.md) — rows sharing a
    /// phase are grouped together under a header in the activity panel. `None` for a plain
    /// `spawn_agents` batch (unchanged, ungrouped rendering).
    pub(crate) phase: Option<String>,
    /// Trailing edge of the child's streamed activity (RFC subagent-orchestration Phase 3b).
    pub(crate) last: String,
    /// Recent progress snippets, newest last, for the expandable detail view. Bounded so a chatty
    /// child can't grow the buffer without limit.
    log: Vec<String>,
    pub(crate) done: bool,
    /// Only meaningful once `done` — whether the child's own call succeeded. `true` while
    /// running (no news isn't bad news); set from `SubagentResult.ok` on completion.
    pub(crate) ok: bool,
    pub(crate) cost: f64,
}

/// What an [`ActivityRow`] / [`TranscriptView`] represents in the unified activity list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    /// The main conversation (top-level transcript).
    MainChat,
    /// A spawned child agent.
    Subagent,
    /// An assay critic.
    AssayCritic,
}

/// Run state of one activity entry, kind-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityStatus {
    Running,
    Done,
    /// Finished, but the underlying subagent call itself failed — rendered distinctly (red ✗,
    /// not a green ✓) so a failed workflow phase doesn't look like it succeeded.
    Failed,
    Skipped,
}

/// A cheap, allocation-light row for the sticky activity panel: metadata only, no transcript lines.
/// Built every frame, so it must NOT clone the (potentially large) transcript — that's deferred to
/// [`App::activity_views`], which only runs when the full-screen viewer is open.
#[derive(Debug, Clone)]
pub struct ActivitySummary {
    pub kind: ActivityKind,
    pub title: String,
    pub subtitle: String,
    pub model: Option<String>,
    /// A workflow-script `phase()` label, if any (docs/rfcs/forge-workflow.md) — consecutive
    /// entries sharing a phase are grouped under one header in the activity panel.
    pub phase: Option<String>,
    pub status: ActivityStatus,
    pub cost: f64,
    pub line_count: usize,
}

/// One owned snapshot in the unified activity viewer: main chat, a subagent, or an assay critic.
/// Carries pre-styled transcript lines so the full-screen viewer renders them exactly like the
/// main chat (markdown + role coloring), wrapped to the terminal width.
#[derive(Debug, Clone)]
pub struct TranscriptView {
    pub kind: ActivityKind,
    /// Display title — "main chat", the agent name, or the critic lens.
    pub title: String,
    /// One-line subtitle: the task (subagent), the focus (critic), or empty (main chat).
    pub subtitle: String,
    /// The model this entry routed to, if known.
    pub model: Option<String>,
    pub status: ActivityStatus,
    pub cost: f64,
    /// Monotonic source-content revision used to memoize standalone viewer wrapping.
    pub revision: u64,
    /// The full transcript, pre-styled and ready to wrap+render in the viewer.
    pub lines: Vec<TextLine<'static>>,
    /// Plain-text count of source entries (for the panel's "N lines" hint).
    pub line_count: usize,
}

/// State of the in-loop activity transcript viewer (full-screen mode). Selection + scroll position;
/// `scroll == usize::MAX / 2` is the "tail" sentinel, clamped to the real max at render time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewerState {
    /// Index into the activity list (0 = main chat, then subagents, then critics).
    pub selected: usize,
    /// Wrapped-row offset from the top; the tail sentinel while `follow`.
    pub scroll: usize,
    /// Auto-scroll to the tail as the selected entry grows.
    pub follow: bool,
}

impl Default for ViewerState {
    fn default() -> Self {
        Self {
            selected: 0,
            scroll: usize::MAX / 2,
            follow: true,
        }
    }
}

/// A serializable snapshot of the live TUI view (activity panel + viewer + scroll), persisted to the
/// session at the end of each turn so a resume restores the exact on-screen state. The main
/// conversation transcript is NOT included — it's already rehydrated from the message history on
/// resume; this captures only the ephemeral view that history doesn't carry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ViewSnapshot {
    pub subagents: Vec<SubagentSnapshot>,
    pub assay_critics: Vec<forge_types::AssayCriticRow>,
    pub assay_verifying: Option<usize>,
    pub tasks: Vec<forge_types::TodoItem>,
    pub activity_focused: bool,
    pub activity_idx: usize,
    pub viewer: Option<ViewerState>,
    pub transcript_scroll: usize,
    pub transcript_follow: bool,
}

/// A pre-fix persisted snapshot has no `ok` field at all — default it to `true` (matching the
/// old, always-green rendering) rather than fail to deserialize or flip old rows to "failed".
fn default_ok() -> bool {
    true
}

/// Serializable form of a subagent row (the live `SubRow` isn't serde and is private).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubagentSnapshot {
    pub id: String,
    pub agent: String,
    pub task: String,
    pub model: Option<String>,
    pub phase: Option<String>,
    pub last: String,
    pub log: Vec<String>,
    pub done: bool,
    #[serde(default = "default_ok")]
    pub ok: bool,
    pub cost: f64,
}

/// A keystroke, decoupled from crossterm so input handling is testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Char(char),
    Backspace,
    Enter,
    Esc,
    /// Ctrl-C (the `interrupt` bind). Mid-turn it stops the response like Esc; idle with nothing
    /// modal open it quits. Kept distinct from `Esc` so a bare Esc can carry its own idle meaning
    /// (double-tap opens the rewind picker) without turning the quit key into a rewind key.
    Interrupt,
    /// Arrow up/down — navigate the command palette / pickers (ignored by the input line).
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    /// Ctrl+J — insert a newline into the input without submitting.
    InsertNewline,
    /// Delete key — delete the character forward of the cursor.
    DeleteForward,
    /// Ctrl+W — delete word backward (to the previous word boundary).
    DeleteWordBack,
    /// Ctrl+U — kill from cursor to start of the current line.
    KillLineBack,
    /// Ctrl+K — kill from cursor to end of the current line.
    KillLineForward,
    /// Ctrl+Left — move cursor one word left.
    WordLeft,
    /// Ctrl+Right — move cursor one word right.
    WordRight,
    /// TAB — complete the palette selection (ignored by the input line).
    Tab,
    /// SHIFT+TAB — cycle the operating temper (handled by the shell, not the input line).
    CycleTemper,
    /// CTRL+O — toggle the expanded detail view for the active subagents (shell-handled).
    ToggleSubagentDetail,
    /// CTRL+T — expand/collapse the most recent tool card (shell-handled). The keyboard path to
    /// what a click on the card does, for terminals without mouse reporting.
    ToggleToolCard,
    /// PageUp — scroll the full-screen transcript up (shell-handled; ignored by the input line).
    PageUp,
    /// PageDown — scroll the full-screen transcript down (shell-handled).
    PageDown,
    /// Ctrl+End — jump the full-screen transcript to the bottom and resume following (shell-handled).
    JumpBottom,
    /// Ctrl+Home — jump the full-screen transcript to the most recent user message above the
    /// current view; pressed again, to the one before it (shell-handled).
    JumpPrevUser,
    /// Alt+↑ / Alt+↓ — move the keyboard focus to the previous / next tool group in the
    /// full-screen transcript (shell-handled; see `App::tool_group_key`).
    PrevToolGroup,
    NextToolGroup,
    /// Alt+O — open every tool group, or fold them all again.
    ToggleToolGroups,
    /// Ctrl+R — toggle the effort-level slider popup above the input bar.
    ToggleEffortSlider,
    /// Mid-turn: abort + retry with next mesh model.
    SkipModel,
    /// Mid-turn or idle: escalate task to next tier (trivial→standard→complex).
    TierUp,
    /// Mid-turn or idle: de-escalate task to previous tier.
    TierDown,
    /// Toggle display of model reasoning/thinking blocks.
    ToggleReasoning,
    /// Open the global searchable command center.
    OpenCommandCenter,
    /// Open the keybind configurator overlay.
    OpenKeybindConfig,
    /// Open the model picker overlay.
    ModelPicker,
    /// Cycle effort level (low→medium→high→max).
    EffortCycle,
    /// Cycle operating temper/mode.
    TemperCycle,
    /// Copy last assistant response to clipboard.
    CopyLast,
    /// Show keybind help overlay (F1).
    ShowHelp,
    /// Save session checkpoint.
    SaveCheckpoint,
    /// Start a fresh session.
    NewSession,
    /// Undo last file write.
    UndoWrite,
    /// Compact/summarize the conversation.
    CompactSession,
    /// Hot-reload config without restarting.
    ReloadConfig,
    /// Ctrl+V (configurable) — open the `/voice` recording overlay, same as typing `/voice`.
    ToggleVoice,
}

/// The result of feeding a keystroke to the input line.
#[derive(Debug, PartialEq, Eq)]
pub enum InputOutcome {
    Editing,
    Submit(String),
    Quit,
}

fn prev_char_boundary(s: &str, pos: usize) -> usize {
    use unicode_segmentation::GraphemeCursor;
    let mut pos = pos.min(s.len());
    while !s.is_char_boundary(pos) {
        pos -= 1;
    }
    if pos == 0 {
        return 0;
    }
    GraphemeCursor::new(pos, s.len(), true)
        .prev_boundary(s, 0)
        .ok()
        .flatten()
        .unwrap_or(0)
}

fn next_char_boundary(s: &str, pos: usize) -> usize {
    use unicode_segmentation::GraphemeCursor;
    if pos >= s.len() {
        return s.len();
    }
    let mut pos = pos;
    while !s.is_char_boundary(pos) {
        pos -= 1;
    }
    GraphemeCursor::new(pos, s.len(), true)
        .next_boundary(s, 0)
        .ok()
        .flatten()
        .unwrap_or(s.len())
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Ctrl+Left / Ctrl+W: find the start of the previous word from `pos`.
fn prev_word_start(s: &str, mut pos: usize) -> usize {
    // Skip non-word chars backward
    while pos > 0 {
        let p = prev_char_boundary(s, pos);
        if is_word_char(s[p..pos].chars().next().unwrap_or(' ')) {
            break;
        }
        pos = p;
    }
    // Skip word chars backward
    while pos > 0 {
        let p = prev_char_boundary(s, pos);
        if !is_word_char(s[p..pos].chars().next().unwrap_or(' ')) {
            break;
        }
        pos = p;
    }
    pos
}

/// Ctrl+Right: find the end of the next word from `pos`.
fn next_word_end(s: &str, mut pos: usize) -> usize {
    // Skip non-word chars forward
    while pos < s.len() {
        let next = next_char_boundary(s, pos);
        if is_word_char(s[pos..next].chars().next().unwrap_or(' ')) {
            break;
        }
        pos = next;
    }
    // Skip word chars forward
    while pos < s.len() {
        let next = next_char_boundary(s, pos);
        if !is_word_char(s[pos..next].chars().next().unwrap_or(' ')) {
            break;
        }
        pos = next;
    }
    pos
}

fn sanitize_terminal_text(text: &str) -> String {
    text.chars()
        .filter(|&c| c == '\n' || c == '\t' || !c.is_control())
        .collect()
}

fn sanitize_paste(text: &str) -> String {
    // Collapse CRLF to a single LF FIRST: mapping '\r' and '\n' independently would turn a
    // Windows-origin "\r\n" into "\n\n", inserting a blank line between every pasted line. Then
    // map any lone CR to LF and drop other control chars.
    text.replace("\r\n", "\n")
        .chars()
        .filter_map(|c| match c {
            '\r' => Some('\n'),
            '\n' | '\t' => Some(c),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

/// Apply one keystroke to the input buffer (pure; no terminal I/O). `cursor` is the byte
/// offset of the text cursor within `input`; updated in place, always kept on a char boundary.
pub fn handle_key(input: &mut String, cursor: &mut usize, key: KeyKind) -> InputOutcome {
    *cursor = (*cursor).min(input.len());
    match key {
        KeyKind::Char(c) => {
            input.insert(*cursor, c);
            *cursor += c.len_utf8();
            InputOutcome::Editing
        }
        KeyKind::InsertNewline => {
            input.insert(*cursor, '\n');
            *cursor += 1;
            InputOutcome::Editing
        }
        KeyKind::Backspace => {
            if *cursor > 0 {
                let prev = prev_char_boundary(input, *cursor);
                input.remove(prev);
                *cursor = prev;
            }
            InputOutcome::Editing
        }
        KeyKind::Left => {
            if *cursor > 0 {
                *cursor = prev_char_boundary(input, *cursor);
            }
            InputOutcome::Editing
        }
        KeyKind::Right => {
            if *cursor < input.len() {
                *cursor = next_char_boundary(input, *cursor);
            }
            InputOutcome::Editing
        }
        KeyKind::Home => {
            let before = &input[..*cursor];
            *cursor = before.rfind('\n').map(|p| p + 1).unwrap_or(0);
            InputOutcome::Editing
        }
        KeyKind::End => {
            let after = &input[*cursor..];
            *cursor += after.find('\n').unwrap_or(after.len());
            InputOutcome::Editing
        }
        KeyKind::Enter => {
            if input.trim().is_empty() {
                InputOutcome::Editing
            } else {
                *cursor = 0;
                InputOutcome::Submit(std::mem::take(input))
            }
        }
        KeyKind::DeleteForward => {
            if *cursor < input.len() {
                let next = next_char_boundary(input, *cursor);
                input.drain(*cursor..next);
            }
            InputOutcome::Editing
        }
        KeyKind::DeleteWordBack => {
            let start = prev_word_start(input, *cursor);
            input.drain(start..*cursor);
            *cursor = start;
            InputOutcome::Editing
        }
        KeyKind::KillLineBack => {
            let line_start = input[..*cursor].rfind('\n').map(|p| p + 1).unwrap_or(0);
            input.drain(line_start..*cursor);
            *cursor = line_start;
            InputOutcome::Editing
        }
        KeyKind::KillLineForward => {
            let line_end = input[*cursor..]
                .find('\n')
                .map(|p| *cursor + p)
                .unwrap_or(input.len());
            input.drain(*cursor..line_end);
            InputOutcome::Editing
        }
        KeyKind::WordLeft => {
            *cursor = prev_word_start(input, *cursor);
            InputOutcome::Editing
        }
        KeyKind::WordRight => {
            *cursor = next_word_end(input, *cursor);
            InputOutcome::Editing
        }
        KeyKind::Esc | KeyKind::Interrupt => InputOutcome::Quit,
        KeyKind::Up
        | KeyKind::Down
        | KeyKind::Tab
        | KeyKind::CycleTemper
        | KeyKind::ToggleSubagentDetail
        | KeyKind::ToggleToolCard
        | KeyKind::PrevToolGroup
        | KeyKind::NextToolGroup
        | KeyKind::ToggleToolGroups
        | KeyKind::ToggleEffortSlider
        | KeyKind::PageUp
        | KeyKind::PageDown
        | KeyKind::JumpBottom
        | KeyKind::JumpPrevUser
        | KeyKind::SkipModel
        | KeyKind::TierUp
        | KeyKind::TierDown
        | KeyKind::ToggleReasoning
        | KeyKind::OpenCommandCenter
        | KeyKind::OpenKeybindConfig
        | KeyKind::ModelPicker
        | KeyKind::EffortCycle
        | KeyKind::TemperCycle
        | KeyKind::CopyLast
        | KeyKind::ShowHelp
        | KeyKind::SaveCheckpoint
        | KeyKind::NewSession
        | KeyKind::UndoWrite
        | KeyKind::CompactSession
        | KeyKind::ReloadConfig
        | KeyKind::ToggleVoice => InputOutcome::Editing,
    }
}

impl App {
    fn set_turn_activity(&mut self, phase: TurnPhase, detail: impl Into<String>) {
        self.turn_activity.phase = phase;
        self.turn_activity.detail = detail.into();
        self.turn_activity.last_event_tick = self.tick;
    }

    fn touch_turn_activity(&mut self, phase: TurnPhase) {
        self.turn_activity.phase = phase;
        self.turn_activity.last_event_tick = self.tick;
    }

    /// Seconds since the last presenter event that proves progress. The main render loop advances
    /// `tick` every ~60ms while busy, so this remains live even when a provider emits nothing.
    fn turn_quiet_secs(&self) -> u64 {
        self.tick
            .saturating_sub(self.turn_activity.last_event_tick)
            .saturating_mul(60)
            .div_euclid(1_000) as u64
    }

    /// Fold one presenter event into the view state, queueing any finalized scrollback.
    pub fn apply(&mut self, event: PresenterEvent) {
        match event {
            PresenterEvent::SessionStarted { id } => self.session_id = id,
            PresenterEvent::Routing {
                tier,
                model,
                rationale,
                effort,
            } => {
                // Mesh's chosen rung, kept separately from the session pin: the statusline must
                // report the rung actually being sent, and since auto-effort the pin is only the
                // ceiling on that choice.
                self.routed_effort = effort;
                self.routing = Some(RoutingView {
                    tier,
                    model: crate::display_model(&model),
                    rationale,
                });
                self.set_turn_activity(
                    TurnPhase::ContactingModel,
                    "request prepared; waiting for the first provider event",
                );
            }
            PresenterEvent::ProviderRequest { model, step } => {
                self.turn_activity.model = Some(model);
                self.turn_activity.composing_tool = None;
                self.turn_activity.thinking_tokens = 0;
                self.set_turn_activity(
                    TurnPhase::ContactingModel,
                    format!(
                        "provider request for model-loop step {} sent; waiting for the first event",
                        step + 1
                    ),
                );
            }
            PresenterEvent::ProviderProgress => {
                self.model_search = None;
                self.turn_activity.provider_events =
                    self.turn_activity.provider_events.saturating_add(1);
                self.set_turn_activity(
                    TurnPhase::ReceivingProvider,
                    format!(
                        "provider stream active; {} buffered event(s) received",
                        self.turn_activity.provider_events
                    ),
                );
            }
            PresenterEvent::ProviderToolInput { name, chars } => {
                self.model_search = None;
                self.turn_activity.provider_events =
                    self.turn_activity.provider_events.saturating_add(1);
                let name = tool_display_name(&name).to_string();
                self.set_turn_activity(
                    TurnPhase::ReceivingProvider,
                    format!(
                        "writing {name} call; {} chars of arguments",
                        human(chars as u64)
                    ),
                );
                self.turn_activity.composing_tool = Some((name, chars));
            }
            PresenterEvent::ProviderThinking { tokens } => {
                self.model_search = None;
                self.turn_activity.provider_events =
                    self.turn_activity.provider_events.saturating_add(1);
                self.turn_activity.thinking_tokens = tokens;
                self.set_turn_activity(
                    TurnPhase::ReceivingProvider,
                    format!("thinking; ~{} tokens", human(tokens)),
                );
            }
            PresenterEvent::AuxiliaryRequest { model, purpose } => {
                self.turn_activity.model = Some(model);
                self.turn_activity.auxiliary_chars = 0;
                self.set_turn_activity(TurnPhase::Auxiliary, purpose);
            }
            PresenterEvent::AuxiliaryProgress { chars } => {
                self.turn_activity.auxiliary_chars =
                    self.turn_activity.auxiliary_chars.saturating_add(chars);
                self.set_turn_activity(
                    TurnPhase::Auxiliary,
                    format!(
                        "internal model streamed {} chars",
                        human(self.turn_activity.auxiliary_chars as u64)
                    ),
                );
            }
            // Failover in progress: keep a single animated indicator instead of one warning per
            // hop. The model that just failed is recorded only for the (dim) hint; the status bar's
            // own routing line shows the model now being tried.
            PresenterEvent::ModelSearch { model, retrying } => {
                self.model_search = Some((model, retrying));
                self.set_turn_activity(
                    TurnPhase::Recovering,
                    if retrying {
                        "backing off before retrying the same model"
                    } else {
                        "selecting and connecting to a fallback model"
                    },
                );
            }
            // A complete (non-streamed) assistant message: render markdown into scrollback.
            PresenterEvent::AssistantText(text) => {
                let text = sanitize_terminal_text(&text);
                self.model_search = None;
                self.turn_activity.response_chars += text.chars().count();
                self.set_turn_activity(TurnPhase::Finalizing, "complete response received");
                self.flush_reasoning();
                self.ensure_assistant_label();
                self.push_answer_segment(&text, crate::render::markdown_to_lines(&text));
            }
            PresenterEvent::Reasoning(delta) => {
                self.model_search = None;
                let delta = sanitize_terminal_text(&delta);
                self.transcript_ui
                    .reasoning_since
                    .get_or_insert_with(std::time::Instant::now);
                self.turn_activity.reasoning_chars += delta.chars().count();
                self.throughput
                    .on_delta(delta.chars().count(), std::time::Instant::now());
                self.touch_turn_activity(TurnPhase::Reasoning);
                self.reasoning.push_str(&delta)
            }
            PresenterEvent::AssistantDelta(delta) => {
                let delta = sanitize_terminal_text(&delta);
                self.model_search = None;
                self.turn_activity.response_chars += delta.chars().count();
                self.throughput
                    .on_delta(delta.chars().count(), std::time::Instant::now());
                self.touch_turn_activity(TurnPhase::Responding);
                if !self.streaming_active {
                    self.flush_reasoning();
                    self.ensure_assistant_label();
                    self.streaming_active = true;
                }
                // Accumulate the whole reply; it's rendered as markdown on AssistantDone so
                // multi-line blocks (lists, code fences) stay whole. The growing tail shows
                // live (plain) in the preview. (Per-block streaming finalize is a follow-up.)
                self.streaming.push_str(&delta);
                self.streaming_rev = self.streaming_rev.wrapping_add(1);
            }
            PresenterEvent::AssistantDone => {
                self.set_turn_activity(TurnPhase::Finalizing, "response stream completed");
                if self.streaming_active {
                    self.finish_streaming_text();
                } else {
                    // Reasoning arrived but no answer text streamed — still show the thinking.
                    self.flush_reasoning();
                }
            }
            PresenterEvent::Steered(text) => {
                // The steered reply is a new message: close the text streamed so far so the two
                // are not glued into one paragraph.
                if self.streaming_active {
                    self.finish_streaming_text();
                }
                self.echo_steered(&text)
            }
            PresenterEvent::Warning(msg) => {
                if !self.apply_btw_failure(&msg) {
                    let msg = sanitize_terminal_text(&msg);
                    let (kind, body) = lines::notice::classify(&msg, NoticeKind::Warn);
                    self.push_notice(kind, &body);
                }
            }
            PresenterEvent::Error(msg) => {
                let msg = sanitize_terminal_text(&msg);
                let (_, body) = lines::notice::classify(&msg, NoticeKind::Err);
                self.push_error(&body);
            }
            PresenterEvent::ToolStart { name, args } => {
                self.model_search = None;
                if self.streaming_active {
                    self.finish_streaming_text();
                }
                self.ensure_assistant_label();
                self.turn_activity.composing_tool = None;
                self.turn_activity.tool_calls += 1;
                self.transcript_ui.tools_used += 1;
                let target = tool_cards::headline(&name, &args);
                self.set_turn_activity(
                    TurnPhase::RunningTool,
                    format!(
                        "executing {}",
                        [tool_cards::verb(&name), target]
                            .iter()
                            .filter(|p| !p.is_empty())
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(" ")
                    ),
                );
                let origin = LineOrigin::tool_call(&name);
                self.interaction_ui.note_tool_call(&name, &args);
                self.open_tool_card(origin, name, args);
            }
            PresenterEvent::ToolResult {
                name,
                ok,
                summary,
                detail,
            } => {
                self.set_turn_activity(
                    TurnPhase::ProcessingToolResult,
                    format!(
                        "{name} {}; sending the result back to the model",
                        if ok { "finished" } else { "failed" }
                    ),
                );
                // Resolve a pending write preview: the preview's tool ran to completion (the
                // write path is synchronous between its Diff and its ToolResult), so ok means
                // the change LANDED — promote it to the turn's diff card. A failed or denied
                // write never touched the file: drop the preview.
                if let Some(d) = self.pending_diff.take() {
                    if ok {
                        self.push_turn_diff(d);
                    }
                }
                if !self.close_tool_card(&name, ok, &summary, detail.clone()) {
                    self.ensure_assistant_label();
                    self.print_orphan_result(&name, ok, &summary, detail);
                }
            }
            PresenterEvent::ToolOutput { name, output } => self.attach_tool_output(&name, output),
            PresenterEvent::ContextInjected {
                symbols,
                files,
                tokens,
            } => {
                self.set_turn_activity(
                    TurnPhase::Preparing,
                    format!("loaded {symbols} symbols from {files} files ({tokens} tokens)"),
                );
                let in_turn = self.transcript_ui.turn_open;
                let rows = lattice_line(symbols, files, tokens, in_turn);
                self.flush.extend(rows);
            }
            PresenterEvent::ShellDiagnosis {
                command,
                diagnosis,
                fix,
            } => {
                self.set_turn_activity(
                    TurnPhase::ProcessingToolResult,
                    "shell diagnosis ready; sending the result back to the model",
                );
                let in_turn = self.transcript_ui.turn_open;
                self.flush.extend(shell_diagnosis_lines(
                    &command,
                    &diagnosis,
                    fix.as_deref(),
                    self.last_width.get(),
                    in_turn,
                ));
            }
            PresenterEvent::BtwAnswer {
                question,
                answer,
                model,
                cost_usd,
            } => self.apply_btw_answer(question, answer, model, cost_usd),
            PresenterEvent::Cost {
                session_total_usd,
                session_in,
                session_cached_in,
                session_out,
                context_tokens,
                context_limit,
            } => {
                self.cost_usd = session_total_usd;
                self.session_in = session_in;
                self.session_cached_in = session_cached_in;
                self.session_out = session_out;
                self.context_tokens = context_tokens;
                self.context_limit = context_limit;
                // Per-turn token deltas from the baseline snapshotted in `on_turn_start`.
                self.turn_in = session_in.saturating_sub(self.turn_base_in);
                self.turn_cached_in = session_cached_in
                    .map(|now| now.saturating_sub(self.turn_base_cached_in.unwrap_or(0)));
                self.turn_out = session_out.saturating_sub(self.turn_base_out);
                self.throughput
                    .on_usage(self.turn_out, std::time::Instant::now());
            }
            PresenterEvent::SubagentStart {
                id,
                agent,
                task,
                model,
                phase,
            } => {
                self.set_turn_activity(TurnPhase::Coordinating, "subagent work is running");
                // A live workflow owns its agents: rows fold into the dedicated workflow view,
                // never the sticky activity panel (its scrollback record is written by the
                // WorkflowPhase/WorkflowFinished handlers instead of the batch header/footer).
                if self.workflow.active {
                    self.workflow.on_agent_start(id, agent, task, model, phase);
                    return;
                }
                // A new batch begins once every existing row has already finished (or there are
                // none yet) — open a fresh scrollback header for it. Rows themselves are NOT
                // cleared here: they're retained across batches within the same turn (so e.g. a
                // multi-phase workflow's earlier phases stay visible/viewable alongside a later
                // one) and are only reset by `on_turn_start` at the start of a genuinely new user
                // turn. (Previously this branch also cleared `self.subagents`, which wiped a
                // finished batch the instant the NEXT batch's first child started — invisible for
                // a single spawn_agents call, but silently dropped every earlier batch/phase the
                // moment a second one began within the same turn.)
                let starting_new_batch =
                    self.subagents.is_empty() || self.subagents.iter().all(|r| r.done);
                if starting_new_batch {
                    self.subagent_batch_start = self.subagents.len();
                    self.flush.push(subagent_header_line());
                }
                self.subagents.push(SubRow {
                    id,
                    agent,
                    task,
                    model,
                    phase,
                    last: String::new(),
                    log: Vec::new(),
                    done: false,
                    ok: true,
                    cost: 0.0,
                });
                // Bound total retention: drop the oldest already-finished rows first (never an
                // in-progress one) so a turn with many batches/phases can't grow this unboundedly.
                const MAX_SUBAGENT_ROWS: usize = 500;
                if self.subagents.len() > MAX_SUBAGENT_ROWS {
                    let mut excess = self.subagents.len() - MAX_SUBAGENT_ROWS;
                    let mut removed_before_batch_start = 0usize;
                    let mut idx = 0usize;
                    self.subagents.retain(|r| {
                        let keep = !(excess > 0 && r.done);
                        if !keep {
                            excess -= 1;
                            if idx < self.subagent_batch_start {
                                removed_before_batch_start += 1;
                            }
                        }
                        idx += 1;
                        keep
                    });
                    self.subagent_batch_start -= removed_before_batch_start;
                }
            }
            PresenterEvent::SubagentProgress { id, snippet } => {
                self.set_turn_activity(TurnPhase::Coordinating, "subagent reported progress");
                if self.workflow.active {
                    self.workflow.on_progress(&id, &snippet);
                    return;
                }
                if let Some(row) = self.subagents.iter_mut().find(|r| r.id == id && !r.done) {
                    // Keep only the trailing edge of the child's activity for its row.
                    row.last.push_str(snippet.replace('\n', " ").as_str());
                    let n = row.last.chars().count();
                    if n > 80 {
                        row.last = row.last.chars().skip(n - 80).collect();
                    }
                    // Assemble the streamed token-fragments into coherent transcript lines: append
                    // to the open last line, breaking only on real newlines. (Pushing each fragment
                    // as its own line fragmented identifiers like `count_text` across many rows.)
                    if row.log.is_empty() {
                        row.log.push(String::new());
                    }
                    for ch in snippet.chars() {
                        if ch == '\n' {
                            row.log.push(String::new());
                        } else {
                            row.log.last_mut().unwrap().push(ch);
                        }
                    }
                    // Cap the transcript by a high safety bound so a pathological child can't
                    // exhaust memory; keep the newest lines.
                    const MAX_LOG: usize = 10_000;
                    if row.log.len() > MAX_LOG {
                        let drop = row.log.len() - MAX_LOG;
                        row.log.drain(0..drop);
                    }
                }
            }
            PresenterEvent::SubagentResult {
                id,
                agent,
                ok,
                summary,
                cost_usd,
            } => {
                self.set_turn_activity(TurnPhase::Coordinating, "subagent result received");
                if self.workflow.active {
                    // The row lives in the workflow view; a branch line still goes to scrollback
                    // so the run leaves a readable history after the view closes.
                    self.flush.push(subagent_branch_line(
                        &agent,
                        ok,
                        cost_usd,
                        &summary,
                        self.last_width.get(),
                    ));
                    self.workflow.on_result(&id, ok, &summary, cost_usd);
                    return;
                }
                self.flush.push(subagent_branch_line(
                    &agent,
                    ok,
                    cost_usd,
                    &summary,
                    self.last_width.get(),
                ));
                if let Some(row) = self.subagents.iter_mut().find(|r| r.id == id && !r.done) {
                    row.done = true;
                    row.ok = ok;
                    row.cost = cost_usd;
                    // Drop a trailing empty/partial line, then record the outcome at the tail of the
                    // transcript so the browser shows it.
                    if row.log.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
                        row.log.pop();
                    }
                    row.log.push(String::new());
                    row.log.push(format!(
                        "── result ({}) ──",
                        if ok { "ok" } else { "failed" }
                    ));
                    for piece in summary.split('\n') {
                        row.log.push(piece.to_string());
                    }
                }
                // When every child in the CURRENT batch has reported, close the scrollback box.
                // Scoped to `subagent_batch_start..` — earlier phases' rows are retained (see the
                // `SubagentStart` handler) but must not be re-counted into this batch's total.
                // The rows are KEPT (not cleared) so their full transcripts remain viewable via
                // Ctrl+O until the next batch starts; the live panel collapses on its own (no rows
                // are running).
                let current_batch = &self.subagents[self.subagent_batch_start..];
                if current_batch.iter().all(|r| r.done) {
                    let n = current_batch.len();
                    let total: f64 = current_batch.iter().map(|r| r.cost).sum();
                    self.flush.push(subagent_footer_line(n, total));
                }
            }
            PresenterEvent::Diff(diff) => {
                // Stash the capped projection for the remote diff card (the diff arrives BEFORE
                // the permission gate, so while `prompt` is armed this is exactly what a remote
                // Allow would apply); the following ToolResult promotes or drops it.
                self.pending_diff = Some(crate::render::diff_file_snapshot(
                    &diff,
                    crate::render::REMOTE_DIFF_MAX_HUNK_LINES,
                ));
                // The change belongs inside the card of the call that makes it. With no such
                // call waiting, it stands alone as rows on the rail.
                if !self.attach_diff_to_card(diff.clone()) {
                    self.ensure_assistant_label();
                    let rows = crate::render::diff_to_lines(&diff);
                    self.push_turn_lines(rows);
                }
            }
            PresenterEvent::AssayProgress(msg) => {
                self.set_turn_activity(TurnPhase::Coordinating, "analysis critics are running");
                self.push_notice(NoticeKind::Info, &msg);
            }
            PresenterEvent::AssayCriticRow(row) => {
                self.set_turn_activity(TurnPhase::Coordinating, "analysis critic updated");
                // Update the live panel: insert on Queued, merge status+model+cost+output on Done/Skipped.
                if let Some(existing) = self.assay_critics.iter_mut().find(|r| r.lens == row.lens) {
                    existing.status = row.status;
                    if row.model.is_some() {
                        existing.model = row.model;
                        existing.cost_usd = row.cost_usd;
                    }
                    if !row.output.is_empty() {
                        existing.output = row.output;
                    }
                } else {
                    self.assay_critics.push(row);
                }
            }
            PresenterEvent::AssayVerifying { candidates } => {
                self.set_turn_activity(
                    TurnPhase::Coordinating,
                    format!("verifying {candidates} analysis candidates"),
                );
                self.assay_verifying = Some(candidates);
            }
            PresenterEvent::AssayReport(report) => {
                self.assay_critics.clear();
                self.assay_verifying = None;
                self.clamp_activity_selection();
                self.flush
                    .extend(crate::render::assay_report_lines(&report));
                self.flush.push(TextLine::default());
            }
            PresenterEvent::Tasks(tasks) => {
                // The task list lives ONLY in the sticky panel above the input — it does not scroll
                // with the chat and is not part of history, so it is NOT flushed to scrollback.
                // An empty list collapses the panel.
                self.tasks = tasks;
            }
            PresenterEvent::McpStatus(servers) => {
                self.mcp_count = servers.len();
                self.flush.extend(crate::render::mcp_status_lines(&servers));
                self.flush.push(TextLine::default());
            }
            PresenterEvent::Recap { text } => {
                if self.recap_is_worth_showing(&text) {
                    let row = self.recap_row(&text);
                    self.flush.push(row);
                }
            }
            PresenterEvent::Progress(text) => self.push_notice(NoticeKind::Info, &text),
            PresenterEvent::SuggestionReady { text } => {
                // Silent — no scrollback line. It only ever shows as ghost text in the (by now
                // empty, idle) input box; see `render_input`.
                self.suggested_prompt = Some(text);
            }
            PresenterEvent::Done { stop_reason, .. } => {
                self.model_search = None;
                self.set_turn_activity(TurnPhase::Finalizing, "turn completed");
                self.throughput.on_turn_end(std::time::Instant::now());
                if self.streaming_active {
                    self.finish_streaming_text();
                }
                self.done = true;
                self.last_stop_reason = Some(stop_reason);
                self.turns_finished += 1;
                self.settle_running_cards();
                self.close_turn();
            }
            PresenterEvent::QuotaUpdate {
                provider,
                window,
                fraction,
            } => {
                let pct = Some(fraction * 100.0);
                match (provider.as_str(), window.as_str()) {
                    ("claude-cli", "five_hour") => self.usage_overlay.claude_5h_pct = pct,
                    ("claude-cli", "weekly") => self.usage_overlay.claude_weekly_pct = pct,
                    ("codex-cli", "five_hour") => self.usage_overlay.codex_5h_pct = pct,
                    ("codex-cli", "weekly") => self.usage_overlay.codex_weekly_pct = pct,
                    ("opencode_go", "five_hour") => self.usage_overlay.opencode_go_5h_pct = pct,
                    ("opencode_go", "weekly") => self.usage_overlay.opencode_go_weekly_pct = pct,
                    ("opencode_go", "monthly") => self.usage_overlay.opencode_go_monthly_pct = pct,
                    _ => {}
                }
            }
            PresenterEvent::QuotaPace {
                provider,
                window,
                rate_per_hour,
                projected_fraction_at_reset,
                exhaustion_warning,
            } => {
                let candidate = QuotaPaceInfo {
                    provider,
                    window,
                    rate_per_hour,
                    projected_fraction_at_reset,
                    exhaustion_warning,
                };
                // "Closest to exhaustion" = highest projected fraction at reset (unprojected
                // windows sort last); a warning always outranks a non-warning candidate.
                let replace = match &self.quota_pace {
                    None => true,
                    Some(current)
                        if current.provider == candidate.provider
                            && current.window == candidate.window =>
                    {
                        true // refresh this same window's reading
                    }
                    Some(current) => {
                        candidate.exhaustion_warning && !current.exhaustion_warning
                            || candidate.exhaustion_warning == current.exhaustion_warning
                                && candidate.projected_fraction_at_reset.unwrap_or(0.0)
                                    > current.projected_fraction_at_reset.unwrap_or(0.0)
                    }
                };
                if replace {
                    self.quota_pace = Some(candidate);
                }
            }
            PresenterEvent::CustomWidgetOutput { id, text } => {
                self.custom_widget_cache.insert(id, text);
            }
            PresenterEvent::CompactionStarted { auto } => {
                self.set_turn_activity(
                    TurnPhase::Compacting,
                    if auto {
                        "summarizing older context automatically"
                    } else {
                        "summarizing older context"
                    },
                );
                self.compaction = Some(CompactionState {
                    start_tick: self.tick,
                    auto,
                });
            }
            PresenterEvent::CompactionFinished { .. } => {
                // The band clears; the core also emits a "compacted N → M" warning into scrollback.
                self.compaction = None;
                self.set_turn_activity(TurnPhase::Preparing, "context compaction finished");
            }
            PresenterEvent::PlanProposed(plan) => {
                self.flush.extend(plan_card_lines(&plan));
                // Retained as live state for the remote plan card (the scrollback card above is
                // unreachable from a phone); the approve/revise question follows at turn end.
                self.plan = Some(plan);
            }
            PresenterEvent::Temper(label) => self.temper = label,
            PresenterEvent::Effort(effort) => self.effort = effort,
            PresenterEvent::WorkflowStarted { name } => {
                self.set_turn_activity(TurnPhase::Coordinating, "workflow started");
                self.flush.push(workflow_started_line(name.as_deref()));
                self.workflow.begin(name);
            }
            PresenterEvent::WorkflowPhase { title } => {
                self.set_turn_activity(TurnPhase::Coordinating, format!("workflow phase: {title}"));
                self.flush.push(workflow_phase_line(&title));
                self.workflow.on_phase(title);
            }
            PresenterEvent::WorkflowLog(msg) => {
                self.set_turn_activity(TurnPhase::Coordinating, "workflow reported progress");
                // Scrollback gets a one-line note (a script can `log()` a whole report — the full
                // text lives in the view's narration feed, not the transcript).
                self.flush.push(workflow_log_line(
                    &msg,
                    width_cap(self.last_width.get(), 8, 72),
                ));
                self.workflow.on_log(msg);
            }
            PresenterEvent::WorkflowFinished { ok, summary } => {
                self.set_turn_activity(TurnPhase::Finalizing, "workflow finished");
                self.flush.push(workflow_finished_line(
                    ok,
                    &summary,
                    width_cap(self.last_width.get(), 8, 72),
                ));
                self.workflow.finish(ok, summary);
            }
        }
    }

    /// The live task list (`update_tasks`), for the sticky tasks panel. Empty → panel hidden.
    pub fn tasks(&self) -> &[forge_types::TodoItem] {
        &self.tasks
    }

    /// Set the git branch label (call once at startup).
    pub fn set_git_branch(&mut self, branch: Option<String>) {
        self.git_branch = branch;
    }

    /// Number of subagents currently running (between SubagentStart and SubagentResult). When `> 0`
    /// the sticky subagents panel is shown.
    pub fn running_subagents(&self) -> usize {
        self.subagents.iter().filter(|r| !r.done).count()
    }

    /// Rows the sticky tasks panel wants in the live region (0 = hidden). Header + up to
    /// [`TASKS_PANEL_MAX`] items + an overflow line. This is a DEDICATED region (not the stream
    /// preview) so the list stays visible even while the model is streaming.
    pub fn tasks_panel_height(&self) -> u16 {
        if self.tasks.is_empty() {
            return 0;
        }
        let shown = self.tasks.len().min(TASKS_PANEL_MAX);
        let overflow = u16::from(self.tasks.len() > TASKS_PANEL_MAX);
        1 + shown as u16 + overflow
    }

    /// Number of assay critics in the live panel (> 0 while an assay run is in flight).
    pub fn running_assay_critics(&self) -> usize {
        self.assay_critics.len()
    }

    /// True when there is subagent or assay activity to show in the sticky activity panel. The
    /// panel (and its "main chat" entry) only appears while there's something to drill into.
    pub fn has_activity(&self) -> bool {
        !self.subagents.is_empty() || !self.assay_critics.is_empty()
    }

    /// Number of rows in the activity list: main chat + each subagent + each assay critic.
    pub fn activity_len(&self) -> usize {
        if !self.has_activity() {
            return 0;
        }
        1 + self.subagents.len() + self.assay_critics.len()
    }

    /// Rows the sticky activity panel wants in the live region (0 = hidden). Header + up to
    /// [`ACTIVITY_PANEL_MAX`] entry rows + an overflow line + one line per workflow-script
    /// `phase()` transition among those rows (see `needs_phase_header`) — a plain `spawn_agents`
    /// batch has no phases, so this term is always 0 there, matching the old reservation exactly.
    pub fn activity_panel_height(&self) -> u16 {
        let n = self.activity_len();
        if n == 0 {
            return 0;
        }
        let shown = n.min(ACTIVITY_PANEL_MAX);
        let overflow = u16::from(n > ACTIVITY_PANEL_MAX);
        let views = self.activity_summaries();
        let mut last_phase: Option<&str> = None;
        let mut headers = 0u16;
        for v in views.iter().take(shown) {
            if needs_phase_header(last_phase, v) {
                headers += 1;
            }
            last_phase = v.phase.as_deref();
        }
        1 + shown as u16 + overflow + headers
    }

    /// Keep `activity_idx` within range as the list grows/shrinks; drop focus when empty.
    fn clamp_activity_selection(&mut self) {
        let n = self.activity_len();
        if n == 0 {
            self.activity_focused = false;
            self.activity_idx = 0;
        } else if self.activity_idx >= n {
            self.activity_idx = n - 1;
        }
    }

    /// Pure Tab-accept helper for the next-prompt ghost text (see `render_input`): when `input`
    /// is empty and a suggestion is pending, takes it (clearing `suggested_prompt`) for the
    /// caller to move into `input`. A no-op (`None`, `suggested_prompt` left untouched) when
    /// `input` already has text — a stray Tab mid-sentence must never clobber what's being typed.
    pub fn take_suggestion_for_tab(&mut self) -> Option<String> {
        if self.input.is_empty() {
            self.suggested_prompt.take()
        } else {
            None
        }
    }

    /// Called at the start of each new user turn. Collapses the "done" subagent batch that the
    /// panel was holding so it doesn't bleed into the new turn's live region.
    pub fn on_turn_start(&mut self) {
        // `tick` is the current turn's ~60ms clock. Reset it here, before recording the first
        // activity timestamp, so a long previous turn cannot suppress the silence warning on a
        // new turn until its smaller fresh tick catches up.
        self.tick = 0;
        if !self.subagents.is_empty() && self.subagents.iter().all(|r| r.done) {
            self.subagents.clear();
            self.subagent_batch_start = 0;
        }
        // A workflow run is per-turn, so ANY carry-over here is stale — including `active`
        // (an interrupted run whose WorkflowFinished never arrived). The old `if !active`
        // guard kept exactly that stale state alive forever: frozen status band, and every
        // later spawn_agents batch misrouted into the dead run's rows.
        self.workflow = Default::default();
        self.activity_focused = false;
        self.activity_idx = 0;
        // Reset the per-turn timer + token counters and snapshot the session-token baseline so this
        // turn's in/out are measured as a delta from here.
        self.turn_elapsed_secs = 0;
        self.turn_in = 0;
        self.turn_cached_in = None;
        self.turn_out = 0;
        self.throughput.on_turn_start(std::time::Instant::now());
        self.turn_base_in = self.session_in;
        self.turn_base_cached_in = self.session_cached_in;
        self.turn_base_out = self.session_out;
        self.turn_ran = true;
        self.turn_activity = TurnActivity {
            phase: TurnPhase::Preparing,
            detail: "building context and selecting a provider route".to_string(),
            last_event_tick: self.tick,
            ..Default::default()
        };
        // A suggestion predicted from the PREVIOUS turn's response is stale the moment a new
        // turn starts — never let it linger as ghost text for an unrelated prompt.
        self.suggested_prompt = None;
    }

    /// Cheap per-frame metadata for the sticky activity panel (no transcript cloning). Order:
    /// main chat, then subagents, then assay critics. Empty when there's no activity.
    pub fn activity_summaries(&self) -> Vec<ActivitySummary> {
        if !self.has_activity() {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(self.activity_len());
        out.push(ActivitySummary {
            kind: ActivityKind::MainChat,
            title: "main chat".to_string(),
            subtitle: String::new(),
            model: self.routing.as_ref().map(|r| r.model.clone()),
            phase: None,
            status: if self.busy {
                ActivityStatus::Running
            } else {
                ActivityStatus::Done
            },
            cost: self.cost_usd,
            line_count: self.main_log.len(),
        });
        for r in &self.subagents {
            out.push(ActivitySummary {
                kind: ActivityKind::Subagent,
                title: r.agent.clone(),
                subtitle: r.task.clone(),
                model: r.model.clone(),
                phase: r.phase.clone(),
                status: if !r.done {
                    ActivityStatus::Running
                } else if r.ok {
                    ActivityStatus::Done
                } else {
                    ActivityStatus::Failed
                },
                cost: r.cost,
                line_count: r.log.len(),
            });
        }
        for c in &self.assay_critics {
            use forge_types::AssayCriticStatus;
            let (status, subtitle) = match &c.status {
                AssayCriticStatus::Queued => (ActivityStatus::Running, c.focus.clone()),
                AssayCriticStatus::Done { candidates } => {
                    (ActivityStatus::Done, format!("{candidates} found"))
                }
                AssayCriticStatus::Skipped { reason } => {
                    (ActivityStatus::Skipped, format!("skipped ({reason})"))
                }
            };
            out.push(ActivitySummary {
                kind: ActivityKind::AssayCritic,
                title: c.lens.clone(),
                subtitle,
                model: c.model.clone(),
                phase: None,
                status,
                cost: c.cost_usd,
                line_count: c.output.lines().count(),
            });
        }
        out
    }

    /// Build the unified activity views (main chat + subagents + assay critics), in panel order, so
    /// the full-screen viewer can render any of them. Heavier (clones transcripts + renders
    /// markdown) — call ONLY when the full-screen viewer is open, never per render frame.
    /// Empty when there's no activity (see [`has_activity`]).
    pub fn activity_views(&self) -> Vec<TranscriptView> {
        let mut views = self.activity_view_headers();
        for (idx, view) in views.iter_mut().enumerate() {
            view.lines = self.activity_view_lines(idx);
        }
        views
    }

    /// The activity views WITHOUT their transcripts: everything the viewer's chrome reads (title,
    /// subtitle, status, cost, model, revision) with `lines` left empty. Cheap enough to rebuild
    /// every frame — no transcript clone, no markdown render — so the header stays live while only
    /// the one entry actually on screen pays for [`activity_view_lines`].
    /// Empty when there's no activity (see [`has_activity`]).
    fn activity_view_headers(&self) -> Vec<TranscriptView> {
        if !self.has_activity() {
            return Vec::new();
        }
        let mut views = Vec::with_capacity(self.activity_len());

        // 0: main chat — the real, already-styled transcript lines plus anything still pending in
        // the flush outbox (so the view updates live even while the full-screen viewer is open).
        views.push(TranscriptView {
            kind: ActivityKind::MainChat,
            title: "main chat".to_string(),
            subtitle: String::new(),
            model: self.routing.as_ref().map(|r| r.model.clone()),
            status: if self.busy {
                ActivityStatus::Running
            } else {
                ActivityStatus::Done
            },
            cost: self.cost_usd,
            // Both halves of the content, packed: `main_log_rev` alone would leave a wrap cache
            // stale for the queued flush lines until the next drain folded them in, and a sum of
            // the two can hold still across a "drain 2, queue 1" step. `flush` only ever grows
            // between drains (and every non-empty drain bumps the rev), so the pair is exact.
            revision: (self.main_log_rev << 32) | (self.flush.len() as u64 & 0xffff_ffff),
            lines: Vec::new(),
            line_count: self.main_log.len() + self.flush.len(),
        });

        // Subagents, in spawn order.
        for r in &self.subagents {
            views.push(TranscriptView {
                kind: ActivityKind::Subagent,
                title: r.agent.clone(),
                subtitle: r.task.clone(),
                model: r.model.clone(),
                status: if r.done {
                    ActivityStatus::Done
                } else {
                    ActivityStatus::Running
                },
                cost: r.cost,
                revision: r.log.len() as u64,
                lines: Vec::new(),
                line_count: r.log.len(),
            });
        }

        // Assay critics, in panel order.
        for c in &self.assay_critics {
            use forge_types::AssayCriticStatus;
            let (status, subtitle) = match &c.status {
                AssayCriticStatus::Queued => (ActivityStatus::Running, c.focus.clone()),
                AssayCriticStatus::Done { candidates } => (
                    ActivityStatus::Done,
                    format!("{candidates} found · {}", c.focus),
                ),
                AssayCriticStatus::Skipped { reason } => {
                    (ActivityStatus::Skipped, format!("skipped ({reason})"))
                }
            };
            views.push(TranscriptView {
                kind: ActivityKind::AssayCritic,
                title: c.lens.clone(),
                subtitle,
                model: c.model.clone(),
                status,
                cost: c.cost_usd,
                revision: c.output.len() as u64,
                lines: Vec::new(),
                line_count: c.output.lines().count(),
            });
        }

        views
    }

    /// The pre-styled transcript of activity entry `idx` (indices as in [`activity_view_headers`]).
    /// This is the expensive half — a main-chat clone or a markdown render — so build only the entry
    /// about to be drawn, and only when its revision says the last one is stale.
    fn activity_view_lines(&self, idx: usize) -> Vec<TextLine<'static>> {
        if !self.has_activity() {
            return Vec::new();
        }
        if idx == 0 {
            let mut lines = self.main_log.clone();
            lines.extend(self.flush.iter().cloned());
            return lines;
        }
        // Subagent transcripts are streamed token-fragments assembled into lines — render as plain
        // styled text (markdown would mangle the partial streaming).
        if let Some(r) = self.subagents.get(idx - 1) {
            if r.log.iter().all(|l| l.trim().is_empty()) {
                return vec![TextLine::from(Span::styled(
                    "(no activity captured yet)",
                    styles::muted(),
                ))];
            }
            return r
                .log
                .iter()
                .map(|l| {
                    let style = if l.starts_with("── result") {
                        styles::muted()
                    } else {
                        styles::fg()
                    };
                    TextLine::from(Span::styled(l.clone(), style))
                })
                .collect();
        }
        match self.assay_critics.get(idx - 1 - self.subagents.len()) {
            Some(c) if c.output.trim().is_empty() => vec![TextLine::from(Span::styled(
                "(no output yet)",
                styles::muted(),
            ))],
            Some(c) => crate::render::markdown_to_lines(&c.output),
            None => Vec::new(),
        }
    }

    /// Toggle the effort slider popup.
    pub fn toggle_effort_slider(&mut self) {
        self.effort_slider = !self.effort_slider;
    }

    /// Move the effort slider one step left (lower).
    pub fn effort_slider_left(&mut self) {
        use forge_types::EffortLevel::*;
        let levels = [Low, Medium, High, XHigh, WhiteHot];
        let cur = self.effort.unwrap_or(Medium);
        let i = levels.iter().position(|&l| l == cur).unwrap_or(1);
        if i > 0 {
            self.effort = Some(levels[i - 1]);
        }
    }

    /// Move the effort slider one step right (higher).
    pub fn effort_slider_right(&mut self) {
        use forge_types::EffortLevel::*;
        let levels = [Low, Medium, High, XHigh, WhiteHot];
        let cur = self.effort.unwrap_or(Medium);
        let i = levels.iter().position(|&l| l == cur).unwrap_or(1);
        if i + 1 < levels.len() {
            self.effort = Some(levels[i + 1]);
        }
    }

    /// Update the active temper label. The colored statusline segment is the live indicator —
    /// switching no longer spams a scrollback line per change (that flooded the view on rapid
    /// SHIFT+TAB cycling).
    pub fn set_temper(&mut self, label: &str) {
        self.temper = label.to_string();
    }

    /// Begin a single-question `ask_user` (the original shape; also what plan approval uses).
    pub fn set_question(&mut self, question: &str, options: &[QChoice], allow_other: bool) {
        self.open_form(vec![forge_types::Question::single(
            question,
            options,
            allow_other,
        )]);
    }

    /// Begin an `ask_user` form. The questions render in the live region (not scrollback) until
    /// answered; the answers are then recorded in scrollback by [`App::finish_form`].
    pub fn open_form(&mut self, questions: Vec<forge_types::Question>) {
        if questions.is_empty() {
            return;
        }
        self.question_prompt = Some(questions[0].text.clone());
        self.form = Some(crate::QuestionForm::new(questions));
        self.prompt = None;
    }

    /// True while a question is awaiting an answer.
    pub fn awaiting_question(&self) -> bool {
        self.form.is_some()
    }

    /// Drop any in-flight question/permission prompt (e.g. when the turn is interrupted).
    pub fn clear_question(&mut self) {
        self.form = None;
        self.prompt = None;
        self.question_prompt = None;
    }

    /// Feed a key to the open form. `None` when no form is open; otherwise the form's verdict,
    /// with `Submit`/`Cancel` already torn down and recorded in scrollback.
    pub fn form_key(&mut self, key: KeyKind) -> Option<crate::FormOutcome> {
        let form = self.form.as_mut()?;
        let outcome = form.key(key);
        match &outcome {
            crate::FormOutcome::Open => {
                self.question_prompt = Some(form.question().text.clone());
            }
            crate::FormOutcome::Submit(answers) => {
                let answers = answers.clone();
                self.finish_form(Some(&answers));
            }
            crate::FormOutcome::Cancel => self.finish_form(None),
        }
        Some(outcome)
    }

    /// Close the form and write a compact record of each question + answer to scrollback.
    pub fn finish_form(&mut self, answers: Option<&[forge_types::Answer]>) {
        let Some(form) = self.form.take() else {
            return;
        };
        self.prompt = None;
        self.question_prompt = None;
        for (i, q) in form.questions.iter().enumerate() {
            self.flush.push(lines::rail::text_row(
                RailKind::User,
                &q.text,
                styles::user().bold(),
            ));
            let (text, style) = match answers.and_then(|a| a.get(i)) {
                Some(a) if !a.is_empty() => (a.summary(), styles::ok()),
                Some(_) => ("(no answer)".to_string(), styles::muted()),
                None => ("(dismissed)".to_string(), styles::muted()),
            };
            self.flush
                .push(lines::rail::text_row(RailKind::User, &text, style));
        }
    }

    /// Apply a remote answer (`RemoteInput::Answer` text): either the structured form body or
    /// the legacy option-number / free-text shape for the current question.
    pub fn resolve_remote_answer(&mut self, text: &str) -> RemoteAnswer {
        let Some(form) = self.form.as_mut() else {
            return RemoteAnswer::Invalid;
        };
        let is_plan_approval = form.question().text.starts_with("Build this plan?");
        if let Some(answers) = crate::parse_structured_answers(text) {
            form.apply_structured(&answers);
            let answers = form.answers();
            self.finish_form(Some(&answers));
            self.drop_cancelled_plan(is_plan_approval, &answers);
            return RemoteAnswer::Complete(answers);
        }
        if !form.apply_text(text) {
            return RemoteAnswer::Invalid;
        }
        if form.current + 1 < form.len() {
            form.current += 1;
            self.question_prompt = Some(form.question().text.clone());
            return RemoteAnswer::Partial;
        }
        let answers = form.answers();
        self.finish_form(Some(&answers));
        self.drop_cancelled_plan(is_plan_approval, &answers);
        RemoteAnswer::Complete(answers)
    }

    /// A discarded plan must not stay in the remote snapshot (and `GET /api/plans`) as if it were
    /// still pending or being built.
    fn drop_cancelled_plan(&mut self, is_plan_approval: bool, answers: &[forge_types::Answer]) {
        if is_plan_approval
            && answers
                .first()
                .is_some_and(|a| a.summary().eq_ignore_ascii_case("cancel"))
        {
            self.plan = None;
        }
    }

    /// Try to resolve a submitted line against a single active question, returning the answer
    /// as the original tool did (an option label or the free text). `None` = invalid input.
    pub fn resolve_question(&mut self, line: &str) -> Option<String> {
        match self.resolve_remote_answer(line) {
            RemoteAnswer::Complete(answers) => Some(
                answers
                    .first()
                    .map(forge_types::Answer::summary)
                    .unwrap_or_else(|| crate::NO_ANSWER.to_string()),
            ),
            RemoteAnswer::Partial => Some(String::new()),
            RemoteAnswer::Invalid => None,
        }
    }

    /// Flush accumulated reasoning into scrollback as a thinking block (once), if any. Hidden
    /// by default it is one faint line (`▸ Thought for 12s`); `/thinking` opens it.
    fn flush_reasoning(&mut self) {
        if self.reasoning.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.reasoning);
        let secs = self
            .transcript_ui
            .reasoning_since
            .take()
            .map_or(0, |t| t.elapsed().as_secs())
            .max(1);
        self.ensure_assistant_label();
        let rows = lines::turn::thinking_rows(&text, secs, self.show_thinking);
        // Reasoning is the model talking, so it rides the wire as assistant provenance — a
        // remote client renders it under the same speaker, dimmed, exactly like the TUI does.
        self.push_turn_lines(rows);
    }

    /// Clamp `input_cursor` into a valid byte index: never past the end, and always on a UTF-8
    /// char boundary. The cursor can drift out of sync (e.g. a cleared input left it > 0, or it
    /// landed inside a multibyte char) — slicing `input` at such a position panics, which is the
    /// class of crash this guards. Cheap to call before any slice into `input`.
    fn sanitize_cursor(&mut self) {
        self.input_cursor = self.input_cursor.min(self.input.len());
        if !self.input.is_char_boundary(self.input_cursor) {
            self.input_cursor = prev_char_boundary(&self.input, self.input_cursor);
        }
    }

    /// Insert a bracketed paste at the cursor. Single-line pastes are inserted as plain text;
    /// multiline pastes show a `[pasted text (N lines)]` placeholder so the input stays on one
    /// line and won't accidentally auto-submit when the pasted text contains newlines.
    pub fn handle_paste(&mut self, content: String) {
        self.sanitize_cursor();
        let content = sanitize_paste(&content);
        if !content.contains('\n') && content.chars().count() <= 2000 {
            // Short single-line: insert directly as if the user typed it.
            self.input.insert_str(self.input_cursor, &content);
            self.input_cursor += content.len();
            return;
        }
        let n = content.lines().count();
        let placeholder = if n <= 1 {
            format!("[pasted text ({} chars)]", content.chars().count())
        } else {
            format!("[pasted text ({n} lines)]")
        };
        self.insert_block(placeholder, PasteKind::Text(content));
    }

    /// Attach an image as an input block: a `[image (<label>)]` placeholder is inserted at the
    /// cursor (deletable as a unit, like a text paste), and the image is sent as vision input when
    /// the prompt is submitted. `label` is a short human descriptor, e.g. "PNG 800x600".
    pub fn attach_image(&mut self, image: forge_types::ImageAttachment, label: &str) {
        self.sanitize_cursor();
        let placeholder = format!("[image ({label})]");
        self.insert_block(placeholder, PasteKind::Image(image));
    }

    /// Insert a placeholder at the cursor and register its backing paste-block.
    fn insert_block(&mut self, placeholder: String, kind: PasteKind) {
        self.input.insert_str(self.input_cursor, &placeholder);
        self.input_cursor += placeholder.len();
        self.paste_blocks.push(PasteBlock { placeholder, kind });
    }

    /// If cursor is immediately after (Backspace) or at (DeleteForward) a paste-block placeholder,
    /// delete the entire placeholder in one action. Returns `true` if consumed.
    pub fn try_delete_paste_block(&mut self, key: KeyKind) -> bool {
        self.sanitize_cursor();
        match key {
            KeyKind::Backspace => {
                let found = {
                    let before = &self.input[..self.input_cursor];
                    self.paste_blocks
                        .iter()
                        .position(|b| before.ends_with(&b.placeholder))
                        .map(|i| (i, self.paste_blocks[i].placeholder.len()))
                };
                if let Some((idx, ph_len)) = found {
                    let start = self.input_cursor - ph_len;
                    self.input.drain(start..self.input_cursor);
                    self.input_cursor = start;
                    self.paste_blocks.remove(idx);
                    true
                } else {
                    false
                }
            }
            KeyKind::DeleteForward => {
                let found = {
                    let after = &self.input[self.input_cursor..];
                    self.paste_blocks
                        .iter()
                        .position(|b| after.starts_with(&b.placeholder))
                        .map(|i| (i, self.paste_blocks[i].placeholder.len()))
                };
                if let Some((idx, ph_len)) = found {
                    let end = self.input_cursor + ph_len;
                    self.input.drain(self.input_cursor..end);
                    self.paste_blocks.remove(idx);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Resolve every paste-block placeholder in `text`: text blocks are substituted back inline,
    /// image blocks are stripped from the text and their attachments returned (for vision input).
    /// Call with the line from `handle_key`'s Submit. Drains `paste_blocks`.
    pub fn resolve_paste_blocks(
        &mut self,
        text: String,
    ) -> (String, Vec<forge_types::ImageAttachment>) {
        let mut result = text;
        let mut images = Vec::new();
        for block in self.paste_blocks.drain(..) {
            let Some(pos) = result.find(&block.placeholder) else {
                // Placeholder was deleted from the input — drop the block (and its image) too.
                continue;
            };
            let span = pos..pos + block.placeholder.len();
            match block.kind {
                PasteKind::Text(content) => result.replace_range(span, &content),
                PasteKind::Image(img) => {
                    result.replace_range(span, "");
                    images.push(img);
                }
            }
        }
        // Stash a copy so the user-turn echo (submit_user) can show a marker per image, since the
        // placeholders were just stripped from the text.
        self.last_submit_images = images.clone();
        (result, images)
    }

    /// Echo a just-submitted user message into scrollback. Any images attached to this prompt
    /// (stashed by `resolve_paste_blocks`) are shown as a marker line each, so the conversation
    /// history reflects that an image was sent (terminals can't render the pixels inline here).
    pub fn submit_user(&mut self, line: &str) {
        // A new prompt starts a new turn: the previous turn's diff card and plan card are
        // history now (still in scrollback), so the remote projections reset here.
        self.pending_diff = None;
        self.turn_diffs.clear();
        self.turn_diffs_skipped = 0;
        self.plan = None;
        self.begin_user_turn();
        if self.echo_harness_notice(line) {
            return;
        }
        let peer = crate::peer_message::parse(line);
        let origin = if peer.is_some() {
            LineOrigin::harness("peer")
        } else {
            LineOrigin::user()
        };
        self.tag_flush(origin, |s| {
            match &peer {
                Some((who, _)) => s.flush.push(lines::turn::peer_label(who)),
                None => s.flush.push(lines::turn::user_label(None)),
            }
            let shown = peer.as_ref().map_or(line, |(_, body)| body.as_str());
            s.echo_user_text(shown);
            for img in std::mem::take(&mut s.last_submit_images) {
                let kb = (img.data_base64.len() * 3 / 4).div_ceil(1024);
                s.flush.push(lines::turn::user_text(&format!(
                    "🖼 image attached ({}, ~{kb} KB)",
                    img.media_type
                )));
            }
            s.flush.push(TextLine::default());
        });
    }

    /// Render a resumed session's prior transcript into scrollback (after a `/resume` swap), so the
    /// FULL conversation reappears — user turns, assistant text, AND the tool calls/results between
    /// them — instead of a sparse user-only echo. User turns echo like live input; assistant turns
    /// render markdown under the `⚒ forge` header; tool activity renders exactly like it did live.
    /// Seed what a resumed session reports before its next turn ends: how the last turn ended and
    /// the model it last ran on. Without it the remote snapshot showed no outcome and a `—` model
    /// until a new turn completed. Never overwrites a value a live turn already set, and leaves
    /// `turns_finished` alone (it counts this process's turns; see `Session::restored_turn_state`).
    pub fn seed_restored_turn_state(
        &mut self,
        last_stop_reason: Option<forge_types::StopReason>,
        model: Option<String>,
    ) {
        if self.last_stop_reason.is_none() {
            self.last_stop_reason = last_stop_reason;
        }
        if self.routing.is_none() {
            if let Some(model) = model {
                self.routing = Some(RoutingView {
                    tier: String::new(),
                    model,
                    rationale: "restored from the previous run".to_string(),
                });
            }
        }
    }

    pub fn replay_history(&mut self, items: &[ReplayItem]) {
        for item in items {
            match item {
                ReplayItem::User(content) => self.submit_user(content),
                ReplayItem::Assistant(content) => {
                    self.ensure_assistant_label();
                    self.push_turn_lines(crate::render::markdown_to_lines(content));
                }
                ReplayItem::Tool { name, args } => {
                    self.ensure_assistant_label();
                    // Same card the live turn showed, instead of a raw JSON line.
                    self.open_tool_card(LineOrigin::tool_call(name), name.clone(), args.clone());
                }
                ReplayItem::ToolResult { name, ok, summary } => {
                    if !self.close_tool_card(name, *ok, summary, None) {
                        self.ensure_assistant_label();
                        self.print_orphan_result(name, *ok, summary, None);
                    }
                }
                ReplayItem::Note(text) => {
                    let (kind, body) = lines::notice::classify(text, NoticeKind::Warn);
                    self.push_notice(kind, &body);
                }
            }
        }
        // A replayed turn has no footer: its timings are not this process's.
        self.flush_notice_run();
        if self.transcript_ui.turn_open {
            self.flush.push(TextLine::default());
            self.transcript_ui.turn_open = false;
        }
        self.transcript_ui.cooling = None;
    }

    /// Update the visible queued-prompts list (prompts typed while a turn was running).
    pub fn set_queued(&mut self, queued: &[String]) {
        self.queued = queued.to_vec();
        // Drop images whose prompt left the queue by any route (removed, absorbed, interrupted).
        let mut remaining = self.queued.clone();
        self.queued_images.retain(|(text, _)| {
            remaining
                .iter()
                .position(|q| q == text)
                .map(|i| remaining.remove(i))
                .is_some()
        });
    }

    /// Remember the images of a prompt queued mid-turn. Call before [`Self::set_queued`].
    pub fn stash_queued_images(&mut self, text: &str, images: Vec<forge_types::ImageAttachment>) {
        if !images.is_empty() {
            self.queued_images.push((text.to_string(), images));
        }
    }

    /// A queued prompt is about to run: move its stashed images (if any) to the ready slot.
    pub fn promote_queued_images(&mut self, text: &str) {
        if let Some(i) = self.queued_images.iter().position(|(t, _)| t == text) {
            let (_, images) = self.queued_images.remove(i);
            self.last_submit_images = images.clone();
            self.ready_images = images;
        }
    }

    /// Images the turn being spawned must carry (empty unless a queued prompt had some).
    pub fn take_ready_images(&mut self) -> Vec<forge_types::ImageAttachment> {
        std::mem::take(&mut self.ready_images)
    }

    /// Push a dim separator line after replaying a resumed session's transcript, so the user
    /// can tell where the prior history ends and new input begins.
    pub fn push_resume_separator(&mut self, text: &str) {
        self.flush.push(TextLine::default());
        self.flush
            .extend(lines::notice::notice_lines(NoticeKind::Info, false, text));
        self.flush.push(TextLine::default());
    }

    /// Register an idle Esc press. Returns true when it completes a double-tap (the caller opens
    /// the rewind picker); otherwise the press is armed and the statusline says "Esc again".
    pub fn esc_tap(&mut self) -> bool {
        let now = std::time::Instant::now();
        if self
            .esc_armed
            .is_some_and(|at| now.duration_since(at) < ESC_DOUBLE_TAP)
        {
            self.esc_armed = None;
            return true;
        }
        self.esc_armed = Some(now);
        false
    }

    /// Drop an expired Esc arm. Returns true when it did (the statusline hint needs a redraw).
    pub fn expire_esc_hint(&mut self) -> bool {
        if self
            .esc_armed
            .is_some_and(|at| at.elapsed() >= ESC_DOUBLE_TAP)
        {
            self.esc_armed = None;
            return true;
        }
        false
    }

    /// Whether the "Esc again to rewind" hint is live.
    pub fn esc_hint_active(&self) -> bool {
        self.esc_armed
            .is_some_and(|at| at.elapsed() < ESC_DOUBLE_TAP)
    }

    /// Render the streamed assistant text as finished scrollback and end the stream. The text
    /// is one segment of the turn: a segment identical to the one before it (a re-answer after
    /// the completeness review) is dropped.
    pub(super) fn finish_streaming_text(&mut self) {
        let rest = std::mem::take(&mut self.streaming);
        self.streaming_rev = self.streaming_rev.wrapping_add(1);
        let cache = std::mem::take(&mut *self.stream_cache.borrow_mut());
        if !rest.is_empty() {
            let rows = cache.finish(&rest);
            self.push_answer_segment(&rest, rows);
        }
        self.streaming_active = false;
        self.flush_deferred_notices();
    }

    /// Echo a prompt the running turn just picked up from the queue (`PresenterEvent::Steered`),
    /// where it landed in the conversation. Deliberately does NOT reset the turn's diff/plan
    /// projections the way `submit_user` does — this is the same turn continuing.
    fn echo_steered(&mut self, text: &str) {
        if self.echo_harness_notice(text) {
            return;
        }
        // The steer is a new message in the conversation: the assistant turn it interrupted
        // ends here (the answer that follows opens a new one).
        self.flush_notice_run();
        if self.transcript_ui.turn_open {
            self.flush.push(TextLine::default());
            self.transcript_ui.turn_open = false;
            self.transcript_ui.cooling = None;
        }
        self.tag_flush(LineOrigin::user(), |s| {
            s.flush.push(lines::turn::user_label(Some("steer")));
            s.echo_user_text(text);
            s.flush.push(TextLine::default());
        });
    }

    /// Push a dim informational line into scrollback (command feedback, session lists, etc).
    ///
    /// A note comes from the surface (a queued prompt, a toggle), not from the turn, so it says
    /// nothing about where the answer being streamed ends. The reply is rendered as one markdown
    /// segment when it ends; closing it here would cut a sentence, a list or a code block in two
    /// around the note. It is printed right after that segment instead.
    pub fn note(&mut self, text: &str) {
        let (kind, body) = lines::notice::classify(text, NoticeKind::Info);
        if self.streaming_active && !self.streaming.is_empty() {
            self.transcript_ui.deferred_notices.push((kind, body));
            return;
        }
        self.push_notice(kind, &body);
    }

    /// Push plain (unstyled) multi-line text into the scrollback outbox — for verbatim blocks
    /// like a QR code whose alignment must not be restyled. Drained with the next flush.
    pub fn print_lines(&mut self, lines: Vec<String>) {
        for l in lines {
            self.flush.push(TextLine::from(l));
        }
    }

    /// Emit scrollback whose provenance the caller knows: everything `push` queues is tagged
    /// `origin`. Lines already queued untagged are sealed as system notes first, so a later tag
    /// can never reach back and reclassify an unrelated warning that landed before it.
    fn tag_flush(&mut self, origin: LineOrigin, push: impl FnOnce(&mut Self)) {
        self.seal_flush_origins(&LineOrigin::default());
        push(self);
        self.seal_flush_origins(&origin);
    }

    /// Give every still-untagged queued line the provenance `origin`.
    fn seal_flush_origins(&mut self, origin: &LineOrigin) {
        while self.flush_origins.len() < self.flush.len() {
            self.flush_origins.push(origin.clone());
        }
    }

    /// Take the finalized scrollback lines queued since the last call. Each line is also mirrored
    /// into [`main_log`] so the activity viewer can show the full "main chat" transcript.
    pub fn drain_flush(&mut self) -> Vec<TextLine<'static>> {
        let lines = std::mem::take(&mut self.flush);
        self.flush_origins.clear();
        self.remote_only.clear();
        self.place_pending_cards();
        self.fold_main_log(&lines);
        self.inline_rows(lines)
    }

    /// Rows for the terminal's native scrollback. They are wrapped here, with the rail and the
    /// hanging indent repeated on every row, because the terminal would wrap them without. In
    /// full-screen the transcript wraps at draw time, so the rows pass through.
    fn inline_rows(&self, lines: Vec<TextLine<'static>>) -> Vec<TextLine<'static>> {
        if self.fullscreen {
            return lines;
        }
        let width = match self.last_width.get() {
            0 => 80,
            w => w,
        };
        crate::transcript::wrap_lines(&lines, width.saturating_sub(1) as usize)
    }

    /// Resolve each queued card's `main_log` position: the log's current length plus the card's
    /// offset within the flush about to be appended. Called before `fold_main_log`, which may then
    /// shift every position when it trims the front of the ring.
    fn place_pending_cards(&mut self) {
        if self.pending_card_marks.is_empty() {
            return;
        }
        let base = self.main_log.len();
        for (offset, id, len) in std::mem::take(&mut self.pending_card_marks) {
            if let Some(card) = self.tool_cards.iter_mut().find(|c| c.id == id) {
                card.start = base + offset;
                card.len = len;
            }
        }
    }

    /// Like [`drain_flush`], but also folds each line's plain text (and the provenance the
    /// emitter recorded) into the remote transcript ring buffer so the remote-control snapshot
    /// mirrors the conversation tail. Use this when remote control is active; otherwise
    /// [`drain_flush`] is cheaper.
    pub fn drain_flush_remote(&mut self) -> Vec<TextLine<'static>> {
        self.seal_flush_origins(&LineOrigin::default());
        let lines = std::mem::take(&mut self.flush);
        let origins = std::mem::take(&mut self.flush_origins);
        for (l, origin) in lines.iter().zip(origins.iter()) {
            self.push_remote_transcript_line(l, origin);
        }
        for (line, origin) in std::mem::take(&mut self.remote_only) {
            self.push_remote_transcript_line(&line, &origin);
        }
        self.place_pending_cards();
        self.fold_main_log(&lines);
        self.inline_rows(lines)
    }

    /// Append out-of-band scrollback (banner, command output, `/clear` marker, …) directly into the
    /// full-screen transcript log. In inline mode these lines go to the terminal's native scrollback
    /// instead (the I/O shell calls `Tui::insert_lines`); in full-screen mode the transcript IS the
    /// log, so they must land here. Auto-tails the view (`transcript_follow`).
    pub fn push_scrollback(&mut self, lines: Vec<TextLine<'static>>) {
        self.fold_main_log(&lines);
    }

    /// Like [`push_scrollback`] but for plain multi-line text (e.g. command output that isn't
    /// pre-styled). Full-screen counterpart to `Tui::print_text`.
    pub fn push_scrollback_text(&mut self, text: &str) {
        let lines: Vec<TextLine<'static>> =
            text.lines().map(|s| TextLine::from(s.to_owned())).collect();
        self.fold_main_log(&lines);
    }

    /// Serialize the live view (activity panel, viewer, scroll, tasks) to JSON for session storage.
    /// Returns `None` when there's nothing worth persisting (no activity, no viewer, no tasks) so a
    /// plain chat session doesn't write an empty blob every turn.
    pub fn view_snapshot_json(&self) -> Option<String> {
        if self.subagents.is_empty()
            && self.assay_critics.is_empty()
            && self.tasks.is_empty()
            && self.viewer.is_none()
        {
            return None;
        }
        serde_json::to_string(&self.view_snapshot()).ok()
    }

    /// Restore a view previously captured by [`view_snapshot_json`] (best-effort; malformed or
    /// stale JSON is ignored). Called on resume after the transcript has been replayed.
    pub fn restore_view_json(&mut self, json: &str) {
        if let Ok(snap) = serde_json::from_str::<ViewSnapshot>(json) {
            self.restore_view(snap);
        }
    }

    fn view_snapshot(&self) -> ViewSnapshot {
        ViewSnapshot {
            subagents: self
                .subagents
                .iter()
                .map(|r| SubagentSnapshot {
                    id: r.id.clone(),
                    agent: r.agent.clone(),
                    task: r.task.clone(),
                    model: r.model.clone(),
                    phase: r.phase.clone(),
                    last: r.last.clone(),
                    log: r.log.clone(),
                    done: r.done,
                    ok: r.ok,
                    cost: r.cost,
                })
                .collect(),
            assay_critics: self.assay_critics.clone(),
            assay_verifying: self.assay_verifying,
            tasks: self.tasks.clone(),
            activity_focused: self.activity_focused,
            activity_idx: self.activity_idx,
            viewer: self.viewer.clone(),
            transcript_scroll: self.transcript_scroll,
            transcript_follow: self.transcript_follow,
        }
    }

    fn restore_view(&mut self, s: ViewSnapshot) {
        self.subagents = s
            .subagents
            .into_iter()
            .map(|r| SubRow {
                id: r.id,
                agent: r.agent,
                task: r.task,
                model: r.model,
                phase: r.phase,
                last: r.last,
                log: r.log,
                done: r.done,
                ok: r.ok,
                cost: r.cost,
            })
            .collect();
        self.assay_critics = s.assay_critics;
        self.assay_verifying = s.assay_verifying;
        if !s.tasks.is_empty() {
            self.tasks = s.tasks;
        }
        self.activity_focused = s.activity_focused;
        self.activity_idx = s.activity_idx;
        self.viewer = s.viewer;
        self.transcript_scroll = s.transcript_scroll;
        self.transcript_follow = s.transcript_follow;
        self.clamp_activity_selection();
    }

    /// Open the in-loop activity viewer at `selected` (full-screen mode). Renders over the whole
    /// frame using the main terminal — no nested alternate screen.
    pub fn open_viewer(&mut self, selected: usize) {
        let n = self.activity_len();
        if n == 0 {
            return;
        }
        self.viewer = Some(ViewerState {
            selected: selected.min(n - 1),
            ..ViewerState::default()
        });
    }

    /// Feed a keystroke to the in-loop activity viewer. Returns true if the viewer consumed it (it
    /// is open). Esc closes it. ↑↓/PgUp/PgDn scroll, ←→/Tab switch entry, g/G top/tail.
    pub fn viewer_key(&mut self, key: KeyKind) -> bool {
        let n = self.activity_len().max(1);
        // Read render-recorded scroll geometry BEFORE the mutable borrow of `viewer` (Cell::get
        // needs `&self`). Lets a downward scroll that reaches the tail re-arm follow.
        let geom = self.viewer_geom.get();
        let Some(v) = self.viewer.as_mut() else {
            return false;
        };
        match key {
            KeyKind::Esc => self.viewer = None,
            KeyKind::Up => {
                Self::viewer_pin_scroll_before_up(v, geom);
                v.follow = false;
                v.scroll = v.scroll.saturating_sub(1);
            }
            KeyKind::Down => {
                v.scroll = v.scroll.saturating_add(1);
                Self::viewer_refollow_at_tail(v, geom);
            }
            KeyKind::PageUp => {
                Self::viewer_pin_scroll_before_up(v, geom);
                v.follow = false;
                v.scroll = v.scroll.saturating_sub(10);
            }
            KeyKind::PageDown => {
                v.scroll = v.scroll.saturating_add(10);
                Self::viewer_refollow_at_tail(v, geom);
            }
            KeyKind::Home => {
                v.follow = false;
                v.scroll = 0;
            }
            KeyKind::End => {
                v.follow = true;
                v.scroll = usize::MAX / 2;
            }
            KeyKind::Right | KeyKind::Tab => {
                v.selected = (v.selected + 1) % n;
                v.scroll = usize::MAX / 2;
                v.follow = true;
            }
            KeyKind::Left => {
                v.selected = (v.selected + n - 1) % n;
                v.scroll = usize::MAX / 2;
                v.follow = true;
            }
            KeyKind::Char('q') => self.viewer = None,
            KeyKind::Char('k') => {
                Self::viewer_pin_scroll_before_up(v, geom);
                v.follow = false;
                v.scroll = v.scroll.saturating_sub(1);
            }
            KeyKind::Char('j') | KeyKind::Char(' ') => {
                v.scroll = v.scroll.saturating_add(1);
                Self::viewer_refollow_at_tail(v, geom);
            }
            KeyKind::Char('g') => {
                v.follow = false;
                v.scroll = 0;
            }
            KeyKind::Char('G') => {
                v.follow = true;
                v.scroll = usize::MAX / 2;
            }
            _ => {}
        }
        true
    }

    /// Before an upward scroll, pin the tail sentinel down to the real bottom row. While `follow`
    /// is true `v.scroll` sits at the raw `usize::MAX / 2` init value (the render path substitutes
    /// its own `usize::MAX / 2` for display and clamps THAT copy, never writing the clamped value
    /// back to `v.scroll`). Left unpinned, Up would decrement the untouched sentinel and the
    /// render's `.min(max_scroll)` clamp would keep showing the same tail row — Up doing nothing —
    /// for roughly `usize::MAX / 2 - max_scroll` presses. `geom` is `(wrapped_len, body_h)`
    /// recorded by the render path; absent (no frame drawn yet) leaves `scroll` as-is.
    fn viewer_pin_scroll_before_up(v: &mut ViewerState, geom: Option<(usize, u16)>) {
        if v.follow {
            if let Some((wrapped_len, body_h)) = geom {
                v.scroll = wrapped_len.saturating_sub(body_h as usize);
            }
        }
    }

    /// When a downward scroll reaches the tail (last full page), clamp and re-arm follow so new
    /// activity auto-tails again — matching the full-screen browser (transcript.rs) and `End`/`G`.
    /// `geom` is `(wrapped_len, body_h)` recorded by the render path; absent → leave follow as-is.
    fn viewer_refollow_at_tail(v: &mut ViewerState, geom: Option<(usize, u16)>) {
        if let Some((wrapped_len, body_h)) = geom {
            let max_scroll = wrapped_len.saturating_sub(body_h as usize);
            if v.scroll >= max_scroll {
                v.scroll = max_scroll;
                v.follow = true;
            }
        }
    }

    /// Clear the full-screen transcript (`/clear`, `/new`): wipe the rendered log, the activity
    /// panel, and the viewer, re-anchoring at the tail. The session/transcript on disk are
    /// untouched — this only resets what's drawn (and what a snapshot would capture).
    pub fn clear_transcript(&mut self) {
        self.main_log.clear();
        self.main_log_rev += 1;
        // Placed cards pointed into the log just cleared; only cards still queued in the flush
        // have rows left to own.
        let queued: Vec<u64> = self.pending_card_marks.iter().map(|m| m.1).collect();
        self.tool_cards.retain(|c| queued.contains(&c.id));
        self.fold_ui.clear();
        self.subagents.clear();
        self.assay_critics.clear();
        self.assay_verifying = None;
        self.workflow = Default::default();
        self.viewer = None;
        self.activity_focused = false;
        self.activity_idx = 0;
        self.transcript_scroll = 0;
        self.transcript_follow = true;
        self.suggested_prompt = None; // stale once the session it was predicted for is gone
        self.turn_activity = TurnActivity::default();
        let seen = self.transcript_ui.bridge_notice_seen;
        self.transcript_ui = Default::default();
        self.transcript_ui.bridge_notice_seen = seen;
    }

    /// Zero the statusline's usage counters, for a brand-new session (`/new`).
    pub fn reset_session_stats(&mut self) {
        self.session_in = 0;
        self.session_cached_in = None;
        self.session_out = 0;
        self.context_tokens = 0;
        self.turn_elapsed_secs = 0;
        self.turn_in = 0;
        self.turn_cached_in = None;
        self.turn_out = 0;
        self.turn_base_in = 0;
        self.turn_base_cached_in = None;
        self.turn_base_out = 0;
        self.cost_usd = 0.0;
    }

    /// Wrapped-row metrics for the full-screen transcript: total rows at the given width, and the
    /// max scroll offset given a visible body height. Used by the I/O shell to clamp paging.
    pub fn transcript_metrics(&self, width: u16, body_h: u16) -> (usize, usize) {
        let total = self.transcript_total_rows(width);
        let max_scroll = total.saturating_sub(body_h.max(1) as usize);
        (total, max_scroll)
    }

    /// Scroll the full-screen transcript up by `rows` (toward the top); pauses auto-follow.
    pub fn transcript_scroll_up(&mut self, rows: usize) {
        // While following, `transcript_scroll` is a sentinel (usize::MAX/2, or 0 after a clear), not
        // the row on screen: scrolling up from it did nothing or jumped to the top. Start from the
        // row the last frame actually rendered at.
        let base = match self.transcript_geom.get() {
            Some(g) if self.transcript_follow || self.transcript_scroll > g.scroll => g.scroll,
            _ => self.transcript_scroll,
        };
        self.transcript_follow = false;
        self.transcript_scroll = base.saturating_sub(rows);
    }

    /// Scroll the full-screen transcript down by `rows`; resumes follow at the bottom.
    pub fn transcript_scroll_down(&mut self, rows: usize, max_scroll: usize) {
        self.transcript_scroll = (self.transcript_scroll + rows).min(max_scroll);
        if self.transcript_scroll >= max_scroll {
            self.transcript_follow = true;
        }
    }

    /// Jump to the top of the full-screen transcript; pauses follow.
    pub fn transcript_to_top(&mut self) {
        self.transcript_follow = false;
        self.transcript_scroll = 0;
    }

    /// Jump to the tail of the full-screen transcript; resumes follow.
    pub fn transcript_to_bottom(&mut self) {
        self.transcript_follow = true;
        self.transcript_scroll = usize::MAX / 2;
    }

    /// Wrapped-row indices where a user message starts, ascending, at the last rendered width.
    fn user_message_rows(&self) -> Vec<usize> {
        let Some(g) = self.transcript_geom.get() else {
            return Vec::new();
        };
        self.ensure_wrapped_main(g.width);
        let cache = self.wrap_cache.borrow();
        let mut rows = Vec::new();
        for (row, &line) in cache.origins.iter().enumerate() {
            if row > 0 && cache.origins[row - 1] == line {
                continue; // a later wrapped row of the same line
            }
            let is_header = self
                .main_log
                .get(line)
                .is_some_and(lines::rail::is_user_label);
            if is_header {
                rows.push(row);
            }
        }
        rows
    }

    /// Whether at least one user message starts above the row the last frame scrolled to — i.e.
    /// whether the "previous message" bar has somewhere to go.
    pub fn has_user_message_above(&self) -> bool {
        let Some(g) = self.transcript_geom.get() else {
            return false;
        };
        self.user_message_rows()
            .first()
            .is_some_and(|&r| r < g.scroll)
    }

    /// Jump the full-screen transcript so the most recent user message ABOVE the current view
    /// sits at the top; pressed again it walks to the one before it. Pauses follow. Uses the
    /// geometry of the last rendered frame, so it is a no-op before the first draw or when
    /// nothing is above.
    pub fn transcript_to_prev_user(&mut self) -> bool {
        let Some(g) = self.transcript_geom.get() else {
            return false;
        };
        let rows = self.user_message_rows();
        let Some(&target) = rows.iter().rev().find(|&&r| r < g.scroll) else {
            return false;
        };
        self.transcript_follow = false;
        self.transcript_scroll = target;
        true
    }

    /// Wrap the in-loop activity viewer's selected entry to `width` (already the wrap width, i.e.
    /// one column short of the area), reusing the previous frame's rows unless the selection, the
    /// width, or that entry's content revision changed. The viewer redraws at the render loop's full
    /// rate, so without this every frame cloned the whole transcript and wrapped it twice.
    /// `headers` must be the same list `selected` indexes into, from this frame.
    fn ensure_viewer_wrapped(
        &self,
        headers: &[TranscriptView],
        selected: usize,
        width: u16,
    ) -> std::cell::Ref<'_, ViewerWrapCache> {
        let key = headers.get(selected).map(|v| (selected, width, v.revision));
        {
            let mut c = self.viewer_wrap.borrow_mut();
            if c.key != key {
                c.rows = match key {
                    Some(_) => crate::transcript::wrap_lines(
                        &self.activity_view_lines(selected),
                        width as usize,
                    ),
                    None => Vec::new(),
                };
                c.key = key;
            }
        }
        self.viewer_wrap.borrow()
    }

    /// Drop the viewer's wrapped rows once it closes, so the transcript copy they hold doesn't
    /// outlive the view that needed it.
    fn forget_viewer_wrapped(&self) {
        let mut c = self.viewer_wrap.borrow_mut();
        if c.key.is_some() {
            *c = ViewerWrapCache::default();
        }
    }

    /// Refresh the wrap cache for `width` if `main_log` or the width changed. Re-wrapping the whole
    /// log every frame was O(transcript) and showed up as input/scroll lag on long full-screen
    /// conversations; the cache makes a streaming frame re-wrap only the one in-flight edge line.
    fn ensure_wrapped_main(&self, width: u16) {
        let mut c = self.wrap_cache.borrow_mut();
        let fold = self.fold_key();
        if c.width != width || c.rev != self.main_log_rev || c.fold_key != fold {
            if fold.enabled {
                self.wrap_folded(width, &mut c);
            } else {
                let (rows, origins) = crate::transcript::wrap_lines_indexed(
                    &self.main_log,
                    width.saturating_sub(1) as usize,
                );
                c.rows = rows;
                c.origins = origins;
                c.fold_rows.clear();
                c.group_hits.clear();
            }
            c.width = width;
            c.rev = self.main_log_rev;
            c.fold_key = fold;
        }
    }

    /// Bring the in-flight reply's rows up to date. Finished markdown blocks are rendered and
    /// wrapped once; only the open block at the end is re-rendered as tokens arrive, so the cost
    /// of a frame does not grow with the length of the reply.
    fn ensure_stream_cache(&self, width: u16) {
        let mut cache = self.stream_cache.borrow_mut();
        cache.set_rail(Some(styles::RailKind::Live));
        cache.refresh(&self.streaming, self.streaming_rev, width);
    }

    /// The reply text past what the cached rows cover, wrapped as plain rows so a token that
    /// arrived since the last refresh is still on screen.
    fn streaming_tail_rows(&self, width: u16, cached_len: usize) -> Vec<TextLine<'static>> {
        let tail = &self.streaming[cached_len.min(self.streaming.len())..];
        if tail.is_empty() {
            Vec::new()
        } else {
            let lines: Vec<TextLine<'static>> = tail
                .split('\n')
                .map(|l| TextLine::raw(l.to_string()))
                .collect();
            crate::transcript::wrap_lines(&lines, width.saturating_sub(1) as usize)
        }
    }

    /// The in-flight reply edge, markdown-rendered and wrapped to `width`, with the orange cursor
    /// block appended to the final visible row. Only the requested window is cloned.
    fn streaming_edge(&self, width: u16, start: usize, len: usize) -> Vec<TextLine<'static>> {
        if !self.streaming_active {
            return Vec::new();
        }
        self.ensure_stream_cache(width);
        let cache = self.stream_cache.borrow();
        let tail = self.streaming_tail_rows(width, cache.len());
        let cached_rows = cache.row_count();
        let total = (cached_rows + tail.len()).max(1);
        let end = start.saturating_add(len).min(total);
        let mut rows = cache.window(start, end.min(cached_rows));
        if end > cached_rows {
            rows.extend(
                tail.into_iter()
                    .skip(start.saturating_sub(cached_rows))
                    .take(end - cached_rows),
            );
        }
        if end == total {
            let cursor = Span::styled(crate::theme::glyphs::g().caret, styles::brand_hot());
            match rows.last_mut() {
                Some(line) => line.spans.push(cursor),
                None => rows.push(TextLine::from(cursor)),
            }
        }
        rows
    }

    /// Wrapped-row count of the streaming edge without cloning it (for scroll math).
    fn streaming_edge_len(&self, width: u16) -> usize {
        if !self.streaming_active {
            return 0;
        }
        self.ensure_stream_cache(width);
        let cache = self.stream_cache.borrow();
        (cache.row_count() + self.streaming_tail_rows(width, cache.len()).len()).max(1)
    }

    /// Total wrapped rows of the full-screen transcript (memoized log + the streaming edge).
    fn transcript_total_rows(&self, width: u16) -> usize {
        self.ensure_wrapped_main(width);
        self.wrap_cache.borrow().rows.len() + self.streaming_edge_len(width)
    }

    /// Map a screen cell to a transcript position (wrapped-row, col) if it's inside the transcript
    /// area, using the geometry captured at the last render.
    fn pointer_to_text(&self, col: u16, row: u16) -> Option<TextPos> {
        let g = self.transcript_geom.get()?;
        if row < g.row0 || row >= g.row0 + g.height || col < g.col0 || col >= g.col0 + g.width {
            return None;
        }
        Some((g.scroll + (row - g.row0) as usize, col - g.col0))
    }

    /// True if a screen cell is on the floating "previous message" bar.
    pub fn prev_bar_hit(&self, col: u16, row: u16) -> bool {
        match self.prev_bar_geom.get() {
            Some((br, c0, w)) => row == br && col >= c0 && col < c0 + w,
            None => false,
        }
    }

    /// True if a screen cell is on the floating jump-to-bottom bar.
    pub fn jump_bar_hit(&self, col: u16, row: u16) -> bool {
        match self.jump_bar_geom.get() {
            Some((br, c0, w)) => row == br && col >= c0 && col < c0 + w,
            None => false,
        }
    }

    /// Begin a text selection at a screen cell (left-button down in the transcript). Returns false
    /// if the cell isn't inside the transcript area (the caller may treat it as a click elsewhere).
    pub fn selection_begin(&mut self, col: u16, row: u16) -> bool {
        match self.pointer_to_text(col, row) {
            Some(p) => {
                self.selection = Some((p, p));
                true
            }
            None => false,
        }
    }

    /// Extend the active selection to a screen cell (left-button drag), clamped to the area.
    pub fn selection_extend(&mut self, col: u16, row: u16) {
        let Some((anchor, _)) = self.selection else {
            return;
        };
        let Some(g) = self.transcript_geom.get() else {
            return;
        };
        let cr = row.clamp(g.row0, g.row0 + g.height.saturating_sub(1));
        let cc = col.clamp(g.col0, g.col0 + g.width.saturating_sub(1));
        if let Some(p) = self.pointer_to_text(cc, cr) {
            self.selection = Some((anchor, p));
        }
    }

    /// The currently selected text, or `None` if nothing/empty is selected. The shell copies this
    /// to the clipboard on button release. Decoration (frames, gutters) is left out; see
    /// [`crate::copy_mark`].
    pub fn selection_text(&self) -> Option<String> {
        let (a, b) = self.selection?;
        let ((r0, c0), (r1, c1)) = if a <= b { (a, b) } else { (b, a) };
        let width = self.transcript_geom.get()?.width;
        self.ensure_wrapped_main(width);
        self.ensure_stream_cache(width);
        let cache = self.wrap_cache.borrow();
        let stream = self.stream_cache.borrow();
        let committed = cache.rows.len();
        // The rendered transcript is the committed wrap-cache followed by the live streaming
        // edge; a selection can span that boundary, so pull each row from whichever side it
        // falls in. Without the streaming side, a copy ending in the in-flight reply was cut off.
        let rows = (r0..=r1).map_while(|r| {
            let line = if r < committed {
                cache.rows.get(r)
            } else if self.streaming_active {
                stream.row(r - committed)
            } else {
                None
            }?;
            let from = if r == r0 { c0 as usize } else { 0 };
            let to = if r == r1 { c1 as usize } else { usize::MAX };
            Some((line, from, to))
        });
        let out = crate::copy_mark::selection_text(rows);
        let trimmed = out.trim_end_matches('\n').to_string();
        (!trimmed.trim().is_empty()).then_some(trimmed)
    }

    /// Clear any active selection (e.g. a fresh click or a new turn).
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// Whether a selection is currently held (for the shell to decide click vs select).
    pub fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    /// Append flushed lines to the bounded main-chat log used by the activity viewer.
    fn fold_main_log(&mut self, lines: &[TextLine<'static>]) {
        if lines.is_empty() {
            return;
        }
        self.main_log.extend(lines.iter().cloned());
        if self.main_log.len() > MAIN_LOG_MAX {
            let drop = self.main_log.len() - MAIN_LOG_MAX;
            self.main_log.drain(0..drop);
            if let Some(c) = &mut self.transcript_ui.cooling {
                c.from = c.from.saturating_sub(drop);
            }
            if self
                .transcript_ui
                .notice_run
                .as_ref()
                .is_some_and(|r| r.start < drop)
            {
                self.transcript_ui.notice_run = None;
            } else if let Some(r) = &mut self.transcript_ui.notice_run {
                r.start -= drop;
                r.end -= drop;
            }
            // Every card's recorded position is an index into `main_log`, so trimming the front
            // moves all of them. A card whose own lines were (partly) trimmed no longer has a
            // region to rewrite and is dropped — expanding it would splice over its neighbours.
            self.tool_cards.retain_mut(|card| {
                if card.start < drop {
                    false
                } else {
                    card.start -= drop;
                    true
                }
            });
        }
        self.main_log_rev += 1;
    }

    fn push_remote_transcript_line(&mut self, line: &TextLine<'static>, origin: &LineOrigin) {
        // A card's body (diff preview, failure tail) and a turn's footer are for the terminal:
        // the phone has its own diff cards and keeps a short ring of what was said and run.
        if origin.meta.as_deref() == Some("local") || lines::rail::is_card_body(line) {
            return;
        }
        // The rail is decoration for a terminal; a phone draws its own. Rows keep the two-cell
        // indent they have always had on the wire, and the assistant's label stays the
        // `⚒ forge` header the daemon's supervisor looks for when it extracts a reply.
        let railed = line.spans.first().is_some_and(crate::copy_mark::is_rail);
        let mut plain: String = if lines::rail::is_assistant_label(line) {
            REMOTE_ASSISTANT_HEADER.to_string()
        } else {
            line.spans
                .iter()
                .filter(|s| !crate::copy_mark::is_rail(s))
                .map(|s| s.content.as_ref())
                .collect()
        };
        if plain.trim().is_empty() {
            return;
        }
        if railed {
            plain.insert_str(0, "  ");
        }
        let plain = remote_transcript_text(&plain, origin);
        self.recent_transcript.push_back(TranscriptRow {
            kind: origin.kind,
            text: plain,
            tool: origin.tool.clone(),
            meta: origin.meta.clone(),
        });
        self.transcript_total += 1;
        while self.recent_transcript.len() > REMOTE_TRANSCRIPT_MAX {
            self.recent_transcript.pop_front();
        }
    }
}

// ---- Scrollback line builders (own their text; identical styling to the old panel). ----

/// Flatten to one line, then delegate to the shared [`forge_types::truncate_ellipsis`] so the
/// truncation-length semantics stay in lockstep with the rest of the codebase.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    forge_types::truncate_ellipsis(&s.replace('\n', " "), max)
}

/// Width-aware truncation budget: scales with the terminal width but never drops below `min` (the
/// old fixed cap, so narrow terminals are unchanged). `reserve` accounts for the columns already
/// taken by the line's glyph/label prefix. A width of 0 (pre-first-render) falls back to 80.
fn width_cap(width: u16, reserve: usize, min: usize) -> usize {
    let w = if width == 0 { 80 } else { width as usize };
    w.saturating_sub(reserve).max(min)
}

mod card_flow;
mod group_flow;
mod lines;
mod state;
mod transcript_flow;
pub use lines::banner::{banner_lines, print_banner_direct};
pub(crate) use lines::rail::rail_lines;
pub use lines::turn::lattice_view_lines;
use lines::{
    agents::{
        subagent_branch_line, subagent_footer_line, subagent_header_line, workflow_finished_line,
        workflow_log_line, workflow_phase_line, workflow_started_line,
    },
    plan::plan_card_lines,
    turn::{lattice_line, shell_diagnosis_lines},
};
mod idle_interrupt;
pub use idle_interrupt::IdleInterrupt;
mod harness_notice;
mod job_notice;
mod output_view;
mod render;
mod screen_select;
#[cfg(test)]
mod steer_esc_tests;
pub use output_view::{page_file, OutputAction, OutputView};
mod tool_cards;
mod tool_groups;
/// The shared list component, for the takeover screens outside `app` (Help, select).
pub(crate) use render::chrome::hint as chrome_hint;
pub(crate) use render::list_view;
pub(crate) use render::{human, mesh_pace_suffix, model_short, needs_phase_header};
pub use render::{
    input_box_height, input_cursor_up, render_live, render_mesh_overlay, render_usage_overlay,
    render_voice_overlay,
};
#[cfg(test)]
pub(crate) use tool_cards::CardStatus;
pub(crate) use tool_cards::ToolCard;

#[cfg(test)]
mod stream_perf_tests;

#[cfg(test)]
mod tests;
