//! Shared, dependency-free domain types used across every Forge crate.
//!
//! This is a leaf crate (no internal dependencies) so the workspace graph stays acyclic:
//! provider, mesh, tools, store, core and tui all depend on it, it depends on none of them.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod interaction;
pub use interaction::{ConfirmOutcome, Presenter, PresenterEvent, QChoice, ReplayItem, NO_ANSWER};

/// Who produced a message in a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }

    /// Parse the stored string form back into a `Role`.
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "system" => Some(Role::System),
            "user" => Some(Role::User),
            "assistant" => Some(Role::Assistant),
            "tool" => Some(Role::Tool),
            _ => None,
        }
    }
}

/// Who a transcript message is for. `Llm` is shared provider/user transcript, `LlmOnly` preserves
/// provider continuity without publishing provisional text, and `UiOnly` is user-facing chrome
/// that never spends provider context.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    #[default]
    Llm,
    LlmOnly,
    UiOnly,
}

impl Visibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Visibility::Llm => "llm",
            Visibility::LlmOnly => "llm_only",
            Visibility::UiOnly => "ui",
        }
    }
    pub fn parse(s: &str) -> Visibility {
        match s {
            "ui" => Visibility::UiOnly,
            "llm_only" => Visibility::LlmOnly,
            _ => Visibility::Llm,
        }
    }
    pub fn is_llm(&self) -> bool {
        matches!(self, Visibility::Llm | Visibility::LlmOnly)
    }
    pub fn is_default(&self) -> bool {
        matches!(self, Visibility::Llm)
    }
    pub fn is_user_visible(&self) -> bool {
        !matches!(self, Visibility::LlmOnly)
    }
}

/// An image attached to a user message (vision input). `data_base64` is the raw image bytes,
/// base64-encoded; `media_type` is the MIME type (e.g. "image/png"). Carried on the user `Message`
/// and mapped to the provider's multimodal content parts at request time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageAttachment {
    pub media_type: String,
    pub data_base64: String,
}

/// A single message in a session transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// Tool calls the assistant requested in this turn (empty otherwise). Carried so the
    /// transcript can be replayed to a provider as a faithful tool-calling round-trip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// For a `Tool` message, the id of the call this result answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Images attached to a user message (vision input). Empty for non-user messages and
    /// text-only turns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageAttachment>,
    /// Who this message is for. `UiOnly` messages are stripped before every provider call.
    #[serde(default, skip_serializing_if = "Visibility::is_default")]
    pub visibility: Visibility,
}

impl Message {
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            images: Vec::new(),
            visibility: Visibility::Llm,
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self::new(Role::User, content)
    }
    /// A user message carrying attached images (vision input) alongside the text.
    pub fn user_with_images(content: impl Into<String>, images: Vec<ImageAttachment>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            images,
            visibility: Visibility::Llm,
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(Role::Assistant, content)
    }
    pub fn system(content: impl Into<String>) -> Self {
        Self::new(Role::System, content)
    }
    /// An assistant turn that requested tool calls.
    pub fn assistant_tool_calls(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls,
            tool_call_id: None,
            images: Vec::new(),
            visibility: Visibility::Llm,
        }
    }
    /// A tool result answering a specific call.
    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
            images: Vec::new(),
            visibility: Visibility::Llm,
        }
    }
    /// Mark this message as UI-only: shown to the user (and persisted), never sent to a model.
    pub fn ui_only(mut self) -> Self {
        self.visibility = Visibility::UiOnly;
        self
    }
    /// Keep this provider-visible for continuation/replay, but hide it from user conversation.
    pub fn llm_only(mut self) -> Self {
        self.visibility = Visibility::LlmOnly;
        self
    }
}

/// A request from a model to invoke a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Arguments as a JSON object.
    pub args: serde_json::Value,
}

/// Token counts and computed cost for one provider call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Of `input_tokens`, how many were served from the provider's prompt cache (billed at a
    /// fraction of the input rate). 0 when caching is unused/unsupported. Subset of `input_tokens`,
    /// not additive. Used so cost reflects the cache-read discount the provider actually bills.
    #[serde(default)]
    pub cached_input_tokens: u64,
    pub cost_usd: f64,
}

impl Usage {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

/// The Model Mesh's difficulty classification for a task (ADR-0006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskTier {
    Trivial,
    Standard,
    Complex,
}

impl TaskTier {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskTier::Trivial => "trivial",
            TaskTier::Standard => "standard",
            TaskTier::Complex => "complex",
        }
    }

    /// Parse a tier name (`"trivial"`/`"standard"`/`"complex"`, case-insensitive). `None` otherwise.
    pub fn from_name(s: &str) -> Option<TaskTier> {
        match s.trim().to_ascii_lowercase().as_str() {
            "trivial" => Some(TaskTier::Trivial),
            "standard" => Some(TaskTier::Standard),
            "complex" => Some(TaskTier::Complex),
            _ => None,
        }
    }

    /// The next tier up (trivial→standard→complex), clamped at the top.
    pub fn up(self) -> TaskTier {
        match self {
            TaskTier::Trivial => TaskTier::Standard,
            TaskTier::Standard | TaskTier::Complex => TaskTier::Complex,
        }
    }

    /// The next tier down (complex→standard→trivial), clamped at the bottom.
    pub fn down(self) -> TaskTier {
        match self {
            TaskTier::Complex => TaskTier::Standard,
            TaskTier::Standard | TaskTier::Trivial => TaskTier::Trivial,
        }
    }

    /// True when already at the highest tier (`up()` is a no-op).
    pub fn is_max(self) -> bool {
        self == TaskTier::Complex
    }

    /// True when already at the lowest tier (`down()` is a no-op).
    pub fn is_min(self) -> bool {
        self == TaskTier::Trivial
    }
}

/// What project/codebase the current session is operating in — lets the mesh classifier reason
/// about stakes beyond the raw prompt text. Computed once per session (see
/// `forge_core::project_context::compute`, which needs file I/O this crate deliberately avoids)
/// and passed into `Router::route`/`route_hinted` on every call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectContext {
    /// The project root's package name (from Cargo.toml `[package]`), if determinable.
    pub project_name: Option<String>,
    /// True when this project IS the same source tree the running binary was itself built from —
    /// a structural comparison (compile-time package identity vs. the runtime project's own
    /// Cargo.toml), not a keyword match on any one project's literal name. Used to weight
    /// otherwise-ordinary infrastructure vocabulary ("mesh", "router", "classifier"...) as
    /// higher-stakes only when the agent is actually touching its own core routing logic.
    pub is_self_hosting: bool,
}

// ---- Assay: AI-slop / quality analysis (docs/features/analysis-mode.md) ----

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Critical => "critical",
            Severity::High => "high",
            Severity::Medium => "medium",
            Severity::Low => "low",
        }
    }
    pub fn parse(s: &str) -> Option<Severity> {
        match s.trim().to_lowercase().as_str() {
            "critical" | "crit" => Some(Severity::Critical),
            "high" => Some(Severity::High),
            "medium" | "med" => Some(Severity::Medium),
            "low" => Some(Severity::Low),
            _ => None,
        }
    }
    /// Higher = more severe (for ranking, since the enum's declaration order would sort the
    /// other way).
    pub fn weight(self) -> u8 {
        match self {
            Severity::Critical => 3,
            Severity::High => 2,
            Severity::Medium => 1,
            Severity::Low => 0,
        }
    }
}

/// Post-verification confidence that a finding is real.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::High => "high",
            Confidence::Medium => "medium",
            Confidence::Low => "low",
        }
    }
    pub fn parse(s: &str) -> Option<Confidence> {
        match s.trim().to_lowercase().as_str() {
            "high" => Some(Confidence::High),
            "medium" | "med" => Some(Confidence::Medium),
            "low" => Some(Confidence::Low),
            _ => None,
        }
    }
    pub fn weight(self) -> u8 {
        match self {
            Confidence::High => 2,
            Confidence::Medium => 1,
            Confidence::Low => 0,
        }
    }
}

/// The lens a critic applies. Mechanical lenses route to the cheap/local tier; judgment lenses
/// to the frontier tier (FR-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindingCategory {
    DeadWeight,
    Correctness,
    Unsafe,
    TestCoverage,
    Design,
    Architecture,
    DocumentationRot,
    OverEngineering,
}

impl FindingCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            FindingCategory::DeadWeight => "dead-weight",
            FindingCategory::Correctness => "correctness",
            FindingCategory::Unsafe => "unsafe",
            FindingCategory::TestCoverage => "test-coverage",
            FindingCategory::Design => "design",
            FindingCategory::Architecture => "architecture",
            FindingCategory::DocumentationRot => "documentation",
            FindingCategory::OverEngineering => "over-engineering",
        }
    }
    pub fn parse(s: &str) -> Option<FindingCategory> {
        match s.trim().to_lowercase().as_str() {
            "dead-weight" | "dead" | "deadweight" => Some(FindingCategory::DeadWeight),
            "correctness" | "bug" | "bugs" => Some(FindingCategory::Correctness),
            "unsafe" => Some(FindingCategory::Unsafe),
            "test-coverage" | "tests" | "test" => Some(FindingCategory::TestCoverage),
            "design" => Some(FindingCategory::Design),
            "architecture" | "arch" => Some(FindingCategory::Architecture),
            "documentation" | "docs" | "doc" => Some(FindingCategory::DocumentationRot),
            "over-engineering" | "over-eng" | "overeng" | "ai-slop" | "slop" => {
                Some(FindingCategory::OverEngineering)
            }
            _ => None,
        }
    }
    /// The intended Model-Mesh tier for this lens: mechanical scans are cheap, judgment is
    /// frontier (`docs/features/analysis-mode.md` §U4).
    pub fn tier(self) -> TaskTier {
        match self {
            FindingCategory::DeadWeight
            | FindingCategory::Unsafe
            | FindingCategory::TestCoverage => TaskTier::Trivial,
            _ => TaskTier::Complex,
        }
    }
    /// The v0.1 critic crew, in display order.
    pub fn crew() -> &'static [FindingCategory] {
        &[
            FindingCategory::DeadWeight,
            FindingCategory::Correctness,
            FindingCategory::Unsafe,
            FindingCategory::TestCoverage,
            FindingCategory::Design,
            FindingCategory::Architecture,
            FindingCategory::DocumentationRot,
            FindingCategory::OverEngineering,
        ]
    }
}

/// Reasoning / thinking intensity hint forwarded to the model (e.g. OpenAI `reasoning_effort`,
/// Anthropic extended-thinking budget). Maps to [`genai::chat::ReasoningEffort`] on the genai
/// path; ignored by CLI-bridge providers that manage their own thinking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EffortLevel {
    Low,
    Medium,
    High,
    XHigh,
    /// Above XHigh — the forge at its hottest, where the metal glows white. Same maximum
    /// reasoning intensity as XHigh, PLUS a standing per-turn instruction to orchestrate
    /// substantive work through `run_workflow` scripts automatically
    /// (docs/features/whitehot-effort.md). Providers have no knob above xhigh — the extra
    /// lift comes from the orchestration guidance, not a provider setting.
    WhiteHot,
}

impl EffortLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            EffortLevel::Low => "low",
            EffortLevel::Medium => "medium",
            EffortLevel::High => "high",
            EffortLevel::XHigh => "xhigh",
            EffortLevel::WhiteHot => "whitehot",
        }
    }

    /// Parse case-insensitively from "low", "medium", "high", "xhigh", "whitehot".
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "low" => Some(EffortLevel::Low),
            "medium" | "med" => Some(EffortLevel::Medium),
            "high" => Some(EffortLevel::High),
            "xhigh" | "x-high" | "extra-high" => Some(EffortLevel::XHigh),
            "whitehot" | "white-hot" | "ultra" | "max" => Some(EffortLevel::WhiteHot),
            _ => None,
        }
    }
}

/// Rough fix effort for a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Trivial,
    #[default]
    Small,
    Medium,
    Large,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Trivial => "trivial",
            Effort::Small => "small",
            Effort::Medium => "medium",
            Effort::Large => "large",
        }
    }
    pub fn parse(s: &str) -> Option<Effort> {
        match s.trim().to_lowercase().as_str() {
            "trivial" => Some(Effort::Trivial),
            "small" => Some(Effort::Small),
            "medium" | "med" => Some(Effort::Medium),
            "large" => Some(Effort::Large),
            _ => None,
        }
    }
}

/// What part of the repo an assay run covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssayScope {
    Repo,
    Path(String),
    /// Uncommitted working-tree changes (git diff).
    Diff,
    /// Files changed between this branch and `base` (git diff <base>...).
    Branch(String),
    /// Files changed since a git ref (git diff <ref> --name-only).
    Since(String),
}

impl AssayScope {
    pub fn label(&self) -> String {
        match self {
            AssayScope::Repo => "repo".to_string(),
            AssayScope::Path(p) => format!("path {p}"),
            AssayScope::Diff => "diff (working tree)".to_string(),
            AssayScope::Branch(b) => format!("branch vs {b}"),
            AssayScope::Since(r) => format!("since {r}"),
        }
    }
}

/// One verified problem the crew surfaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub category: FindingCategory,
    pub severity: Severity,
    pub confidence: Confidence,
    pub file: String,
    pub line: Option<u32>,
    /// One-line "what's wrong".
    pub title: String,
    /// WHY it's a problem (the critic's reasoning).
    pub rationale: String,
    pub suggested_fix: String,
    pub effort: Effort,
    /// Which lens raised it.
    pub lens: String,
    /// Survived adversarial verification.
    pub verified: bool,
}

/// The full result of an assay run, findings pre-sorted by (severity, confidence).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssayReport {
    pub run_id: String,
    pub scope: AssayScope,
    pub findings: Vec<Finding>,
    pub cost_usd: f64,
    /// Lenses that errored out, with the reason — graceful degradation.
    pub skipped_lenses: Vec<(String, String)>,
}

impl AssayReport {
    /// Sort findings by severity (most severe first), then confidence, then category for a
    /// stable order. Mutates in place.
    pub fn rank(&mut self) {
        self.findings.sort_by(|a, b| {
            b.severity
                .weight()
                .cmp(&a.severity.weight())
                .then(b.confidence.weight().cmp(&a.confidence.weight()))
                .then(a.category.as_str().cmp(b.category.as_str()))
        });
    }

    /// Count of findings per severity, for the summary header.
    pub fn severity_counts(&self) -> [usize; 4] {
        let mut c = [0usize; 4];
        for f in &self.findings {
            c[match f.severity {
                Severity::Critical => 0,
                Severity::High => 1,
                Severity::Medium => 2,
                Severity::Low => 3,
            }] += 1;
        }
        c
    }
}

/// Live status of one critic lens during an assay run, for per-row TUI progress tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AssayCriticStatus {
    Queued,
    Done { candidates: usize },
    Skipped { reason: String },
}

/// One row in the live assay critics panel: the lens name + its current status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssayCriticRow {
    pub lens: String,
    /// One-line description of what this lens checks (e.g. "bugs, wrong logic, panics").
    pub focus: String,
    /// The model that ran (or will try first for) this critic.
    pub model: Option<String>,
    /// Cost of this critic call in USD — 0.0 until the critic completes.
    pub cost_usd: f64,
    /// Raw model output captured when the critic finished (empty while queued/skipped).
    pub output: String,
    pub status: AssayCriticStatus,
}

/// Session-level tool-safety posture (ADR-0008). Exposed in the UI as the **temper** (the
/// forge/metallurgy framing for the agent's disposition); see `docs/features/temper-modes.md`.
/// Serde accepts both the canonical kebab key and the temper-label alias.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionMode {
    /// Ask before any side effect. Temper: **Ask**.
    #[default]
    #[serde(alias = "ask")]
    Default,
    /// Auto-allow file writes/edits; still ask for shell. Temper: **Auto-edit**.
    #[serde(alias = "auto-edit", alias = "autoedit")]
    AcceptEdits,
    /// Auto-allow everything (explicit, dangerous opt-in). Temper: **Full**.
    #[serde(alias = "full")]
    Bypass,
    /// Read-only: deny all side effects. Temper: **Read-only**.
    #[serde(alias = "read-only", alias = "readonly")]
    Plan,
}

impl PermissionMode {
    /// The temper label shown in the UI — names the permission plainly so the active posture
    /// is obvious at a glance (the dimension is themed "temper"; the values are descriptive).
    pub fn label(self) -> &'static str {
        match self {
            PermissionMode::Plan => "Read-only",
            PermissionMode::Default => "Ask",
            PermissionMode::AcceptEdits => "Auto-edit",
            PermissionMode::Bypass => "Full",
        }
    }

    /// One-line description of what this temper does, for the mode picker.
    pub fn description(self) -> &'static str {
        match self {
            PermissionMode::Plan => "analyze & plan only — no file edits or commands",
            PermissionMode::Default => "ask before every file edit and command",
            PermissionMode::AcceptEdits => "auto-apply file edits; still ask before shell commands",
            PermissionMode::Bypass => "auto-approve everything — dangerous, explicit opt-in",
        }
    }

    /// All tempers, safest → most permissive, for the mode picker (unlike the SHIFT+TAB cycle,
    /// the picker can reach `Full`/Bypass since it's an explicit, deliberate choice).
    pub fn all() -> &'static [PermissionMode] {
        &[
            PermissionMode::Plan,
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Bypass,
        ]
    }

    /// Parse a temper from its UI label (or canonical/kebab key) — used to resolve a picker row.
    pub fn from_label(s: &str) -> Option<PermissionMode> {
        match s.trim().to_lowercase().as_str() {
            "read-only" | "readonly" | "plan" => Some(PermissionMode::Plan),
            "ask" | "default" => Some(PermissionMode::Default),
            "accept-edits" | "auto-edit" | "autoedit" | "acceptedits" => {
                Some(PermissionMode::AcceptEdits)
            }
            "full" | "bypass" => Some(PermissionMode::Bypass),
            _ => None,
        }
    }

    /// Canonical kebab key (matches the serde rename) — stable for crossing a process boundary,
    /// e.g. the `FORGE_PERMISSION_MODE` env the parent hands its CLI-bridge `forge mcp-serve` child
    /// so the bridge gates on the parent's *runtime* temper, not the stale on-disk config mode.
    pub fn key(self) -> &'static str {
        match self {
            PermissionMode::Plan => "plan",
            PermissionMode::Default => "default",
            PermissionMode::AcceptEdits => "accept-edits",
            PermissionMode::Bypass => "bypass",
        }
    }

    /// Inverse of [`PermissionMode::key`] — exact, no fuzzy aliases.
    pub fn from_key(s: &str) -> Option<PermissionMode> {
        match s {
            "plan" => Some(PermissionMode::Plan),
            "default" => Some(PermissionMode::Default),
            "accept-edits" => Some(PermissionMode::AcceptEdits),
            "bypass" => Some(PermissionMode::Bypass),
            _ => None,
        }
    }

    /// The next temper in the SHIFT+TAB cycle. The cycle covers the three everyday tempers and
    /// **wraps** — `Bypass`/Full is intentionally excluded (reachable only via explicit
    /// `--mode bypass`/config, never by tapping a key). From Full, cycling re-enters
    /// the safe loop at Ask.
    pub fn cycle_next(self) -> PermissionMode {
        match self {
            PermissionMode::Default => PermissionMode::AcceptEdits, // Ask → Auto-edit
            PermissionMode::AcceptEdits => PermissionMode::Plan,    // Auto-edit → Read-only
            PermissionMode::Plan => PermissionMode::Default,        // Read-only → Ask (wrap)
            PermissionMode::Bypass => PermissionMode::Default,      // leave the unsafe temper
        }
    }
}

/// How aggressively Forge conserves metered API credits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CreditMode {
    /// No conservation — use the best model for each task (default).
    #[default]
    Normal,
    /// Prefer free/subscription models; cap output tokens at 2048; skip auto-probing.
    Frugal,
    /// Only route to free-tier or subscription models; cap output tokens at 1024.
    Strict,
}

impl CreditMode {
    pub fn label(self) -> &'static str {
        match self {
            CreditMode::Normal => "Normal",
            CreditMode::Frugal => "Frugal",
            CreditMode::Strict => "Strict",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            CreditMode::Normal => "no restriction — best model per task",
            CreditMode::Frugal => "prefer free/sub; cap output to 2048 tokens",
            CreditMode::Strict => "free/sub only; cap output to 1024 tokens",
        }
    }

    pub fn all() -> &'static [CreditMode] {
        &[CreditMode::Normal, CreditMode::Frugal, CreditMode::Strict]
    }
}

/// How "dangerous" a tool is — drives the permission decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SideEffect {
    /// No side effects (read/search/list) — never prompts.
    ReadOnly,
    /// Mutates files in the workspace.
    Write,
    /// Executes arbitrary shell commands.
    Shell,
    /// Reaches the network (web fetch/search) — distinct from a local read: egress can
    /// leak context or hit internal hosts, so it is gated separately from `ReadOnly`.
    Network,
    /// A call into an external MCP server (untrusted third-party tool). Gated like a side
    /// effect even when "read-shaped": the server is untrusted code and its result enters the
    /// agent loop, so MCP tool calls always go through the permission broker (mcp-client.md §6).
    External,
}

/// One line of `forge mcp` / `/mcp` server-status output. A dependency-free DTO so both
/// `forge-mcp` (which produces it) and `forge-tui` (which renders it) can share it without a
/// crate dependency between them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerLine {
    pub name: String,
    /// Human status word: connected / reconnecting / unauthorized / slow / failed / disabled.
    pub status: String,
    /// Transport label: "stdio" or "http".
    pub transport: String,
    pub tools: usize,
    pub resources: usize,
    pub prompts: usize,
    /// Extra detail for non-healthy states (the failure reason, retry attempt, latency, …).
    pub detail: Option<String>,
}

/// A single tracked task in the agent's todo list (the `update_tasks` tool). Mirrors the
/// TodoWrite pattern: the model maintains an ordered list and updates each item's status as it
/// works, giving the user a live view of multi-step progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub title: String,
    pub status: TodoStatus,
}

/// One step of a proposed plan (the `present_plan` tool). `detail` is an optional one-line
/// elaboration shown dimmed under the step title in the plan card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    pub title: String,
    #[serde(default)]
    pub detail: String,
}

/// A plan the agent proposes for review in planning mode (the `present_plan` tool): a titled,
/// ordered list of steps plus optional notes (risks/assumptions). Rendered as an interactive card;
/// on approval its steps seed the live task list and execution begins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanProposal {
    pub title: String,
    pub steps: Vec<PlanStep>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    #[default]
    Pending,
    InProgress,
    Done,
}

impl TodoStatus {
    /// Parse a status from the model's free-form string (tolerant of synonyms/casing/spacing).
    pub fn parse_loose(s: &str) -> Self {
        match s
            .trim()
            .to_ascii_lowercase()
            .replace([' ', '-'], "_")
            .as_str()
        {
            "in_progress" | "active" | "doing" | "started" | "wip" => Self::InProgress,
            "done" | "completed" | "complete" | "finished" => Self::Done,
            _ => Self::Pending,
        }
    }

    /// A checkbox glyph for the TUI list.
    pub fn marker(&self) -> &'static str {
        match self {
            Self::Pending => "☐",
            Self::InProgress => "◐",
            Self::Done => "☑",
        }
    }
}

/// Outcome of a permission check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    /// Must ask the user to confirm.
    Ask,
    Deny,
}

/// Where a permission rule came from. Drives precedence: a `Builtin` deny is a safety floor
/// that no configured rule and no permission mode (not even `Bypass`) can override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleSource {
    /// Shipped safety default (e.g. `rm -rf /`, secret reads) — unoverridable.
    Builtin,
    /// From a user or project `config.toml`.
    Configured,
}

/// One fine-grained allow/ask/deny rule (FR-10), matching a tool call by name + argument
/// pattern. The decision composes with the global [`PermissionMode`] in the broker.
#[derive(Debug, Clone)]
pub struct PermissionRule {
    /// Tool name to match, or `"*"` for any tool.
    pub tool: String,
    /// Glob patterns over the relevant argument (the effective shell command, or a file
    /// path). Empty means "match any arguments for this tool".
    pub patterns: Vec<String>,
    pub decision: PermissionDecision,
    pub source: RuleSource,
    /// Optional human reason, surfaced when the rule drives the decision.
    pub reason: Option<String>,
}

/// What kind of change a [`FileDiff`] represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffKind {
    Created,
    Modified,
    Deleted,
}

/// A proposed file change, computed *before* a write tool runs so the human can review it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub kind: DiffKind,
    /// Prior on-disk content (`None` for a created file).
    pub old: Option<String>,
    /// Proposed new content (`None` for a deleted file).
    pub new: Option<String>,
    /// Language token inferred from the extension; drives diff-body highlighting.
    pub lang: Option<String>,
    /// True → don't attempt a textual diff (non-UTF-8 target).
    pub binary: bool,
}

/// A new opaque identifier (UUID v4) as a string.
pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

/// Truncate `s` to at most `max` characters, appending an ellipsis (`…`) when truncated. The
/// ellipsis counts toward `max`, so the result is never longer than `max` characters. The single
/// canonical implementation — forge-core and forge-tui each used to carry their own copy of this,
/// with the copies having drifted (some took exactly `max` chars before the ellipsis, making the
/// result `max + 1` chars long).
pub fn truncate_ellipsis(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        format!(
            "{}…",
            s.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    } else {
        s.to_string()
    }
}

/// The argument keys a file-touching tool may use to name its target path. Centralized as a single
/// source of truth so the permission broker, the secret denylist, the pre-write snapshot, and the
/// in-process workspace confinement all key off the SAME set. A write tool that names its path arg
/// `file_path`/`target` (instead of `path`) must not slip past the secret deny or miss a snapshot.
/// Ordered by how common/canonical the key is — the first present string-valued key wins.
pub const PATH_ARG_KEYS: &[&str] = &[
    "path",
    "file_path",
    "filename",
    "file",
    "target",
    "target_file",
    "dest",
    "destination",
];

/// Extract a tool call's target file path from any of the known [`PATH_ARG_KEYS`]. Returns the
/// first present string value, or `None` if the args carry no recognizable path key (e.g. a shell
/// call keyed on `command`, or a tool that takes no path).
pub fn extract_path_arg(args: &serde_json::Value) -> Option<&str> {
    PATH_ARG_KEYS
        .iter()
        .find_map(|k| args.get(*k).and_then(|v| v.as_str()))
}

/// A snapshot of the models that are currently benched (rate-limited / unavailable / failed a
/// probe) and must not be routed to. Built by the store from the `model_health` table — only
/// models whose cooldown has not yet elapsed are included — and consulted by the mesh router.
/// Carries no clock or I/O: the time filtering happens where the snapshot is built.
/// A synthetic `model_health.model` key representing every model for one provider. Kept in the
/// existing health table so provider-wide authentication failures persist across sessions without
/// a schema migration.
pub const PROVIDER_BENCH_PREFIX: &str = "__forge_provider__::";

/// Stable health-table key for a provider-wide bench.
pub fn provider_bench_key(provider: &str) -> String {
    format!("{PROVIDER_BENCH_PREFIX}{provider}")
}

#[derive(Debug, Default, Clone)]
pub struct ModelHealth {
    benched: std::collections::HashSet<String>,
    benched_providers: std::collections::HashSet<String>,
}

impl ModelHealth {
    pub fn new(benched: std::collections::HashSet<String>) -> Self {
        let mut benched_providers = std::collections::HashSet::new();
        let mut benched_models = std::collections::HashSet::new();
        for entry in benched {
            if let Some(provider) = entry.strip_prefix(PROVIDER_BENCH_PREFIX) {
                benched_providers.insert(provider.to_string());
            } else {
                benched_models.insert(entry);
            }
        }
        Self {
            benched: benched_models,
            benched_providers,
        }
    }

    /// Whether `model` is currently benched and should be skipped by routing.
    pub fn is_benched(&self, model: &str) -> bool {
        self.benched.contains(model)
            || self.benched_providers.contains(
                model
                    .split_once("::")
                    .map_or(model, |(provider, _)| provider),
            )
    }

    /// True when no model is benched (the common case — lets the router skip filtering).
    pub fn is_empty(&self) -> bool {
        self.benched.is_empty() && self.benched_providers.is_empty()
    }
}

/// How a subscription is sitting relative to its rolling usage window (quota-aware routing, L3,
/// mesh-routing.md). Ordered so the stricter wins with `.max()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum QuotaStatus {
    /// Comfortably within the window.
    #[default]
    Ok,
    /// Near the window limit (or using overage) — demote the subscription below alternatives.
    Warning,
    /// At/over the limit — skip the subscription entirely (route around it), like a benched model.
    Exhausted,
}

/// One observation of a CLI-bridge subscription's quota, surfaced by the bridge's event stream
/// (e.g. Claude Code's `rate_limit_event`) alongside a completion. Recorded by the store so the
/// router can avoid overrunning a near-limit plan.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotaHint {
    /// Bridge provider prefix the quota belongs to (`claude-cli` / `codex-cli`).
    pub provider: String,
    /// The rolling-window kind the provider reported (`five_hour`, `weekly`, …); `""` if unknown.
    pub window: String,
    pub status: QuotaStatus,
    /// Epoch seconds when the window resets, if the provider told us.
    pub resets_at: Option<i64>,
    /// Fraction of the window consumed (0.0–1.0), if the provider told us.
    pub fraction_used: Option<f64>,
}

/// A snapshot of every subscription's current quota pressure, built by the store from the
/// `subscription_usage` table (rows whose window hasn't reset). Consulted by the mesh router to
/// demote or skip a pressured subscription. Carries no clock/I/O (filtering happens at build).
#[derive(Debug, Default, Clone)]
/// Documented in docs/features/mesh-routing.md.
pub struct SubscriptionQuota {
    by_provider: std::collections::HashMap<String, QuotaStatus>,
    /// Fraction (0.0–1.0) of the strictest active window consumed, per provider. Drives the
    /// graduated subscription-conservation spreading (distinct from the coarse `QuotaStatus`).
    fraction: std::collections::HashMap<String, f64>,
    /// Subscription plan slug per provider (`claude-cli` → `max-20x`, `codex-cli` → `plus`), from
    /// config. A larger plan has more headroom, so it is spent more freely.
    plans: std::collections::HashMap<String, String>,
    /// Pace projection per provider (mesh-routing.md), derived from `quota_history` — where
    /// the strictest active window is headed by its reset time, not just its current fraction.
    /// Absent when there isn't enough history to derive a rate (see `compute_quota_pace`).
    pace: std::collections::HashMap<String, QuotaPace>,
    /// Whether proactive subscription-conservation spreading is enabled (config opt-out).
    conserve: bool,
}

/// Maximum age of a Codex account-wide quota observation. Codex usage may be consumed through
/// either Forge's OAuth surface or the official CLI, so older data is not authoritative enough to
/// influence mesh conservation or provider selection.
pub const CODEX_QUOTA_FRESHNESS_SECS: i64 = 5 * 60;

impl SubscriptionQuota {
    pub fn new(by_provider: std::collections::HashMap<String, QuotaStatus>) -> Self {
        Self {
            by_provider,
            ..Default::default()
        }
    }

    /// Attach per-provider usage fractions (the strictest active window).
    pub fn with_fractions(mut self, fraction: std::collections::HashMap<String, f64>) -> Self {
        self.fraction = fraction;
        self
    }

    /// Attach per-provider subscription plan slugs (from `config.mesh.subscriptions`).
    /// Documented in docs/features/mesh-routing.md.
    pub fn with_plans(mut self, plans: std::collections::HashMap<String, String>) -> Self {
        self.plans = plans;
        self
    }

    /// Attach per-provider quota-pace projections (mesh-routing.md).
    pub fn with_paces(mut self, pace: std::collections::HashMap<String, QuotaPace>) -> Self {
        self.pace = pace;
        self
    }

    /// Enable/disable proactive conservation spreading (`config.mesh.subscription_conserve`).
    /// Documented in docs/features/mesh-routing.md.
    pub fn with_conserve(mut self, on: bool) -> Self {
        self.conserve = on;
        self
    }

    /// The pressure for a provider prefix (defaults to `Ok` when unknown/unconstrained).
    /// Documented in docs/features/mesh-routing.md.
    pub fn status_for(&self, provider: &str) -> QuotaStatus {
        self.by_provider
            .get(provider)
            .copied()
            .unwrap_or(QuotaStatus::Ok)
    }

    /// Fraction of the strictest window consumed for a provider (0.0 when unknown).
    pub fn fraction_for(&self, provider: &str) -> f64 {
        self.fraction.get(provider).copied().unwrap_or(0.0)
    }

    /// The pace projection for a provider, if one was attached and there was enough quota
    /// history to derive it (see [`compute_quota_pace`]).
    pub fn pace_for(&self, provider: &str) -> Option<QuotaPace> {
        self.pace.get(provider).copied()
    }

    /// The conservation input for a provider (mesh-routing.md): `fraction_for(provider)`
    /// raised to the pace's `projected_fraction_at_reset` (clamped to 1.0) when a pace is present
    /// AND projects HIGHER than the current fraction — a window projected to hit 90% by reset is
    /// treated as if it's already at 90%, so spreading ramps up ahead of the overrun instead of
    /// reacting to it only once it actually crosses a threshold. The projection can only raise,
    /// never lower, the fraction: a cooling-down window (rate has since dropped, so the projection
    /// now reads under the current fraction) still conserves on what's already spent, not on a
    /// stale lower number.
    /// Documented in docs/features/mesh-routing.md.
    pub fn effective_fraction_for(&self, provider: &str) -> f64 {
        let frac = self.fraction_for(provider);
        let projected = self
            .pace_for(provider)
            .and_then(|p| p.projected_fraction_at_reset)
            .unwrap_or(0.0)
            .min(1.0);
        frac.max(projected)
    }

    /// The configured plan slug for a provider (`""` when unset).
    pub fn plan_for(&self, provider: &str) -> &str {
        self.plans.get(provider).map(String::as_str).unwrap_or("")
    }

    /// Whether proactive conservation spreading is enabled.
    pub fn conserve_enabled(&self) -> bool {
        self.conserve
    }

    /// At/over the limit — route around it.
    pub fn is_exhausted(&self, provider: &str) -> bool {
        self.status_for(provider) == QuotaStatus::Exhausted
    }

    /// Near or over the limit — usable but demoted below alternatives.
    pub fn is_pressured(&self, provider: &str) -> bool {
        self.status_for(provider) >= QuotaStatus::Warning
    }

    pub fn is_empty(&self) -> bool {
        self.by_provider.is_empty()
    }
}

/// One historical observation of a subscription window's usage (mesh-routing.md),
/// read back from the store's append-only `quota_history` table. Distinct from [`QuotaHint`]
/// (the latest snapshot only) — a series of these is what lets [`compute_quota_pace`] derive a
/// rate of consumption instead of just a point-in-time fraction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuotaHistoryPoint {
    /// Epoch seconds this observation was recorded.
    pub observed_at: i64,
    /// Fraction of the window consumed at `observed_at` (0.0–1.0).
    pub fraction_used: f64,
}

/// A minimum span of wall-clock time between the earliest and latest history point before
/// [`compute_quota_pace`] will derive a rate from them. Two samples seconds apart (e.g. right
/// after a window resets) would otherwise divide by a near-zero denominator and spike into a
/// nonsensical rate/projection — this floor makes "not enough data yet" an explicit `None`
/// instead of a false reading.
pub const QUOTA_PACE_MIN_ELAPSED_SECS: i64 = 300;

/// A lookback window wide enough to cover a `weekly` subscription window's history without
/// pulling the whole `quota_history` table (rows are one per turn's quota hint, so this is small
/// even at 8 days). Shared by the statusline's [`compute_quota_pace`] caller (forge-core's
/// `emit_quota_pace`) and the store's [`SubscriptionQuota`] pace attachment (`quota_at`) so both
/// project off the same history window.
/// Documented in docs/features/mesh-routing.md.
pub const QUOTA_PACE_LOOKBACK_SECS: i64 = 8 * 24 * 3600;

/// A projection of where a subscription window's usage is headed, derived from a short history
/// of `(observed_at, fraction_used)` points by [`compute_quota_pace`]. Pure/deterministic: no
/// clock or I/O, so it's trivially unit-testable — the caller supplies "now".
#[derive(Debug, Clone, Copy, PartialEq)]
/// Documented in docs/features/mesh-routing.md.
pub struct QuotaPace {
    /// Fraction of the window consumed per hour, at the observed rate (>= 0.0).
    pub rate_per_hour: f64,
    /// Fraction of the window consumed per day, at the observed rate (>= 0.0).
    pub rate_per_day: f64,
    /// Fraction of the window projected to be used AT `resets_at`, linearly extrapolating the
    /// observed rate. `None` when the window's reset time isn't known (nothing to project to).
    pub projected_fraction_at_reset: Option<f64>,
    /// Seconds until the window would hit 100% at the current rate. `None` when the rate is
    /// zero/negative (usage isn't climbing, so it will never reach 100% on its own).
    pub time_to_exhaustion_secs: Option<f64>,
    /// True when the projected usage will exceed the window before it resets — i.e.
    /// `time_to_exhaustion_secs < time remaining in the window`. The signal the statusline/mesh
    /// use to warn ahead of an overrun, not just react to one already at `QuotaStatus::Warning`.
    pub exhaustion_warning: bool,
}

/// Derive a [`QuotaPace`] from a subscription window's usage history. `history` need not be
/// sorted; `resets_at` is the window's known reset time (epoch secs, if any); `now` is the
/// caller-supplied clock (epoch secs) — this function performs no I/O and reads no clock itself.
///
/// Returns `None` when there isn't enough data to derive a rate: fewer than two points, or the
/// earliest and latest point are within [`QUOTA_PACE_MIN_ELAPSED_SECS`] of each other (guards the
/// "just reset" near-zero-denominator case described on [`QUOTA_PACE_MIN_ELAPSED_SECS`]).
pub fn compute_quota_pace(
    history: &[QuotaHistoryPoint],
    resets_at: Option<i64>,
    now: i64,
) -> Option<QuotaPace> {
    if history.len() < 2 {
        return None;
    }
    let mut sorted: Vec<QuotaHistoryPoint> = history.to_vec();
    sorted.sort_by_key(|p| p.observed_at);
    let earliest = sorted.first().copied()?;
    let latest = sorted.last().copied()?;

    let elapsed_secs = latest.observed_at - earliest.observed_at;
    if elapsed_secs < QUOTA_PACE_MIN_ELAPSED_SECS {
        return None;
    }

    // A window rollover mid-history would show fraction_used dropping; clamp to 0 rather than
    // report a negative rate — there's nothing sensible to project from a reset in-range.
    let delta_fraction = (latest.fraction_used - earliest.fraction_used).max(0.0);
    let rate_per_sec = delta_fraction / elapsed_secs as f64;
    let rate_per_hour = rate_per_sec * 3600.0;
    let rate_per_day = rate_per_sec * 86_400.0;

    let time_to_exhaustion_secs = if rate_per_sec > 0.0 && latest.fraction_used < 1.0 {
        Some((1.0 - latest.fraction_used) / rate_per_sec)
    } else if latest.fraction_used >= 1.0 {
        Some(0.0)
    } else {
        None
    };

    let projected_fraction_at_reset = resets_at.map(|r| {
        let remaining = (r - now).max(0) as f64;
        latest.fraction_used + rate_per_sec * remaining
    });

    let exhaustion_warning = match (resets_at, time_to_exhaustion_secs) {
        (Some(r), Some(t)) => {
            let remaining = (r - now).max(0) as f64;
            t < remaining
        }
        _ => false,
    };

    Some(QuotaPace {
        rate_per_hour,
        rate_per_day,
        projected_fraction_at_reset,
        time_to_exhaustion_secs,
        exhaustion_warning,
    })
}

/// Why a `run_turn_with` loop ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Model produced a final answer without requesting more tools.
    FinalAnswer,
    /// Loop hit the configured `max_steps` limit while the model still wanted tools.
    MaxSteps,
    /// Daily/monthly budget cap was reached before the turn ran.
    BudgetExhausted,
    /// Turn was aborted via `forge_interrupt` (or an equivalent signal).
    Interrupted,
}

/// The result of a completed (or interrupted) agent turn.
#[derive(Debug, Clone)]
pub struct LoopOutcome {
    /// The assistant's final text response.
    pub text: String,
    /// Why the turn ended.
    pub stop_reason: StopReason,
}

impl LoopOutcome {
    pub fn final_answer(text: String) -> Self {
        Self {
            text,
            stop_reason: StopReason::FinalAnswer,
        }
    }

    pub fn max_steps(text: String) -> Self {
        Self {
            text,
            stop_reason: StopReason::MaxSteps,
        }
    }

    pub fn budget_exhausted(text: String) -> Self {
        Self {
            text,
            stop_reason: StopReason::BudgetExhausted,
        }
    }
}

impl std::ops::Deref for LoopOutcome {
    type Target = str;
    fn deref(&self) -> &str {
        &self.text
    }
}

impl std::fmt::Display for LoopOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl PartialEq for LoopOutcome {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.stop_reason == other.stop_reason
    }
}

impl PartialEq<str> for LoopOutcome {
    fn eq(&self, other: &str) -> bool {
        self.text == other
    }
}

impl PartialEq<&str> for LoopOutcome {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

impl PartialEq<String> for LoopOutcome {
    fn eq(&self, other: &String) -> bool {
        &self.text == other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_bench_blocks_every_model_alias_for_that_provider() {
        let health = ModelHealth::new([provider_bench_key("agy-cli")].into_iter().collect());
        assert!(health.is_benched("agy-cli::gemini-3.1-pro"));
        assert!(health.is_benched("agy-cli::gemini-3.5-flash"));
        assert!(!health.is_benched("codex-cli::gpt-5.6-luna"));
    }

    #[test]
    fn extract_path_arg_covers_all_known_keys() {
        use serde_json::json;
        // Every alias resolves to the path string.
        for key in PATH_ARG_KEYS {
            let args = json!({ *key: "src/main.rs" });
            assert_eq!(
                extract_path_arg(&args),
                Some("src/main.rs"),
                "key `{key}` must be recognized"
            );
        }
        // `path` wins when several keys are present (canonical, first in the list).
        let multi = json!({ "target": "b.rs", "path": "a.rs" });
        assert_eq!(extract_path_arg(&multi), Some("a.rs"));
        // No path key (e.g. a shell call) → None.
        assert_eq!(extract_path_arg(&json!({ "command": "ls" })), None);
        // Non-string path values are ignored.
        assert_eq!(extract_path_arg(&json!({ "path": 7 })), None);
    }

    #[test]
    fn task_tier_up_down_clamps_at_ends() {
        assert_eq!(TaskTier::Trivial.up(), TaskTier::Standard);
        assert_eq!(TaskTier::Standard.up(), TaskTier::Complex);
        assert_eq!(TaskTier::Complex.up(), TaskTier::Complex); // clamped
        assert_eq!(TaskTier::Complex.down(), TaskTier::Standard);
        assert_eq!(TaskTier::Standard.down(), TaskTier::Trivial);
        assert_eq!(TaskTier::Trivial.down(), TaskTier::Trivial); // clamped
        assert!(TaskTier::Complex.is_max() && !TaskTier::Complex.is_min());
        assert!(TaskTier::Trivial.is_min() && !TaskTier::Trivial.is_max());
        assert_eq!(TaskTier::from_name("Complex"), Some(TaskTier::Complex));
        assert_eq!(TaskTier::from_name(" standard "), Some(TaskTier::Standard));
        assert_eq!(TaskTier::from_name("bogus"), None);
    }

    #[test]
    fn todo_status_parses_loosely_and_defaults_to_pending() {
        assert_eq!(
            TodoStatus::parse_loose("in progress"),
            TodoStatus::InProgress
        );
        assert_eq!(
            TodoStatus::parse_loose("In-Progress"),
            TodoStatus::InProgress
        );
        assert_eq!(TodoStatus::parse_loose("DONE"), TodoStatus::Done);
        assert_eq!(TodoStatus::parse_loose("completed"), TodoStatus::Done);
        assert_eq!(TodoStatus::parse_loose("todo"), TodoStatus::Pending);
        assert_eq!(TodoStatus::parse_loose("garbage"), TodoStatus::Pending);
        assert_eq!(TodoStatus::default(), TodoStatus::Pending);
        assert_eq!(TodoStatus::Done.marker(), "☑");
    }

    #[test]
    fn usage_totals() {
        let u = Usage {
            input_tokens: 10,
            output_tokens: 5,
            cached_input_tokens: 0,
            cost_usd: 0.01,
        };
        assert_eq!(u.total_tokens(), 15);
    }

    #[test]
    fn permission_mode_default_is_safe() {
        assert_eq!(PermissionMode::default(), PermissionMode::Default);
    }

    #[test]
    fn temper_labels_name_the_permission_plainly() {
        assert_eq!(PermissionMode::Plan.label(), "Read-only");
        assert_eq!(PermissionMode::Default.label(), "Ask");
        assert_eq!(PermissionMode::AcceptEdits.label(), "Auto-edit");
        assert_eq!(PermissionMode::Bypass.label(), "Full");
    }

    #[test]
    fn temper_cycle_wraps_through_the_safe_three_and_excludes_bypass() {
        let mut m = PermissionMode::Default;
        let mut seen = Vec::new();
        for _ in 0..3 {
            seen.push(m);
            m = m.cycle_next();
        }
        assert_eq!(m, PermissionMode::Default, "cycle wraps after three");
        assert_eq!(
            seen,
            vec![
                PermissionMode::Default,
                PermissionMode::AcceptEdits,
                PermissionMode::Plan
            ]
        );
        // The dangerous temper is never produced by cycling, and cycling off it is safe.
        assert!(!seen.contains(&PermissionMode::Bypass));
        assert_eq!(PermissionMode::Bypass.cycle_next(), PermissionMode::Default);
    }

    #[test]
    fn temper_labels_deserialize_as_aliases() {
        let m: PermissionMode = serde_json::from_str("\"read-only\"").unwrap();
        assert_eq!(m, PermissionMode::Plan);
        let m: PermissionMode = serde_json::from_str("\"auto-edit\"").unwrap();
        assert_eq!(m, PermissionMode::AcceptEdits);
        let m: PermissionMode = serde_json::from_str("\"full\"").unwrap();
        assert_eq!(m, PermissionMode::Bypass);
        // Canonical keys still work.
        let m: PermissionMode = serde_json::from_str("\"accept-edits\"").unwrap();
        assert_eq!(m, PermissionMode::AcceptEdits);
    }

    #[test]
    fn ids_are_unique() {
        assert_ne!(new_id(), new_id());
    }

    fn finding(sev: Severity, conf: Confidence, cat: FindingCategory) -> Finding {
        Finding {
            id: new_id(),
            category: cat,
            severity: sev,
            confidence: conf,
            file: "x.rs".into(),
            line: None,
            title: "t".into(),
            rationale: "r".into(),
            suggested_fix: "f".into(),
            effort: Effort::Small,
            lens: cat.as_str().into(),
            verified: true,
        }
    }

    #[test]
    fn report_ranks_by_severity_then_confidence() {
        let mut report = AssayReport {
            run_id: "r".into(),
            scope: AssayScope::Repo,
            findings: vec![
                finding(Severity::Low, Confidence::High, FindingCategory::Design),
                finding(
                    Severity::Critical,
                    Confidence::Low,
                    FindingCategory::Correctness,
                ),
                finding(Severity::High, Confidence::Low, FindingCategory::Unsafe),
                finding(
                    Severity::High,
                    Confidence::High,
                    FindingCategory::DeadWeight,
                ),
            ],
            cost_usd: 0.0,
            skipped_lenses: vec![],
        };
        report.rank();
        let order: Vec<_> = report.findings.iter().map(|f| f.severity).collect();
        assert_eq!(
            order,
            vec![
                Severity::Critical,
                Severity::High,
                Severity::High,
                Severity::Low
            ]
        );
        // Within the two High findings, higher confidence ranks first.
        assert_eq!(report.findings[1].confidence, Confidence::High);
        assert_eq!(report.severity_counts(), [1, 2, 0, 1]);
    }

    #[test]
    fn mechanical_lenses_route_cheap_judgment_routes_frontier() {
        assert_eq!(FindingCategory::DeadWeight.tier(), TaskTier::Trivial);
        assert_eq!(FindingCategory::Unsafe.tier(), TaskTier::Trivial);
        assert_eq!(FindingCategory::Architecture.tier(), TaskTier::Complex);
        assert_eq!(FindingCategory::Correctness.tier(), TaskTier::Complex);
    }

    #[test]
    fn severity_and_category_parse_round_trip() {
        for s in [
            Severity::Critical,
            Severity::High,
            Severity::Medium,
            Severity::Low,
        ] {
            assert_eq!(Severity::parse(s.as_str()), Some(s));
        }
        for c in FindingCategory::crew() {
            assert_eq!(FindingCategory::parse(c.as_str()), Some(*c));
        }
    }

    #[test]
    fn permission_mode_key_round_trips() {
        // The bridge hands the temper across a process boundary as this key, so the round-trip must
        // be exact (no fuzzy aliases) for every variant.
        for m in PermissionMode::all() {
            assert_eq!(
                PermissionMode::from_key(m.key()),
                Some(*m),
                "round-trip {m:?}"
            );
        }
        assert_eq!(
            PermissionMode::from_key("accept-edits"),
            Some(PermissionMode::AcceptEdits)
        );
        assert_eq!(PermissionMode::from_key("nope"), None);
    }

    fn hp(observed_at: i64, fraction_used: f64) -> QuotaHistoryPoint {
        QuotaHistoryPoint {
            observed_at,
            fraction_used,
        }
    }

    #[test]
    fn quota_pace_normal_rate_projects_comfortably_under_limit_no_warning() {
        // 10% used, climbing to 20% over 5 hours -> 2%/hr. A 5-hour window with 2 hours left to
        // run projects to 20% + 2%*2 = 24% at reset: comfortably under 100%, no warning.
        let now = 10_000_i64;
        let history = vec![hp(now - 5 * 3600, 0.10), hp(now, 0.20)];
        let resets_at = now + 2 * 3600;
        let pace = compute_quota_pace(&history, Some(resets_at), now).expect("enough data");

        assert!((pace.rate_per_hour - 0.02).abs() < 1e-9, "{pace:?}");
        let projected = pace.projected_fraction_at_reset.expect("has reset time");
        assert!((projected - 0.24).abs() < 1e-9, "{pace:?}");
        assert!(!pace.exhaustion_warning, "{pace:?}");
    }

    #[test]
    fn quota_pace_over_pace_warns_with_sane_projection_and_ttl() {
        // 20% used, climbing to 80% over 1 hour -> 60%/hr. 2 hours left in the window: at that
        // rate usage would hit 100% well before reset, so the warning must fire, and the specific
        // projected percentage / time-to-exhaustion must be sane (not just "warning is true").
        let now = 20_000_i64;
        let history = vec![hp(now - 3600, 0.20), hp(now, 0.80)];
        let resets_at = now + 2 * 3600;
        let pace = compute_quota_pace(&history, Some(resets_at), now).expect("enough data");

        assert!((pace.rate_per_hour - 0.60).abs() < 1e-9, "{pace:?}");
        // Projected at reset: 80% + 60%/hr * 2hr = 200%.
        let projected = pace.projected_fraction_at_reset.expect("has reset time");
        assert!((projected - 2.0).abs() < 1e-9, "{pace:?}");
        // Time to 100%: (1.0 - 0.8) / (0.60/3600) = 1200s = 20 minutes.
        let ttl = pace.time_to_exhaustion_secs.expect("rate is positive");
        assert!((ttl - 1200.0).abs() < 1e-6, "{pace:?}");
        assert!(pace.exhaustion_warning, "{pace:?}");
    }

    #[test]
    fn quota_pace_just_reset_guards_against_near_zero_denominator() {
        // Two samples 2 seconds apart, both near 0% used (right after a window rollover). Must
        // NOT compute a rate off a near-zero elapsed time and must NOT report a false warning —
        // it must report "not enough data yet" instead.
        let now = 30_000_i64;
        let history = vec![hp(now - 2, 0.001), hp(now, 0.0011)];
        let pace = compute_quota_pace(&history, Some(now + 5 * 3600), now);
        assert!(
            pace.is_none(),
            "expected not-enough-data guard, got {pace:?}"
        );
    }

    #[test]
    fn quota_pace_needs_at_least_two_points() {
        assert!(compute_quota_pace(&[], None, 0).is_none());
        assert!(compute_quota_pace(&[hp(0, 0.1)], None, 100_000).is_none());
    }

    #[test]
    fn quota_pace_without_reset_time_still_reports_rate_but_no_projection_or_warning() {
        let now = 40_000_i64;
        let history = vec![hp(now - 3600, 0.10), hp(now, 0.20)];
        let pace = compute_quota_pace(&history, None, now).expect("enough data");
        assert!((pace.rate_per_hour - 0.10).abs() < 1e-9, "{pace:?}");
        assert!(pace.projected_fraction_at_reset.is_none());
        assert!(!pace.exhaustion_warning);
    }

    fn pace(projected_fraction_at_reset: Option<f64>) -> QuotaPace {
        QuotaPace {
            rate_per_hour: 0.0,
            rate_per_day: 0.0,
            projected_fraction_at_reset,
            time_to_exhaustion_secs: None,
            exhaustion_warning: false,
        }
    }

    #[test]
    fn effective_fraction_without_pace_is_the_plain_fraction() {
        let mut fr = std::collections::HashMap::new();
        fr.insert("claude-cli".to_string(), 0.42);
        let q = SubscriptionQuota::new(std::collections::HashMap::new()).with_fractions(fr);
        assert!((q.effective_fraction_for("claude-cli") - 0.42).abs() < 1e-9);
    }

    #[test]
    fn effective_fraction_uses_projection_when_it_is_higher() {
        let mut fr = std::collections::HashMap::new();
        fr.insert("claude-cli".to_string(), 0.2);
        let mut pc = std::collections::HashMap::new();
        pc.insert("claude-cli".to_string(), pace(Some(1.3)));
        let q = SubscriptionQuota::new(std::collections::HashMap::new())
            .with_fractions(fr)
            .with_paces(pc);
        // Projection over 100% clamps to 1.0.
        assert!((q.effective_fraction_for("claude-cli") - 1.0).abs() < 1e-9);
    }

    #[test]
    fn effective_fraction_keeps_current_when_projection_is_lower() {
        let mut fr = std::collections::HashMap::new();
        fr.insert("claude-cli".to_string(), 0.6);
        let mut pc = std::collections::HashMap::new();
        pc.insert("claude-cli".to_string(), pace(Some(0.3)));
        let q = SubscriptionQuota::new(std::collections::HashMap::new())
            .with_fractions(fr)
            .with_paces(pc);
        assert!(
            (q.effective_fraction_for("claude-cli") - 0.6).abs() < 1e-9,
            "a cooling-down projection must not lower the fraction below what's already spent"
        );
    }

    #[test]
    fn effective_fraction_with_no_projection_falls_back_to_plain_fraction() {
        // A pace can exist (enough history) but have no resets_at, so no projection.
        let mut fr = std::collections::HashMap::new();
        fr.insert("claude-cli".to_string(), 0.35);
        let mut pc = std::collections::HashMap::new();
        pc.insert("claude-cli".to_string(), pace(None));
        let q = SubscriptionQuota::new(std::collections::HashMap::new())
            .with_fractions(fr)
            .with_paces(pc);
        assert!((q.effective_fraction_for("claude-cli") - 0.35).abs() < 1e-9);
    }
}
