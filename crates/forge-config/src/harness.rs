//! `[harness]` settings: Continual Harness and the opt-in loop/stop behaviours.

use serde::{Deserialize, Serialize};

use crate::default_true;

/// Continual Harness (`/refine`, port of prime-agent's `/refine`): the agent proposes durable
/// edits to its own prompts/skills/subagents, persisted via forge-store's `harness_entry` /
/// `harness_refinement` tables. `enabled` gates both context injection and the ability to apply
/// edits; `auto_refine` separately gates whether a refinement pass ever runs unprompted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub auto_refine: AutoRefineMode,
    /// Turn interval for `auto_refine = "turns"`.
    #[serde(default = "default_harness_auto_refine_turns")]
    pub auto_refine_turns: u32,
    /// Max harness entries injected into a turn's context.
    #[serde(default = "default_harness_max_context_entries")]
    pub max_context_entries: u32,
    /// Max characters per injected harness entry; longer entries are truncated.
    #[serde(default = "default_harness_max_entry_chars")]
    pub max_entry_chars: u32,
    /// Loop/stop behaviours borrowed from oh-my-pi and Factory Droid, each default off so a
    /// benchmark round can A/B them. A flag that is on wins over `[mesh] harness_profile =
    /// "minimal"` (docs/features/harness-profile.md).
    ///
    /// A short completion contract at the end of the system prompt.
    #[serde(default)]
    pub completion_contract: bool,
    /// Up to 3 corrective continuations after an announced-but-undone final reply, and up to 3
    /// retries of an empty or text-only tool-call reply before benching; a pinned model is never
    /// benched for a single empty reply.
    #[serde(default)]
    pub stop_retries: bool,
    /// Inject "act, don't re-plan" when consecutive tool-free replies keep restating a plan.
    #[serde(default)]
    pub thinking_loop_breaker: bool,
    /// Interrupt naming the tool and arguments after 5 identical calls with identical results,
    /// and move the progress ledger's cycle stop two repeats later.
    #[serde(default)]
    pub repeat_interrupt: bool,
    /// Closest-match / occurrence-list text when `edit_file`/`multi_edit` cannot place `old`.
    /// Text-only improvement, on by default.
    #[serde(default = "default_true")]
    pub edit_errors: bool,
    /// Schema-directed coercion of tool arguments (string to array/bool/number, lone value to
    /// array), and the tool's compact schema in a bad-arguments error. On by default.
    #[serde(default = "default_true")]
    pub arg_coercion: bool,
    /// `read_file`/`search` shaping: `path:50-100` ranges, declarations-only outline for bare
    /// reads of code files of 100+ lines, 300-line default page, 768-byte line cap, `search`
    /// paged at 20 files with context. Off by default.
    #[serde(default)]
    pub read_shaping: bool,
    /// `run_code`: concurrent inner calls under `Promise.all`, `background: true` shell jobs, the
    /// saved-output path in the footer, a higher inner-result cap. Off by default.
    #[serde(default)]
    pub script_parallel: bool,
    /// A foreground shell command still running after 60 s moves to the background and the tool
    /// returns the job id and output so far; `job_wake` delivers completion. Off by default.
    #[serde(default)]
    pub shell_autobackground: bool,
    /// Factory Droid's stream-silence watchdog: a model stream silent for 4 minutes (the idle
    /// window is raised to 240s when it is shorter) is aborted and the SAME model is retried,
    /// instead of failing over at once. Streams that deliver any event, reasoning included, are
    /// never cut.
    #[serde(default)]
    pub stream_silence_retry: bool,
    /// Advertise a `wait` action on `shell_job`: block up to N seconds for a background job and
    /// return its exit code and log tail, instead of polling.
    #[serde(default)]
    pub wait_job: bool,
    /// Keep the cached request prefix append-only: the task-list snapshot leaves the pinned
    /// context block (which sits before the whole transcript) and is appended to the transcript
    /// when it changes.
    #[serde(default)]
    pub append_only_prefix: bool,
    /// Warn at once, naming the cause, when a request breaks the provider prompt cache (tool list
    /// changed, or an already-sent message was rewritten) instead of after two in a row.
    #[serde(default)]
    pub cache_monitor: bool,
    /// Cap a compaction summary at 16k tokens (Factory Droid's budget) instead of 20k.
    #[serde(default)]
    pub compaction_summary_cap: bool,
}

impl Default for HarnessConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_refine: AutoRefineMode::default(),
            auto_refine_turns: default_harness_auto_refine_turns(),
            max_context_entries: default_harness_max_context_entries(),
            max_entry_chars: default_harness_max_entry_chars(),
            completion_contract: false,
            stop_retries: false,
            thinking_loop_breaker: false,
            repeat_interrupt: false,
            edit_errors: true,
            arg_coercion: true,
            read_shaping: false,
            script_parallel: false,
            shell_autobackground: false,
            stream_silence_retry: false,
            wait_job: false,
            append_only_prefix: false,
            cache_monitor: false,
            compaction_summary_cap: false,
        }
    }
}

/// When a refinement pass auto-triggers, if ever (ADR: Continual Harness).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AutoRefineMode {
    /// Refinement only runs when explicitly requested (`/refine`).
    #[default]
    Off,
    /// Auto-trigger a refinement pass after every `/compact`.
    Compact,
    /// Auto-trigger a refinement pass every `auto_refine_turns` turns.
    Turns,
}

fn default_harness_auto_refine_turns() -> u32 {
    20
}

fn default_harness_max_context_entries() -> u32 {
    12
}

fn default_harness_max_entry_chars() -> u32 {
    2000
}
