//! Surface-independent interaction contracts emitted by the session core.
//!
//! The core owns this interface; terminal, headless, and remote renderers are adapters at the
//! interaction seam (ADR-0004).

use crate::{
    AssayCriticRow, AssayReport, EffortLevel, FileDiff, McpServerLine, PlanProposal, SideEffect,
    StopReason, TodoItem,
};

/// Sentinel used when a question cannot be answered interactively.
pub const NO_ANSWER: &str = "(no answer — non-interactive)";

/// One choice in a user question.
#[derive(Debug, Clone)]
pub struct QChoice {
    pub label: String,
    pub description: String,
}

/// Things the session core wants to show as a turn progresses.
#[derive(Debug, Clone)]
pub enum PresenterEvent {
    SessionStarted {
        id: String,
    },
    Routing {
        tier: String,
        model: String,
        rationale: String,
    },
    /// A concrete provider request is about to start. Unlike `Routing` (which can precede context
    /// assembly), this marks the exact model-loop boundary used by live progress surfaces.
    ProviderRequest {
        model: String,
        /// Zero-based agentic model/tool-loop step.
        step: usize,
    },
    AssistantText(String),
    AssistantDelta(String),
    Reasoning(String),
    AssistantDone,
    Warning(String),
    Error(String),
    ModelSearch {
        model: String,
        retrying: bool,
    },
    ToolStart {
        name: String,
        args: String,
    },
    ToolResult {
        name: String,
        ok: bool,
        summary: String,
    },
    Cost {
        session_total_usd: f64,
        session_in: u64,
        session_out: u64,
        context_tokens: u64,
        context_limit: Option<u32>,
    },
    SubagentStart {
        id: String,
        agent: String,
        task: String,
        model: Option<String>,
        phase: Option<String>,
    },
    SubagentProgress {
        id: String,
        snippet: String,
    },
    SubagentResult {
        id: String,
        agent: String,
        ok: bool,
        summary: String,
        cost_usd: f64,
    },
    Diff(FileDiff),
    AssayProgress(String),
    AssayCriticRow(AssayCriticRow),
    AssayVerifying {
        candidates: usize,
    },
    AssayReport(AssayReport),
    Tasks(Vec<TodoItem>),
    McpStatus(Vec<McpServerLine>),
    ContextInjected {
        symbols: usize,
        files: usize,
        tokens: usize,
    },
    Recap {
        text: String,
    },
    SuggestionReady {
        text: String,
    },
    ShellDiagnosis {
        command: String,
        diagnosis: String,
        fix: Option<String>,
    },
    Done {
        final_text: String,
        stop_reason: StopReason,
    },
    QuotaUpdate {
        provider: String,
        window: String,
        fraction: f64,
    },
    QuotaPace {
        provider: String,
        window: String,
        rate_per_hour: f64,
        projected_fraction_at_reset: Option<f64>,
        exhaustion_warning: bool,
    },
    CustomWidgetOutput {
        id: String,
        text: String,
    },
    CompactionStarted {
        auto: bool,
    },
    CompactionFinished {
        before: usize,
        after: usize,
    },
    PlanProposed(PlanProposal),
    Temper(String),
    Effort(Option<EffortLevel>),
    WorkflowStarted {
        name: Option<String>,
    },
    WorkflowPhase {
        title: String,
    },
    WorkflowLog(String),
    WorkflowFinished {
        ok: bool,
        summary: String,
    },
}

/// Outcome of a side-effect confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmOutcome {
    Allow,
    AlwaysAllow,
    Deny,
}

/// Rendering and interaction interface implemented by every Forge surface adapter.
pub trait Presenter: Send {
    fn emit(&mut self, event: PresenterEvent);
    fn confirm(&mut self, tool: &str, side_effect: SideEffect) -> ConfirmOutcome;
    fn ask(&mut self, question: &str, options: &[QChoice], allow_other: bool) -> String;
    fn read_line(&mut self) -> Option<String>;
    fn recap_sink(&self) -> Option<Box<dyn Presenter>> {
        None
    }
}

/// One surface-independent item in a resumed session transcript.
#[derive(Debug, Clone)]
pub enum ReplayItem {
    User(String),
    Assistant(String),
    Tool {
        name: String,
        args: String,
    },
    ToolResult {
        name: String,
        ok: bool,
        summary: String,
    },
    Note(String),
}
