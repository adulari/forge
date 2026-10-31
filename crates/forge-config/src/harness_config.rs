//! `[harness]` settings: Continual Harness plus the loop-behaviour flags.

use serde::{Deserialize, Serialize};

/// Continual Harness (`/refine`, port of prime-agent's `/refine`): the agent proposes durable
/// edits to its own prompts/skills/subagents, persisted via forge-store's `harness_entry` /
/// `harness_refinement` tables. `enabled` gates both context injection and the ability to apply
/// edits; `auto_refine` separately gates whether a refinement pass ever runs unprompted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessConfig {
    #[serde(default = "crate::default_true")]
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
    /// Per-model-family defaults for the flags above, a prompt delta and reasoning handling
    /// ([`crate::model_family`]; docs/features/harness-profile.md). Default off while an A/B round
    /// decides. A flag set in a config file or `FORGE_HARNESS__*` always wins over a family default.
    #[serde(default)]
    pub family_profiles: bool,
    /// `[harness]` flag names set explicitly by the user (filled at load, never written back).
    #[serde(skip)]
    pub explicit_flags: Vec<String>,
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
            family_profiles: false,
            explicit_flags: Vec::new(),
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
