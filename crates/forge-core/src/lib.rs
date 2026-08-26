//! The session orchestrator: it runs the agent loop (the walking skeleton's spine) and
//! owns the permission broker — the one component that must be central (ADR-0002). It
//! wires the Mesh (routing), a Provider (model calls), the tool registry, the store
//! (persistence) and a presenter (UI) together, depending on each only through its trait.

use std::sync::Arc;

pub(crate) use completeness::*;
use completion::{
    CompletionContract, CompletionDecision, CompletionEvidence, VerificationLedger,
    VerificationObservation,
};
use forge_config::Config;
use forge_index::Lattice;
use forge_mesh::pricing::Pricing;
use forge_mesh::{
    BudgetState, BudgetStatus, HeuristicRouter, ModelCatalog, RouteHints, Router, RoutingContext,
    SessionAffinity,
};
use forge_provider::{CompletionOptions, Provider, StreamEvent, ToolSpec};
use forge_store::{MeshOutcome, Store};
use forge_tools::ToolRegistry;
use forge_types::{
    EffortLevel, LoopOutcome, Message, ModelHealth, PermissionDecision, PermissionMode,
    PermissionRule, ProjectContext, Role, StopReason, SubscriptionQuota, TaskTier,
};
use forge_types::{Presenter, PresenterEvent};

pub mod assay;
mod auxiliary_policy;
mod btw_policy;
pub mod capsule;
mod compaction_policy;
mod completeness;
pub(crate) mod completion;
pub mod context_pack;
pub(crate) mod context_pipeline;
mod detached_subagents;
pub mod duel;
pub mod fleet;
pub mod heartbeat;
pub mod hooks;
pub mod llm_router;
mod model_loop;
mod model_request;
mod model_response;
mod model_stream;
mod model_tools;
mod orchestration;
pub mod permission;
pub mod project_context;
mod quality_gates;
pub mod readiness;
mod refinement;
mod replay;
mod routing_policy;
mod session_controls;
mod session_history;
mod session_lifecycle;
mod session_virtual_tools;
pub mod snapshot;
pub mod subagent;
mod text_policy;
pub mod tokens;
mod tool_dispatch;
pub mod turn_contract;
pub mod workflow;
mod workspace_context;
pub mod worktree;

pub use llm_router::LlmRouter;
use replay::messages_to_replay_items;
use session_virtual_tools::ASK_USER_TOOL;
pub use session_virtual_tools::{remember_spec, REMEMBER_TOOL};
use text_policy::{completed_tasks_recap, memory_scope_at, recap_line, sanitize_suggestion};
pub use workspace_context::WorkspaceContext;

pub const AUTO_COMPACT_THRESHOLD: f64 = 0.80;

pub fn auto_compact_trigger_tokens(window: u64, cap: u64, fraction: f64) -> u64 {
    let frac = (window as f64 * fraction).max(0.0) as u64;
    frac.min(cap)
}

fn should_reuse_response_chain(
    prompt: &str,
    routing_context: &RoutingContext,
    affinity: Option<&SessionAffinity>,
    routed_model: &str,
) -> bool {
    routing_context.is_dependent_turn(prompt)
        && affinity.is_some_and(|affinity| affinity.model == routed_model)
}

fn completion_prefix_tokens(messages: &[Message], tools: &[ToolSpec]) -> u64 {
    let message_tokens = messages.iter().map(message_tokens).sum::<usize>();
    let tool_tokens = tools
        .iter()
        .map(|spec| {
            tokens::count_text(&spec.name)
                + tokens::count_text(&spec.description)
                + tokens::count_text(&spec.schema.to_string())
                + 8
        })
        .sum::<usize>();
    message_tokens.saturating_add(tool_tokens) as u64
}

/// Compaction (`/compact`): keep this many of the most recent messages verbatim; summarize the
/// rest. Only compact when there are at least `COMPACT_MIN_OLDER` older messages to fold.
pub(crate) const COMPACT_KEEP_RECENT: usize = 6;
pub(crate) const COMPACT_MIN_OLDER: usize = 4;
const COMPACT_SYSTEM: &str = "You are compacting a coding-assistant conversation to save context. \
Summarize the messages below concisely but preserve: decisions made, key facts, file paths, \
function/type names, and any open threads or TODOs. Output only the summary.";

const SHELL_DIAGNOSE_SYSTEM: &str = "A shell command run by a coding agent just failed. \
Respond with exactly one or two lines:\n\
Line 1: the most likely cause in one terse sentence (no preamble, no restating the command).\n\
Line 2 (optional): if a single shell command fixes it, write exactly: FIX: <the command>. \
Omit line 2 if no single command fixes it.";

/// Shell diagnosis is optional context, never part of the user's requested work. Bound both its
/// silent-stream interval and total wall time so an unhealthy auxiliary provider cannot hold the
/// primary model loop (and therefore the entire TUI) hostage.
// This side-call is optional and the main model already receives the original failing output.
// A slow auxiliary must not add a provider-sized latency bubble to the critical turn path.
const SHELL_DIAGNOSE_MAX_SECS: u64 = 12;

/// Default sampling temperature for coding turns: low, so edits/patches are deterministic rather
/// than creatively varied. Only takes effect when reasoning/effort isn't engaged (thinking models
/// reject a custom temperature) — see `genai_provider`.
const CODING_TEMPERATURE: f32 = 0.1;

/// The base coding-agent system prompt, prepended (fresh, never persisted) to every main-loop
/// request so a model performs in Forge the way it does in a purpose-built harness. Kept tight: it
/// establishes role + tool discipline + editing conventions without burning context. Project-level
/// `AGENTS.md` and skill guidance layer on top of this as separate (persisted) system messages.
const FORGE_SYSTEM: &str = "\
You are Forge, an expert software engineering agent operating in a user's terminal on their \
codebase. You complete the user's coding task end-to-end by reading code and editing files with the \
tools provided, then stop.

Approach:
- Work from evidence, not assumption. Before editing, read the relevant files and search the \
codebase so your change fits the existing structure, naming, and conventions.
- Keep verification exit codes trustworthy: run checks separately or join them with `&&`; do not \
mask failures behind `;`, `||`, or a pipeline into `head`/`tail`/`tee` without pipe-failure handling.
- For any non-trivial task, make a short plan and keep it current with the update_tasks tool. \
Plan bookkeeping is non-blocking: NEVER call update_tasks by itself when an independent read, edit, \
or check can advance the work. Request both in the same response. A standalone update is only for \
when no substantive next action exists, such as immediately before the final answer. Do the work; \
don't just describe it.
- Make the smallest change that fully solves the task. Match the surrounding code's style. Do NOT \
add comments unless the code's intent is genuinely non-obvious. Don't reformat unrelated code.
- Solve the general case, not just the tests or examples in front of you — don't hardcode to \
specific inputs. If a test or the task itself looks wrong or infeasible, say so rather than routing \
around it.
- After editing, verify: run focused checks after the relevant change and one final complete \
build/test/lint pass when available. Reuse still-current successful evidence; do not rerun an \
unchanged check or print verbose passing output merely for reassurance. Fix failures before \
reporting done.

Tools:
- Prefer read_file / search / list_dir / glob over shelling out to cat / grep / ls / find.
- When you need several independent reads or searches, request them together in one step.
- edit_file replaces ONE exact, unique occurrence — include enough surrounding context in `old` to \
match exactly once, and read the file first so whitespace matches. To change one file in several \
places at once, multi_edit applies a list of edits atomically. For a large or multi-file change, \
apply_patch takes a unified diff. For a Jupyter notebook (.ipynb) use notebook_edit (cell-level) \
— edit_file would corrupt its JSON. Use write_file for new files or full rewrites. When generated \
content is too large for one reliable tool call, write the first coherent chunk and use append_file \
for the rest. Keep source files syntactically balanced after every write; after a truncated-edit \
rejection, do not retry the same edit shape—switch immediately to a smaller complete edit, \
append_file, or a complete write_file. Don't blind-overwrite a file you haven't read.
- A tool result starting with `error:` means it failed — read the message, fix the cause, and \
retry differently rather than repeating the same call.

Communication:
- Be concise and direct. No filler, no flattery, no restating the question. Reference code as \
`path:line`.
- Report outcomes truthfully: if a test failed, verification was skipped, or something is \
uncertain, say so plainly instead of reporting success.
- When the task is done, stop and give a short summary of what changed. Don't ask whether to \
proceed on work you can just do.";

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error(transparent)]
    Provider(#[from] forge_provider::ProviderError),
    #[error(transparent)]
    Store(#[from] forge_store::StoreError),
    #[error(transparent)]
    Lattice(#[from] forge_index::LatticeError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("invalid session workspace: {0}")]
    Workspace(String),
    /// Failover walked the whole routed/fallback chain (and the last-resort model) without finding
    /// anything usable. Carries the LAST provider error: the generic "everything is rate-limited"
    /// story is wrong (and actively misleading) when the real cause was an expired credential or a
    /// permanent capability failure, and the provider's message is the actionable part —
    /// e.g. "ChatGPT OAuth token rejected (401) — run `forge auth codex-oauth` to sign in again".
    #[error(
        "no healthy model available — every routed/fallback model is rate-limited or down; \
             last error from {model} ({reason}): {last_error}"
    )]
    NoHealthyModel {
        model: String,
        reason: &'static str,
        last_error: String,
    },
    /// The auto-review gate found findings at/above the configured severity and `gate_mode =
    /// "block"` is set — the turn is aborted so the model can fix them before proceeding.
    #[error("auto-review gate blocked: {0}")]
    TurnBlocked(String),
    /// An internal invariant was violated on a path that "can't happen". Surfaced as a clean error
    /// instead of a `panic!`/`.expect()` so a logic/config drift fails the turn loudly rather than
    /// aborting the whole process mid-turn.
    #[error("internal invariant violated: {0}")]
    Internal(String),
}

/// Result of a [`Session::rewind_to`] / [`Session::undo`]: what the file-restore did, plus the
/// prompt that began the rewound-to turn (the UI re-offers it in the input box).
#[derive(Debug, Default, Clone)]
pub struct RewindOutcome {
    pub restore: snapshot::RestoreReport,
    pub rewound_prompt: Option<String>,
}

/// Best-effort single-text embedding via the configured embedder, for semantic memory capture +
/// recall. `None` when no embedder is available or it errors → callers fall back to keyword recall.
/// A FREE function taking `&EmbeddingsConfig` (which is `Sync`) — NOT a `&self` method — so the
/// `.await` doesn't hold a `&Session` borrow (`Session` is `!Sync`, which would make the turn future
/// non-`Send`).
pub async fn embed_one(cfg: &forge_config::EmbeddingsConfig, text: &str) -> Option<Vec<f32>> {
    let (embedder, _) = forge_provider::select_embedder(cfg)?;
    embedder
        .embed(&[text.to_string()])
        .await
        .ok()
        .and_then(|mut v| v.drain(..).next())
        .filter(|e| !e.is_empty())
}

/// Max same-model retries for a TRANSIENT provider failure (5xx / dropped stream / network blip)
/// before benching the model and failing over. Small + backed off so a genuinely-down model still
/// reaches failover quickly, but a one-off blip doesn't needlessly switch models.
const MAX_TRANSIENT_RETRIES: u32 = 2;

/// Max times per turn Forge will WAIT for a rate-limited model to reset and retry it (rather than
/// failing over to a lower-ranked model). Bounds total in-turn blocking. The per-wait length cap is
/// `mesh.rate_limit_wait_secs` (0 disables waiting).
const MAX_RATE_LIMIT_WAITS: u32 = 2;

/// Absolute floor on the context window (tokens) a mesh-routed model must have for an agentic
/// coding turn. A fresh session's transcript is ~empty, but the agent still needs room for the
/// system preamble, tool schemas, a file read or two and the reply — so even at zero transcript we
/// never route to a tiny-window model. Sits in the wide gap between toy models (≤16k: allam-2-7b,
/// gemma-2-2b, …) and real frontier coders (all ≥128k), so any value here filters the former and
/// keeps the latter — mesh auto-rotation stays fully enabled, it just never lands on a window that
/// can't hold the work (which would otherwise trip the "too small, compact?" prompt every turn).
const MIN_CODING_CONTEXT: u32 = 32_000;

/// Reply room used only for context fitting when the provider output is intentionally unbounded.
///
/// `mesh.max_output_tokens = 0` means Forge omits the provider request cap and lets the model use
/// its native maximum. Context planning still needs a realistic amount of reply room, otherwise a
/// nearly-full transcript can be admitted with only the historical 1K-token cushion. This value is
/// never sent to a provider and therefore cannot truncate a response.
const UNBOUNDED_OUTPUT_PLANNING_RESERVE: u32 = 8_192;
const MIN_OUTPUT_PLANNING_RESERVE: u32 = 1_024;

fn output_planning_reserve_tokens(configured_cap: u32) -> u32 {
    if configured_cap == 0 {
        UNBOUNDED_OUTPUT_PLANNING_RESERVE
    } else {
        configured_cap.max(MIN_OUTPUT_PLANNING_RESERVE)
    }
}

/// Minimum context window the router must require for the next turn. Two terms, max-combined:
/// 1. The current transcript must clear `Session::transcript_fits`' bar (transcript ≤ 80% of the
///    post-reply room), which inverts to `window ≥ transcript·5/4 + output_reserve`. Requiring at
///    least this stops the router from admitting a model that `admit_failover_model` would instantly
///    reject — the disagreement that made the mesh churn a consent prompt on every small-window pick.
/// 2. [`MIN_CODING_CONTEXT`], so a near-empty transcript still demands real working room.
///
/// Pure so the gating math is unit-testable without a live `Session`.
fn routing_min_context_tokens(transcript_tokens: u32, output_reserve: u32) -> u32 {
    let for_transcript = transcript_tokens.saturating_mul(5) / 4;
    for_transcript
        .saturating_add(output_reserve)
        .max(MIN_CODING_CONTEXT)
}

// --- Pinned rate-limit backoff (harness-robustness wave 2, fix 1) ------------------------------
// When the model was EXPLICITLY pinned (`--model` / `/model`), a rate limit must not fail the turn
// and must not switch models (a pin must pin — the SWE-bench baseline lost 4 instances to
// "skipped: rate limited" with zero retry). Instead the SAME model is retried on this schedule.
// Provider-level multi-credential rotation runs FIRST: on a 429 the genai provider already retries
// once with the next configured API key (genai_provider.rs KeyPool), and the OAuth provider
// (xai_oauth.rs OAuthAccountPool) retries once with the next stored account — before the error
// ever reaches this loop. Waiting only starts once every key/account is limited.

/// Max same-model retry attempts for a rate-limited pinned model before failing the turn.
const PINNED_RL_MAX_ATTEMPTS: u32 = 6;
/// First backoff delay (seconds); grows ×[`PINNED_RL_GROWTH`] per attempt: 5s, 15s, 45s, then
/// capped — 5·3ᵏ⁻¹ up to [`PINNED_RL_DELAY_CAP_SECS`].
const PINNED_RL_BASE_SECS: u64 = 5;
/// Exponential growth factor between attempts.
const PINNED_RL_GROWTH: u64 = 3;
/// Per-attempt delay cap (seconds): attempts 4-6 wait at most this long.
const PINNED_RL_DELAY_CAP_SECS: u64 = 60;
/// Total in-turn wait budget (seconds) across all pinned-backoff attempts (~3 min). A schedule
/// or `Retry-After` that would exceed the remaining budget fails the turn with the real error
/// instead of blocking indefinitely.
const PINNED_RL_TOTAL_WAIT_SECS: u64 = 180;

/// One pinned-backoff delay. `attempt` is 1-based. A server `Retry-After` (when the provider
/// error carried one) is respected verbatim — the server knows its own reset better than our
/// blind schedule. Otherwise: exponential base delay with ±20% jitter (`jitter` ∈ [0,1] maps to
/// a 0.8-1.2 factor) so many pinned turns limited at once don't retry in lockstep.
fn pinned_backoff_delay(
    attempt: u32,
    retry_after: Option<std::time::Duration>,
    jitter: f64,
) -> std::time::Duration {
    if let Some(ra) = retry_after {
        return ra;
    }
    let base = PINNED_RL_BASE_SECS
        .saturating_mul(PINNED_RL_GROWTH.saturating_pow(attempt.saturating_sub(1)))
        .min(PINNED_RL_DELAY_CAP_SECS);
    let factor = 0.8 + 0.4 * jitter.clamp(0.0, 1.0);
    std::time::Duration::from_secs_f64(base as f64 * factor)
}

/// What the failover machinery may do with a retryable provider error, given pin state
/// (harness-robustness wave 2, fix 2 — strict pin semantics). Pure so the policy is
/// table-testable; [`failover_policy`] is the single chooser `run_model_loop` obeys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailoverPolicy {
    /// Not pinned (or the `mesh.pin_failover` escape hatch is on): normal cross-model
    /// failover down the routed chain.
    SwitchModels,
    /// Pinned + rate-limited, OR pinned + a transient outage that survived the same-model hot
    /// retries: wait it out and retry the SAME model (fix 1's rate-limit backoff; the
    /// pinned-outage-resilience extension covers the outage case with its own budget).
    BackoffSameModel,
    /// Pinned + a PERMANENT error (capability/auth), or a transient outage with
    /// `mesh.pin_outage_wait_secs = 0` (outage backoff disabled): fail the turn with the REAL
    /// error — an explicitly pinned model is never silently switched.
    FailTurn,
}

/// The strict-pin failover chooser: an explicit pin forbids cross-model switching unless
/// `mesh.pin_failover = true` restores the old behaviour. `transient_outage` is true for a
/// retryable, non-permanent, non-rate-limited error (`Unavailable`, typically) once the hot
/// same-model transient retries (`MAX_TRANSIENT_RETRIES`, above) are exhausted, AND
/// `mesh.pin_outage_wait_secs > 0` — the caller folds the config gate into this bool so `0`
/// restores the old FailTurn behaviour without a separate branch here.
/// Documented in docs/features/mesh-routing.md.
fn failover_policy(
    pinned: bool,
    pin_failover: bool,
    rate_limited: bool,
    transient_outage: bool,
) -> FailoverPolicy {
    if !pinned || pin_failover {
        FailoverPolicy::SwitchModels
    } else if rate_limited || transient_outage {
        FailoverPolicy::BackoffSameModel
    } else {
        FailoverPolicy::FailTurn
    }
}

/// The one-shot empty-diff completion nudge (harness-robustness wave 2, fix 4): sent as a
/// synthetic user message when a headless code-change turn ends having changed nothing.
const EMPTY_DIFF_NUDGE: &str =
    "You have not modified any files. Implement the fix now — do not just describe it.";

/// The env-fight nudge (quality guards wave 4, fix 4): injected once per turn after the
/// environment-setup spend cap is reached following a provisioning failure.
const ENV_FIGHT_NUDGE: &str = "Environment setup/build verification hit its spend cap after a \
provisioning failure. Stop installing dependencies, creating environments, or building native \
extensions. Keep the code change, use compile/static/diff evidence that does not require more \
provisioning, and finish; unavailable local dependencies are not a product-code failure.";

/// Environment-setup attempts allowed in a turn before the cap fires, once any attempt failed.
/// One alternate recovery attempt remains available; a third provisioning/native-build command is
/// then blocked deterministically by [`Session::invoke_tool`].
const ENV_FIGHT_ATTEMPT_THRESHOLD: usize = 2;

const ENV_FIGHT_BLOCKED_RESULT: &str = "shell: blocked by environment setup/build spend cap; use \
compile/static/diff evidence without installing dependencies or building native extensions";

/// Repeated build/provision tool invocations within ONE bridge turn before the ceiling folds it
/// into an early terminate (wave 5, fix 2). A CLI bridge runs its tools in a subprocess, so the
/// sink surfaces each tool START but not per-command success/failure — we can't build the
/// consecutive-failure streak the direct-path [`EnvFightTracker`] keys on. This approximates it:
/// a bridge turn that keeps re-issuing build/provision commands this many times is stuck in the
/// same venv/C-extension archaeology the env-fight guard targets, so it's folded into the
/// token-ceiling early-terminate. Higher than the direct threshold because it counts invocations,
/// not failures (some of these commands legitimately succeed).
const BRIDGE_BUILD_FIGHT_THRESHOLD: u64 = 8;

/// Whether a single bridge turn's accumulated input tokens have crossed its ceiling (wave 5,
/// fix 1). Pure so the trip logic is unit-testable. A tail-cost backstop, not a target: `cap == 0`
/// disables it, and the check is `>=` so the turn stops at the first observation boundary at or
/// past the cap.
const fn bridge_turn_over_budget(accumulated_input: u64, cap: u64) -> bool {
    cap != 0 && accumulated_input >= cap
}

/// Best-effort extraction of a shell command from a bridge tool's serialized args (wave 5, fix 2).
/// Bridge tools surface args as a String that is either the raw command (codex `command_execution`)
/// or a JSON blob carrying a `command`/`cmd` field (claude `Bash`, Forge's `shell` over MCP). Falls
/// back to the raw string so the env/build heuristic still sees phrase patterns embedded in JSON.
fn bridge_tool_command(args: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(args) {
        for key in ["command", "cmd"] {
            if let Some(c) = v.get(key).and_then(|x| x.as_str()) {
                return c.to_string();
            }
        }
    }
    args.to_string()
}

/// Whether a shell command looks like environment provisioning or a native build — the heuristic
/// the env-fight cap keys on (pip/venv/virtualenv/ensurepip/apt/uv/conda…), extended in wave 5 with
/// build archaeology (C-extension builds + native toolchains: `setup.py build_ext`, `make`, `gcc`,
/// `cmake`, `pyenv`, `./configure`…) that were the bulk of astropy-12907's failing commands and
/// matched nothing before. Phrase patterns use a substring match on the whitespace-normalized,
/// lowercased command (so wrappers like `cd x && pip install …` still match); single-token compiler
/// invocations are matched as WHOLE tokens so `make` doesn't fire on `cmake`/`makemigrations`.
fn is_env_setup_command(cmd: &str) -> bool {
    let c = cmd
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    const PHRASES: &[&str] = &[
        "pip install",
        "pip3 install",
        "-m pip",
        "-m ensurepip",
        "-m venv",
        "virtualenv",
        "uv venv",
        "uv pip",
        "apt-get install",
        "apt install",
        "conda install",
        "conda create",
        "yum install",
        "apk add",
        "setup.py build",
        "python setup.py",
        "./configure",
        "cmake",
        "pyenv",
    ];
    if PHRASES.iter().any(|p| c.contains(p)) {
        return true;
    }
    // Single-token compiler/build invocations — matched as WHOLE tokens, not substrings, so
    // `make` doesn't fire on `cmake`/`makemigrations` and `cc` doesn't fire on `gcc`/`accept`.
    const TOOLS: &[&str] = &["make", "gcc", "g++", "cc", "clang", "meson", "ninja"];
    c.split(' ').any(|tok| TOOLS.contains(&tok))
}

/// Per-turn setup-attempt tracker + latch for the environment spend cap. Only env-setup commands
/// feed it. Successful attempts alone never arm the cap; once any attempt fails, the threshold-th
/// setup attempt emits the nudge and subsequent setup/build commands are blocked.
#[derive(Default)]
struct EnvFightTracker {
    attempts: usize,
    saw_failure: bool,
    nudged: bool,
}

impl EnvFightTracker {
    fn observe(&mut self, failed: bool) -> bool {
        self.attempts += 1;
        self.saw_failure |= failed;
        if self.saw_failure && self.attempts >= ENV_FIGHT_ATTEMPT_THRESHOLD && !self.nudged {
            self.nudged = true;
            return true;
        }
        false
    }

    fn should_block(&self) -> bool {
        self.nudged
    }
}

/// Minimal-diff bias (quality guards wave 4, fix 3): appended to the system context of every
/// `expect_code_change` turn. The seaborn-2848 forensic: the model chose a plausible-but-wrong
/// fix SHAPE (rewiring semantics instead of a value-level fallback) and self-verified against its
/// own new test. Kept deliberately short — a size test pins it ≤520 bytes so it can't grow into
/// another token-tripling preamble.
///
/// Wave 5 adds one clause: "minimal" governs the FINAL COMMITTED diff, not throwaway verification
/// work. astropy-12907's cheap path was spent on C-extension build archaeology partly because the
/// bias read as "don't touch anything" — so it never stubbed the unrelated failing `.so` it needed
/// to verify against. Permitting out-of-tree scaffolding keeps the fix-shape discipline (the
/// seaborn quality win) while unblocking verification.
const MINIMAL_DIFF_BIAS: &str = "Prefer the most local fix at the failure site. Do not change \
data-flow or filtering semantics when a value-level fallback suffices. Do not edit changelogs. \
Hidden tests assert on unchanged surrounding behavior — keep the diff minimal. Throwaway build or \
verification scaffolding in /tmp, and stubbing an unrelated failing C-extension to unblock \
verification, are fine as long as the FINAL committed diff stays minimal.";

/// The deadline-reconciliation instruction (quality guards wave 4, fix 2): injected once when a
/// turn crosses its soft deadline. The model gets ONE more completion to revert speculative work,
/// then the loop ends — the caller's hard timeout still kills the turn at the full limit.
const DEADLINE_RECONCILE_NUDGE: &str = "Time budget exhausted. Revert any UNVERIFIED speculative \
changes now (git checkout those hunks), keeping only the minimal verified fix, then stop.";

/// The soft-deadline budget for a turn bounded by a hard `timeout_secs` kill: reserve
/// `reserve_secs` for the reconciliation window (one revert turn + slack), or `None` when the
/// timeout is too small to leave a usable working budget. Pure so the gating math is
/// unit-testable; `bench swe` calls it with its per-instance timeout and a 120s reserve.
pub fn reconcile_deadline_budget_secs(timeout_secs: u64, reserve_secs: u64) -> Option<u64> {
    let budget = timeout_secs.saturating_sub(reserve_secs);
    (budget > 0).then_some(budget)
}

/// The existing-tests-are-spec guard turn (quality guards wave 4, fix 1): sent as a synthetic
/// user message after the working diff was found to MODIFY existing test files and those edits
/// were stashed. Hidden evaluation restores pristine tests, so a fix that only passes rewritten
/// expectations is a guaranteed fail (the xarray-3364 forensic: a correct 6-line fix, then a
/// refactor that broke 2 tests, then the tests' expectations rewritten to match).
const TEST_EDIT_GUARD: &str = "Your change edits existing test expectations. Hidden evaluation \
uses the ORIGINAL tests. Re-verify your core change against the pristine tests (they have been \
restored); if they fail, shrink your fix rather than editing tests. Your test edits are stashed \
(`git stash pop` re-applies them); re-apply only if genuinely justified by the issue text.";

/// Proactive counterpart to [`TEST_EDIT_GUARD`]. When the user explicitly requires pristine
/// existing tests, make the recoverable post-hoc guard exceptional rather than the normal path:
/// models may still add useful coverage, but must put it in a new file so the original contract
/// remains byte-identical and no paid restore/reverification loop is needed.
const PRISTINE_TEST_GUIDANCE: &str = "Existing tracked test files are immutable for this session. \
Read and run them, but do not edit, delete, or rename them—even to add coverage. Put useful new \
coverage in a new test file, and fix production code so the original pristine tests still pass.";

/// Prevent a common false-positive completion on rollback work: a model proves a hand-written test
/// double but never checks whether the repository's actual fault-injection seam covers the sibling
/// production path named by the task. The guidance is semantic and repository-agnostic; it is
/// injected only when the current prompt explicitly asks for fault injection or an injected
/// failure, then remains stable for the rest of that session.
const FAULT_SEAM_GUIDANCE: &str = "Fault-injection and rollback coverage must exercise the \
repository's real production seams. If an existing flag, hook, or stub models a failure, inspect \
every analogous write/save path and make that same seam behave consistently before adding \
special-case test doubles. A bespoke subclass may add coverage, but it cannot replace a test of \
the base hook. Before declaring complete, search the affected subsystem for sibling persistence \
paths and verify that each injected failure leaves state unchanged.";

/// Whether `path` looks like a test file — the small, extensible pattern list the
/// existing-tests-are-spec guard keys on. Matches by basename (`test_*.py`, `*_test.py`,
/// `*_tests.rs`, `*_test.rs`, `*.test.js/ts`, `*.spec.js/ts`, `test_*.rs`) or by living under a
/// `tests/` / `test/` / `testing/` directory component. Paths use `/` (git porcelain output).
pub(crate) fn is_test_path(path: &str) -> bool {
    let base = path.rsplit('/').next().unwrap_or(path);
    let by_name = (base.starts_with("test_") && (base.ends_with(".py") || base.ends_with(".rs")))
        || base.ends_with("_test.py")
        || base.ends_with("_test.rs")
        || base.ends_with("_tests.rs")
        || base.ends_with("_test.go")
        || [
            ".test.js",
            ".test.ts",
            ".test.jsx",
            ".test.tsx",
            ".spec.js",
            ".spec.ts",
        ]
        .iter()
        .any(|s| base.ends_with(s));
    let by_dir = path
        .split('/')
        .rev()
        .skip(1) // the basename is not a directory component
        .any(|c| c == "tests" || c == "test" || c == "testing");
    by_name || by_dir
}

fn prompt_requires_pristine_existing_tests(prompt: &str) -> bool {
    let prompt = prompt.to_ascii_lowercase();
    prompt.contains("test")
        && (prompt.contains("do not weaken")
            || prompt.contains("no tests weakened")
            || prompt.contains("do not modify existing tests")
            || prompt.contains("keep existing tests unchanged"))
}

fn session_requires_pristine_existing_tests(messages: &[Message]) -> bool {
    messages.iter().any(|message| {
        message.role == Role::User && prompt_requires_pristine_existing_tests(&message.content)
    })
}

fn prompt_requires_fault_seam_audit(prompt: &str) -> bool {
    let prompt = prompt.to_ascii_lowercase();
    let requests_injection = prompt.contains("fault injection")
        || prompt.contains("fault-injection")
        || prompt.contains("injected failure")
        || prompt.contains("injected storage")
        || prompt.contains("inject a failure")
        || prompt.contains("inject failure");
    requests_injection
        && (prompt.contains("rollback")
            || prompt.contains("storage")
            || prompt.contains("persist")
            || prompt.contains("write")
            || prompt.contains("save"))
}

/// Parse `git status --porcelain` output into the list of MODIFIED (or deleted) existing test
/// files. The status columns distinguish the red flag from allowed practice: `M`/`D` in either
/// column means an existing tracked test was rewritten/removed (the guard's target), while `A`
/// (added) and `??` (untracked) are NEW tests — writing a fresh reproduction test is normal and
/// never trips the guard. Rename lines (`R  old -> new`) are skipped: rare here, and stashing by
/// pathspec doesn't round-trip them cleanly. Pure so it is unit-testable.
fn modified_test_paths(porcelain: &str) -> Vec<String> {
    porcelain
        .lines()
        .filter_map(|line| {
            if line.len() < 4 || line.contains(" -> ") {
                return None;
            }
            let (status, path) = line.split_at(2);
            let modified = status.contains('M') || status.contains('D');
            let path = path.trim_start();
            (modified && is_test_path(path)).then(|| path.trim_matches('"').to_string())
        })
        .collect()
}

/// The working diff's modified-existing-test files at `root` (`None` = process cwd). Any git
/// failure yields an empty list so the guard can never fire outside a real repository.
fn modified_test_files_in_tree(root: Option<&std::path::Path>) -> Vec<String> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(["status", "--porcelain"]);
    if let Some(r) = root {
        cmd.current_dir(r);
    }
    match cmd.output() {
        Ok(out) if out.status.success() => {
            modified_test_paths(&String::from_utf8_lossy(&out.stdout))
        }
        _ => Vec::new(),
    }
}

/// Stash the given pathspecs at `root` (`git stash push -- <paths>`), restoring those files to
/// their committed (pristine) state while keeping the edits recoverable via `git stash pop`.
/// Returns whether the stash actually succeeded — the guard only fires on success (a failed
/// stash leaves the tree untouched, and claiming "tests restored" would then be a lie).
fn stash_paths(root: Option<&std::path::Path>, paths: &[String]) -> bool {
    let mut cmd = std::process::Command::new("git");
    cmd.args(["stash", "push", "--quiet", "--"]);
    cmd.args(paths);
    if let Some(r) = root {
        cmd.current_dir(r);
    }
    matches!(cmd.output(), Ok(out) if out.status.success())
}

/// Whether the working tree at `root` (`None` = process cwd) shows NO changes at all:
/// `git status --porcelain` output empty — no staged or unstaged modifications AND no untracked
/// files (a solution that only ADDS a file is still a change; `git diff` alone would miss it,
/// the same hole `bench swe`'s patch extraction plugs with `git add -A`). Any git failure (not
/// a repo, git missing) counts as "changed" so the empty-diff nudge can never fire outside a
/// real repository.
fn working_tree_status(root: Option<&std::path::Path>) -> Option<Vec<u8>> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(["status", "--porcelain"]);
    if let Some(r) = root {
        cmd.current_dir(r);
    }
    cmd.output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| out.stdout)
}

/// Whether the repository status changed during this turn. Comparing against the turn baseline
/// matters for serve worktrees: Forge intentionally creates an untracked `.cargo/config.toml`
/// before the turn starts, which is not task progress and must not suppress the empty-diff guard.
/// Outside a git repository, conservatively report changed so code-change nudges never fire.
fn working_tree_changed_since(root: Option<&std::path::Path>, baseline: Option<&[u8]>) -> bool {
    match (baseline, working_tree_status(root)) {
        (Some(before), Some(after)) => before != after,
        _ => true,
    }
}

fn git_head(root: Option<&std::path::Path>) -> Option<String> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(["rev-parse", "HEAD"]);
    if let Some(root) = root {
        cmd.current_dir(root);
    }
    cmd.output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|head| head.trim().to_string())
        .filter(|head| !head.is_empty())
}

/// Classify a completed bridge turn as TOOLS-UNAVAILABLE (harness wave 7): the model ran with no
/// working write tools because Forge's `mcp-serve` server failed to start, so a silent empty
/// completion is a broken attempt, NOT "the model chose not to edit". True only when ALL hold:
/// the session expected a code change; the model is a CLI bridge; the child emitted an
/// MCP-startup/tool-unavailable signal (`mcp_startup_failed`); zero forge tools ran this turn; and
/// the working tree is still unchanged. Kept DISTINCT from the wave-2 empty-diff nudge, which fires
/// on a normal empty completion (no startup-failure signal) and re-drives in-process — this signal
/// instead drives a fresh-process retry at the harness level. Pure so the gate is unit-testable.
fn classify_tools_unavailable(
    expect_code_change: bool,
    is_bridge: bool,
    mcp_startup_failed: bool,
    forge_tools_ran: u64,
    tree_unchanged: bool,
) -> bool {
    expect_code_change && is_bridge && mcp_startup_failed && forge_tools_ran == 0 && tree_unchanged
}

/// Lightweight check that `args` satisfies the tool's JSON `schema`: it must be an object and
/// contain every key the schema lists as `required`. Returns a human-readable reason on failure
/// (naming the missing field(s) + the full required list) so the model can fix the call. Kept
/// dependency-free — required-key + object-shape covers the overwhelmingly common malformed call;
/// deep type validation isn't worth a JSON-schema crate here.
fn validate_tool_args(schema: &serde_json::Value, args: &serde_json::Value) -> Result<(), String> {
    let Some(obj) = args.as_object() else {
        return Err("arguments must be a JSON object".to_string());
    };
    if let Some(reason) = obj
        .get(forge_provider::TRUNCATED_TOOL_ARGS_KEY)
        .and_then(|value| value.as_str())
    {
        return Err(reason.to_string());
    }
    let required = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|k| !obj.contains_key(*k))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(format!(
        "missing required field(s): {}. Required: {}",
        missing.join(", "),
        required.join(", ")
    ))
}

/// A stable hash of a tool-call batch (each call's name + JSON arguments), used by the agent loop's
/// doom-loop guard to detect a model repeating the *exact* same call(s) step after step. Identical
/// args → identical result, so a repeat is a death-spiral (re-reading a file, retrying a failing
/// edit) worth halting rather than burning steps on.
fn tool_batch_signature(calls: &[forge_types::ToolCall]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for c in calls {
        c.name.hash(&mut h);
        c.args.to_string().hash(&mut h);
    }
    h.finish()
}

/// Decision of the completion-verification gate for a turn that reported every tracked task Done.
/// A self-reported "all done" is exactly what produced the phantom release (claimed merged + tagged
/// while nothing ran), so completion must be PROVEN with a real state check, not asserted.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletionGate {
    /// Force another tool-grounded verification turn (the caller pushes the verify nudge + loops).
    Reverify,
    /// A real inspection ran — accept cleanly, no note.
    AcceptClean,
    /// Nothing external to check (a pure-analysis answer) — accept with a calm note.
    AcceptNoArtifacts,
    /// Verification budget spent but real work existed and was never checked — accept, flag loudly.
    AcceptUnverified,
}

/// Decide whether an "all tasks Done" claim is accepted or must be re-verified with a real state
/// check. Pure (no I/O) so it is unit-testable; the caller emits the warning and pushes the nudge.
///
/// * `verify_attempts`    — verification turns already spent on the CURRENT claim (0 = first claim).
/// * `did_real_work`      — the turn ran ≥1 inspectable tool at some point, so there IS external
///   state to check (a pure-reasoning turn has none — requiring an inspection would over-fire).
/// * `inspected_this_turn`— the just-observed turn ran an inspection tool (a real check), as opposed
///   to merely re-asserting "done" by re-marking the task list (the C8 hole).
///
/// Shared by the CLI-bridge and direct-API paths so both have ONE completion authority. A completed
/// no-op task is accepted when the model explains that no change is needed; other claims with
/// evidence get one verification chance.
fn completion_claims_no_change(text: &str) -> bool {
    completion::claims_no_change(text)
}

/// A response that explicitly promises another agent action and then stops at an open-ended marker
/// is not a final answer. Keep this deliberately narrow: headings such as `What changed:` are valid
/// prose, while `Let me verify ...:` means the model yielded before doing what it just promised.
fn completion_promises_followup(text: &str) -> bool {
    let trimmed = text.trim();
    if !(trimmed.ends_with(':') || trimmed.ends_with("...") || trimmed.ends_with('…')) {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let tail_reversed: String = lower.chars().rev().take(320).collect();
    let tail: String = tail_reversed.chars().rev().collect();
    let intent_at = [
        "let me ",
        "i'll ",
        "i will ",
        "i'm going to ",
        "i’m going to ",
        "i need to ",
        "now i ",
        "next i ",
    ]
    .iter()
    .filter_map(|marker| tail.rfind(marker))
    .max();
    let Some(intent) = intent_at.map(|at| &tail[at..]) else {
        return false;
    };
    [
        "check",
        "verify",
        "test",
        "run",
        "inspect",
        "review",
        "investigate",
        "fix",
        "edit",
        "write",
        "create",
        "update",
        "continue",
        "try",
    ]
    .iter()
    .any(|action| intent.contains(action))
}

#[cfg(test)]
fn completion_gate(
    verify_attempts: usize,
    max_attempts: usize,
    did_real_work: bool,
    no_change_required: bool,
    inspected_this_turn: bool,
) -> CompletionGate {
    match CompletionContract::with_observation_budget(max_attempts).decide(
        TaskIntent::Mutating,
        verify_attempts,
        CompletionEvidence {
            did_real_work,
            no_change_required,
            inspected_this_turn,
        },
    ) {
        CompletionDecision::RequestObservation => CompletionGate::Reverify,
        CompletionDecision::AcceptClean => CompletionGate::AcceptClean,
        CompletionDecision::AcceptNoArtifacts => CompletionGate::AcceptNoArtifacts,
        CompletionDecision::AcceptUnverified => CompletionGate::AcceptUnverified,
    }
}

fn completion_verification_empty_is_terminal(
    verify_attempts: usize,
    tasks: &[forge_types::TodoItem],
    has_prior_final: bool,
) -> bool {
    completion::empty_verification_is_terminal(verify_attempts, tasks, has_prior_final)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskIntent {
    ReadOnlyReview,
    PlanOnly,
    Mutating,
    Verification,
}

impl TaskIntent {
    pub(crate) fn is_observational(self) -> bool {
        !matches!(self, Self::Mutating)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskScope {
    task: String,
    contract: turn_contract::TurnContract,
    root: Option<std::path::PathBuf>,
    allowed_paths: Vec<std::path::PathBuf>,
    base_head: Option<String>,
    permission: PermissionMode,
    origin_seq: i64,
    origin_incarnation: String,
    origin_generation: u64,
}

impl TaskScope {
    #[allow(clippy::too_many_arguments)]
    fn for_turn(
        task: &str,
        contract: turn_contract::TurnContract,
        mode: PermissionMode,
        origin_seq: i64,
        root: Option<std::path::PathBuf>,
        base_head: Option<String>,
        origin_incarnation: String,
        origin_generation: u64,
    ) -> Self {
        Self {
            task: task.to_string(),
            contract,
            root,
            allowed_paths: Vec::new(),
            base_head,
            permission: mode,
            origin_seq,
            origin_incarnation,
            origin_generation,
        }
    }

    #[cfg(test)]
    fn for_test(
        task: &str,
        intent: TaskIntent,
        mode: PermissionMode,
        origin_seq: i64,
        root: Option<std::path::PathBuf>,
    ) -> Self {
        Self::for_turn(
            task,
            turn_contract::TurnContract::for_test(intent),
            mode,
            origin_seq,
            root,
            None,
            "test".to_string(),
            0,
        )
    }

    fn permits_tool(&self, tool: &str) -> bool {
        if !self.contract.intent().is_observational() {
            return true;
        }
        !matches!(
            tool,
            "write_file"
                | "edit_file"
                | "delete_file"
                | "apply_patch"
                | "shell"
                | "spawn_agents"
                | "send_to_agent"
                | "run_workflow"
                | "update_tasks"
                | "remember"
        )
    }

    fn audit_digest(&self) -> String {
        use std::hash::{Hash, Hasher};

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.task.hash(&mut hasher);
        self.contract.hash(&mut hasher);
        self.root.hash(&mut hasher);
        self.allowed_paths.hash(&mut hasher);
        self.base_head.hash(&mut hasher);
        self.permission.label().hash(&mut hasher);
        self.origin_seq.hash(&mut hasher);
        self.origin_incarnation.hash(&mut hasher);
        self.origin_generation.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }
}

/// Typed outcome for the post-completion gate. Only `RequestObservation` may re-drive a model,
/// and its prompt is fixed observational text rather than an implementation instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PostCheckDecision {
    RequestObservation,
    AcceptClean,
    AcceptNoArtifacts,
    AcceptUnverified,
}

fn post_check_decision(
    intent: TaskIntent,
    verify_attempts: usize,
    did_real_work: bool,
    no_change_required: bool,
    inspected_this_turn: bool,
) -> PostCheckDecision {
    match CompletionContract::production().decide(
        intent,
        verify_attempts,
        CompletionEvidence {
            did_real_work,
            no_change_required,
            inspected_this_turn,
        },
    ) {
        CompletionDecision::RequestObservation => PostCheckDecision::RequestObservation,
        CompletionDecision::AcceptClean => PostCheckDecision::AcceptClean,
        CompletionDecision::AcceptNoArtifacts => PostCheckDecision::AcceptNoArtifacts,
        CompletionDecision::AcceptUnverified => PostCheckDecision::AcceptUnverified,
    }
}

/// Decision of the token-budget continuation guard (H8): when a code-change turn ends without a
/// verified result, should Forge nudge the model to actually do the work, accept the turn as-is, or
/// halt an unproductive spiral with an honest reason? Resolved once per continuation from signals
/// that work on BOTH the direct-API path AND the CLI bridge (see [`continuation_decision`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContinuationDecision {
    /// Under budget, no real progress this turn, goal unverified — push back once more and re-drive
    /// the model instead of accepting a premature "done".
    Nudge,
    /// Diminishing returns: it kept "continuing" while emitting almost nothing (or hit the absolute
    /// ceiling). Halt with an honest surfaced reason rather than looping forever.
    Stop,
    /// Accept the turn as-is: the goal is verified, real progress was made, or there is no budget
    /// headroom left for a productive re-drive.
    Accept,
}

/// Continuations that must have already fired before a tiny-output turn counts as diminishing
/// returns — the spec floor at which a "keeps saying done, emits nothing" spiral is stopped.
const CONTINUATION_DIMINISHING_MIN: usize = 3;
/// Transcript-growth (tokens) below which a continuation produced "almost nothing".
const CONTINUATION_DIMINISHING_TOKEN_FLOOR: u64 = 500;
/// Absolute continuation ceiling so the guard can NEVER loop forever, even if every re-drive keeps
/// emitting more than [`CONTINUATION_DIMINISHING_TOKEN_FLOOR`] tokens without making real progress.
const CONTINUATION_MAX: usize = 6;
/// Only nudge with real budget headroom — at/above this fraction of the context window a re-drive
/// has no room to work, so accept instead of nudging into the wall.
const CONTINUATION_BUDGET_CEILING: f64 = 0.90;

/// Pure decision for the token-budget continuation guard. No I/O, so it is offline-unit-testable
/// with synthetic inputs — no live model required.
///
/// * `goal_verified`      — the completion authority accepts the turn (tasks done + verified, or
///   nothing external to verify). A verified goal is never nudged.
/// * `made_progress`      — BRIDGE-AWARE: this turn started ≥1 tool (direct calls AND bridge-sink
///   `StreamEvent::ToolStarted`) or changed the working tree / closed a task. Real progress is
///   never nudged — the caller derives this from the turn's git-status baseline + sink tool count,
///   both of which reflect a CLI bridge's activity, not just the direct `resp.tool_calls` path.
/// * `budget_used`        — this turn's input tokens / model context window (≈0.0..=1.0). Only
///   nudge below [`CONTINUATION_BUDGET_CEILING`].
/// * `continuation_count` — continuation nudges already fired this turn (0 on the first check).
/// * `delta_tokens_last`  — tokens the LAST continuation grew the managed transcript by; a tiny
///   delta after several continuations is the diminishing-returns spiral. Pass a large sentinel
///   (e.g. `u64::MAX`) before any continuation has run so the stop can't fire on the first check.
fn continuation_decision(
    goal_verified: bool,
    made_progress: bool,
    budget_used: f64,
    continuation_count: usize,
    delta_tokens_last: u64,
) -> ContinuationDecision {
    // A verified goal or a turn that actually did work needs no nudge.
    if goal_verified || made_progress {
        return ContinuationDecision::Accept;
    }
    // Diminishing returns / absolute ceiling: stop the spiral with an honest reason instead of
    // re-driving a model that keeps "continuing" while producing nothing.
    if continuation_count >= CONTINUATION_MAX
        || (continuation_count >= CONTINUATION_DIMINISHING_MIN
            && delta_tokens_last < CONTINUATION_DIMINISHING_TOKEN_FLOOR)
    {
        return ContinuationDecision::Stop;
    }
    // No budget headroom for a productive re-drive — accept rather than nudge into the window wall.
    if budget_used >= CONTINUATION_BUDGET_CEILING {
        return ContinuationDecision::Accept;
    }
    ContinuationDecision::Nudge
}

/// Build the failover chain for a cheap trivial-tier side-call (compaction, classification):
/// the health-filtered top-3 of `trivial` (the router's ranked shortlist) FIRST, then `routed`
/// (the routed model + its failover chain — preserves the pre-existing rate-limit failover so a
/// 429 on the summarizer walks to the routed fallback), then `guaranteed` (the session's own,
/// reachable model) last as a can't-exhaust backstop. Order-preserving dedup, empties dropped, and
/// never empty. Pure/no I/O so it's offline-unit-testable without a live router.
fn compact_candidate_chain(
    trivial: Vec<String>,
    routed: Vec<String>,
    guaranteed: &str,
    is_benched: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let add = |m: String, out: &mut Vec<String>| {
        if !m.is_empty() && !out.iter().any(|x| x == &m) {
            out.push(m);
        }
    };
    for m in trivial.into_iter().filter(|m| !is_benched(m)).take(3) {
        add(m, &mut out);
    }
    for m in routed {
        add(m, &mut out);
    }
    add(guaranteed.to_string(), &mut out);
    if out.is_empty() {
        out.push(guaranteed.to_string());
    }
    out
}

/// Reply room reserved for the compaction summary itself when sizing its request.
///
/// Deliberately far smaller than the main loop's [`output_planning_reserve_tokens`]:
/// [`COMPACT_SYSTEM`] asks for a concise summary, not an unbounded answer, and reserving the main
/// loop's 8k planning cushion would leave an 8k-window trivial summarizer with no input budget at
/// all — the chain is trivial-tier first precisely because those models are cheap and small.
const COMPACT_SUMMARY_RESERVE_TOKENS: usize = 2_048;

/// Stands in for the messages a compaction payload had to drop. `{}` is the dropped count.
///
/// Said out loud in the payload rather than dropped silently, so the summarizer knows its input has
/// a hole in it instead of inventing continuity across the seam.
const COMPACT_ELISION_MARKER: &str =
    "\n[… {} message(s) from the middle of this stretch were omitted because the whole stretch did \
     not fit the summarizer's context window …]\n";

/// Fit a rendered compaction payload into `budget_tokens` by dropping from the MIDDLE.
///
/// The newest-first trimming `context_pipeline::fit_messages` does for a normal request is exactly
/// backwards here: compaction's job is to stand in for the OLDEST messages, so keeping only the tail
/// would summarize the stretch nearest the messages that survive verbatim anyway and silently lose
/// the ones the summary is supposed to replace. So keep both ends instead — the opening messages
/// (the task statement and early decisions everything after them refers back to) and, with the
/// larger share of the budget, the end of the older stretch (the live state the kept-recent messages
/// continue from). What goes is the middle, and [`COMPACT_ELISION_MARKER`] says so.
///
/// `entries` is one rendered line-group per summarizable message; the caller keeps them unjoined so
/// this can be re-run per failover hop against each candidate model's own window.
fn fit_compaction_payload(entries: &[String], budget_tokens: usize) -> String {
    let costs: Vec<usize> = entries.iter().map(|e| tokens::count_message(e)).collect();
    if costs.iter().sum::<usize>() <= budget_tokens {
        return entries.join("\n");
    }
    let marker = |dropped: usize| COMPACT_ELISION_MARKER.replacen("{}", &dropped.to_string(), 1);
    // Charge the marker at its widest possible count, so the assembled payload can never come out
    // over budget just because the omitted count grew a digit.
    let budget = budget_tokens.saturating_sub(tokens::count_message(&marker(entries.len())));

    let mut used = 0usize;
    let mut tail = entries.len();
    let tail_budget = budget * 3 / 4;
    while tail > 0 && used + costs[tail - 1] <= tail_budget {
        tail -= 1;
        used += costs[tail];
    }
    let mut head = 0usize;
    while head < tail && used + costs[head] <= budget {
        used += costs[head];
        head += 1;
    }
    let dropped = tail - head;
    if dropped == 0 {
        return entries.join("\n");
    }
    if head == 0 && tail == entries.len() {
        // Not even one whole message fits. Keep the tail characters of the most recent one rather
        // than sending an empty payload the summarizer would answer with nothing.
        return tail_within_token_budget(
            entries.last().map(String::as_str).unwrap_or_default(),
            budget_tokens,
        );
    }
    let mut out = entries[..head].join("\n");
    out.push_str(&marker(dropped));
    out.push_str(&entries[tail..].join("\n"));
    out
}

/// The longest suffix of `text` that costs at most `budget_tokens`, found by bisecting on chars.
/// The suffix (not the prefix) because it is the most recent — the same reason
/// `truncate_message_to_budget` keeps a message's tail.
fn tail_within_token_budget(text: &str, budget_tokens: usize) -> String {
    if tokens::count_message(text) <= budget_tokens {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let suffix = |keep: usize| -> String { chars[chars.len() - keep..].iter().collect() };
    let (mut fits, mut too_long) = (0usize, chars.len());
    while fits + 1 < too_long {
        let mid = fits + (too_long - fits) / 2;
        if tokens::count_message(&suffix(mid)) <= budget_tokens {
            fits = mid;
        } else {
            too_long = mid;
        }
    }
    suffix(fits)
}

/// Classify a tool RESULT string as a failure of a given kind, or `None` if it looks like a success.
///
/// Anchored on the markers Forge actually produces for failures (`invoke_tool` returns `"error: …"`
/// for a tool `Err`, `"permission denied by policy"` for a blocked call, and [`shell_command_failed`]
/// recognises a non-zero shell exit) — so a *successful* tool output that merely happens to contain a
/// word like "invalid" is NOT misread as a failure. The category is then a keyword sniff of the
/// message. Only consumed behind a ≥3 threshold, so the worst case of a misclassification is one
/// early, still-helpful "change approach" nudge.
fn classify_tool_failure(result: &str) -> Option<ErrorCategory> {
    let lower = result.to_ascii_lowercase();
    let is_failure = lower.starts_with("error:")
        || lower.starts_with("permission denied")
        || shell_command_failed(result);
    if !is_failure {
        return None;
    }
    let kind = if lower.contains("permission denied")
        || lower.contains("forbidden")
        || lower.contains("access is denied")
        || lower.contains("eacces")
    {
        ErrorCategory::Permission
    } else if lower.contains("no such file")
        || lower.contains("not found")
        || lower.contains("does not exist")
        || lower.contains("cannot find")
        || lower.contains("no matches found")
    {
        ErrorCategory::NotFound
    } else if lower.contains("timed out") || lower.contains("timeout") {
        ErrorCategory::Timeout
    } else if lower.contains("invalid")
        || lower.contains("no match")
        || lower.contains("old_string")
        || lower.contains("expected")
        || lower.contains("malformed")
        || lower.contains("could not parse")
        || lower.contains("unexpected")
    {
        ErrorCategory::Schema
    } else {
        ErrorCategory::Other
    };
    Some(kind)
}

/// The live context-fill token count to report on the gauge for `model` after a call.
///
/// For a direct API model the provider's reported `input_tokens` IS the request size, the truest
/// fill measure. But a subscription CLI bridge (claude-cli/codex-cli) runs its own internal agent
/// loop and reports CUMULATIVE usage across every internal step — not the size of the request we
/// sent — so over a long turn it balloons past the window (e.g. 900k against a 272k context). There
/// the conservative transcript estimate, which reflects the context we actually manage, is correct.
/// Gated on `is_cli_bridge`, NOT `is_subscription`: `xai-oauth::` is subscription-billed but is a
/// normal single-request API call (not an internal multi-step loop), so its `reported_input` is
/// already accurate — using the transcript estimate for it would just be a worse number.
fn context_fill_tokens(model: &str, transcript_est: u64, reported_input: u64) -> u64 {
    if forge_provider::is_cli_bridge(model) {
        transcript_est
    } else {
        reported_input
    }
}

/// A generic provider outage is shared health and should fail over immediately. The Codex backend's
/// in-stream `provider request failed` is different: the request/transcript is valid and a manual
/// `continue` was observed to resume it, so retry the same continuation through the existing bounded
/// transient backoff before benching the model.
fn should_retry_same_model_transient(model: &str, error: &forge_provider::ProviderError) -> bool {
    match error {
        forge_provider::ProviderError::Unavailable(message) => {
            model.starts_with("codex-oauth::")
                && message
                    .to_ascii_lowercase()
                    .contains("provider request failed")
        }
        _ => true,
    }
}

// `message_tokens`, `fit_messages`, and `prune_tool_results` moved to [`context_pipeline`] — the
// one seam between the transcript and a provider request (imported below for existing call sites).
#[cfg(test)]
use context_pipeline::{fit_messages, prune_tool_results, PRUNE_MARKER, PRUNE_TOOL_RESULT_MAX};
use context_pipeline::{message_tokens, prune_and_inject, to_llm};

/// Output of one execution of the shared model↔tool loop ([`Session::run_model_loop`]).
/// Carries everything the caller needs; the caller holds `active_model` by value so it is
/// returned here (failover may have changed it from the original).
struct ModelLoopOutcome {
    final_text: String,
    context_tokens: u64,
    hit_step_cap: bool,
    /// A repetition/failure guard deliberately ended this loop. Outer recovery passes must honor
    /// that terminal decision instead of starting a fresh loop with reset guard counters.
    halted_by_loop_guard: bool,
    /// The model that produced the last response (may differ from the input if failover fired).
    active_model: String,
    /// A plan a CLI-bridge model proposed this loop (tailed from the sink as [`StreamEvent::Plan`]).
    /// `None` on the in-process path, where the `present_plan` handler sets `pending_plan` directly.
    plan: Option<forge_types::PlanProposal>,
    /// How many tools STARTED executing across this loop (direct-path calls + bridge tools tailed
    /// from the sink). The empty-diff completion nudge keys on it: "the model worked (tools ran)
    /// but changed nothing" is the description-instead-of-implementation failure mode.
    tools_ran: u64,
    /// A CLI-bridge completion this loop reported that Forge's `mcp-serve` tool server failed to
    /// start (`StreamEvent::ToolsUnavailable`), so the model's write tools were never exposed
    /// (wave 7). Combined with a zero-tool, empty-tree turn this is the toolless-bridge signal the
    /// harness retries on — distinct from a normal empty completion (the wave-2 nudge's job).
    mcp_tools_unavailable: bool,
}

/// A short-lived snapshot used by the mesh inspector. It owns the live router but snapshots the
/// mutable quota, health, and budget inputs before its LLM classification call, so callers can
/// render `/mesh` without holding a session lock across network I/O.
pub struct RoutingInspector {
    router: Arc<dyn Router>,
    selection_router: HeuristicRouter,
    budget: BudgetState,
    health: ModelHealth,
    quota: SubscriptionQuota,
    tier_override: Option<TaskTier>,
    effort: Option<EffortLevel>,
    project: ProjectContext,
    routing_context: RoutingContext,
}

impl RoutingInspector {
    /// Classify with the same live router as a real turn, then expose the shared scoring trace.
    pub async fn explain(&self, prompt: &str) -> forge_mesh::RoutingExplanation {
        let decision = self
            .router
            .route_contextual(
                prompt,
                false,
                self.budget,
                &self.health,
                &self.quota,
                self.tier_override,
                self.effort,
                &self.project,
                &self.routing_context,
            )
            .await;
        let classifier_reason = decision
            .rationale
            .split(" — ")
            .next()
            .unwrap_or(&decision.rationale)
            .to_string();
        let fallback = decision.rationale.contains("llm classify unavailable");
        let mut explanation = self.selection_router.explain_contextual_classified(
            prompt,
            decision.tier,
            vec![classifier_reason],
            self.budget,
            &self.health,
            &self.quota,
            self.effort,
            &self.routing_context,
        );
        explanation.classifier_label = if fallback {
            "heuristic fallback (all LLM candidates unavailable)".to_string()
        } else {
            "llm".to_string()
        };
        explanation
    }
}

/// One interactive session. Construct with [`Session::start`], then drive [`Session::run_turn`].
pub struct Session {
    id: String,
    pub store: Arc<Store>,
    provider: Arc<dyn Provider>,
    router: Arc<dyn Router>,
    tools: ToolRegistry,
    presenter: Box<dyn Presenter>,
    config: Config,
    pricing: Pricing,
    mode: PermissionMode,
    /// The exact non-Plan temper that was active before entering read-only planning. Kept only
    /// while `mode == Plan`; approving or cancelling restores it. Older/resumed Plan sessions have
    /// no captured value and deliberately fall back to Auto-edit when leaving Plan.
    pre_plan_mode: Option<PermissionMode>,
    /// Resolved permission rules (built-in safety denies + configured), consulted per call.
    rules: Vec<PermissionRule>,
    transcript: Vec<Message>,
    seq: i64,
    /// Where code shadow-snapshots live (RFC PR3); defaults to `.forge/checkpoints`.
    checkpoint_root: std::path::PathBuf,
    checkpoint_root_custom: bool,
    /// The seq that began the current turn (its user message), keying this turn's snapshot dir.
    current_turn_seq: i64,
    /// The discovered model catalog (auto-discovery mesh), kept so the TUI `/models` browser can
    /// classify + group what's available without re-running discovery. `None` for mock/offline.
    catalog: Option<ModelCatalog>,
    /// The agent's task list (the `update_tasks` tool), rehydrated from the store on resume.
    tasks: Vec<forge_types::TodoItem>,
    /// A plan proposed this turn via `present_plan` (planning mode), awaiting interactive approval
    /// at turn end. `Some` between the proposal and the approve/revise/cancel decision.
    pending_plan: Option<forge_types::PlanProposal>,
    /// Immutable authority for the active turn. Observational scopes cannot mutate state or spawn.
    task_scope: Option<TaskScope>,
    /// Connected external MCP servers (mcp-client.md). `None` when no servers are configured —
    /// the whole MCP path is then inert (zero overhead for non-MCP users).
    mcp: Option<Arc<forge_mcp::McpManager>>,
    /// The code-intelligence index (code-intelligence.md). `None` when disabled or unavailable —
    /// retrieval then injects nothing and the turn runs exactly as before (additive guarantee).
    /// `Arc` so the model-facing `lattice` tool shares the same index.
    lattice: Option<Arc<Lattice>>,
    /// Background file watcher that keeps the index fresh on external edits. Held as the receiving
    /// end of a channel: the watcher is built off-thread (so a slow filesystem can't gate startup)
    /// and delivered here, where it lives in the channel buffer for the session's lifetime (this
    /// Receiver dropped → channel + watcher drop → watching stops). Per-session ownership so repeated
    /// `build_session` calls (bench, replay) don't leak watcher threads.
    lattice_watcher: Option<std::sync::mpsc::Receiver<forge_index::LatticeWatcher>>,
    /// Once setup completes, hold the watcher here so backend errors can be surfaced at the next
    /// user-turn boundary instead of remaining stranded in the delivery channel.
    lattice_watcher_handle: Option<forge_index::LatticeWatcher>,
    /// Whether a workspace transition must recreate the lattice watcher.
    lattice_watch_enabled: bool,
    /// LSP registry for live diagnostics after writes. `None` when lsp.enabled = false.
    lsp: Option<Arc<forge_lsp::LspRegistry>>,
    /// The discovered command/skill catalog, so the model can find + load Forge's own skills via
    /// the `use_skill` virtual tool (command-skill-system.md). `None` → the tool is not advertised
    /// and the turn runs exactly as before.
    skills: Option<Arc<forge_skills::Catalog>>,
    /// Fleet agent-to-agent messaging callback (composition root; forge-cli's daemon driver wires
    /// this in — see `fleet.rs`). `None` outside `forge serve`, so `message_session` is simply
    /// not advertised there.
    fleet: Option<Arc<dyn fleet::FleetMessaging>>,
    /// In-session model pin set (`/model <id>`). When set, mesh routing still classifies the prompt
    /// (for stats), but this set is used instead of the routed pick. `None` = mesh routing.
    pinned_model: Option<Vec<String>>,
    /// In-session reasoning-effort pin (`/effort <level>`). When set, forwarded to the provider
    /// as a `ReasoningEffort` hint each turn. `None` = provider default (no hint sent).
    pinned_effort: Option<EffortLevel>,
    /// Per-turn shrinking cap on the usable context window (tokens), armed only after a provider
    /// context-overflow error and reset at the start of each turn. Each overflow retry lowers it so
    /// the SENT transcript view trims harder — a non-destructive self-heal (the stored transcript is
    /// untouched) that converges under the model's real window even when our token estimate diverges
    /// from the model's own tokenizer. `None` = no cap (use the model's full window). Keyed to the
    /// model that armed it, so a mid-turn failover to a DIFFERENT (e.g. larger-window) model ignores
    /// a cap derived from the overflowing model's window instead of needlessly over-trimming it.
    overflow_window_cap: Option<(String, u32)>,
    /// Whether white-hot effort's standing orchestration guidance has been injected this session
    /// (docs/features/whitehot-effort.md). One-shot per pin: re-armed by `set_effort` on any
    /// change, so toggling away and back re-injects for the new stretch of the transcript.
    whitehot_guidance_injected: bool,
    /// In-session routing-tier override (the `tier_up`/`tier_down` keybinds). When set, it biases
    /// the mesh to route the next turn at this tier instead of the classifier's pick — unless a
    /// per-turn `tier_override` (a command/skill `tier:` hint) is passed, which still wins. `None`
    /// = normal classification.
    pinned_tier: Option<TaskTier>,
    /// The exact model that most recently completed this session's main task turn. This is
    /// session-local cache warmth, not a model pin: contextual mesh routing may retain it within a
    /// quality band, while health, quota, context, task-class changes, and failover still win.
    route_affinity: Option<SessionAffinity>,
    /// Per-session immutable workspace root. Every runtime filesystem operation must
    /// use this instead of the daemon's process working directory.
    workspace: WorkspaceContext,
    workspace_binding: Arc<std::sync::RwLock<std::path::PathBuf>>,
    /// System hints queued by side-call diagnostics (e.g. shell error interceptor) to be injected
    /// into the transcript immediately after the tool result that triggered them. Cleared each time.
    pending_hints: Vec<String>,
    /// Session-scoped "always" answer to the auto-compact-on-switch consent prompt: once the user
    /// picks "always", a mesh failover to a model that needs compaction proceeds silently for the
    /// rest of this session (reset next launch). `false` = ask each time.
    always_compact_on_switch: bool,
    /// Whether `.forge/AGENTS.md` (or `AGENTS.md`) has been injected as a standing system prompt.
    /// False for fresh sessions so it's injected on the first turn; true for resumed sessions
    /// (the content is already in the stored transcript) and after injection.
    project_prompt_injected: bool,
    /// Images attached to the *next* user turn (vision input, e.g. via `/image <path>`). Drained
    /// when that turn's user message is built; empty for text-only turns.
    pending_images: Vec<forge_types::ImageAttachment>,
    /// Count of successful writes made by `invoke_tool` in the current turn. Reset at the start
    /// of each turn; used to gate the autofix stage (skip it when nothing was edited).
    edits_this_turn: u32,
    /// Headless code-change mode (harness-robustness wave 2): the caller KNOWS each prompt
    /// demands a code change (`bench swe` sets it — an explicit option, not prompt sniffing).
    /// With `mesh.nudge_empty_diff`, a turn that ran tools but edited nothing and left the git
    /// tree clean gets ONE "implement it, don't describe it" push-back. Never set interactively.
    expect_code_change: bool,
    /// Set by the last [`Session::run_turn`] on an `expect_code_change` bridge turn that was
    /// classified TOOLS-UNAVAILABLE (wave 7): Forge's `mcp-serve` tool server failed to start, so
    /// the model ran with no write tools and produced an empty tree. Read by the harness
    /// ([`Session::tools_unavailable`]) to retry the instance on a fresh bridge process instead of
    /// scoring a silent toolless run as a clean empty completion. Recomputed each turn; only ever
    /// true when `mesh.bridge_require_tools` is on and the session expects a code change.
    tools_unavailable_run: bool,
    /// Soft turn deadline (quality guards wave 4): set by a caller that enforces a HARD timeout
    /// the session cannot see (`bench swe`'s tokio timeout), minus a reserve. Once past it the
    /// model loop stops launching new completions except ONE reconciliation turn ("revert
    /// unverified speculative changes, then stop"). `None` = no deadline (interactive default).
    turn_deadline: Option<std::time::Instant>,
    /// One-shot latch for the deadline-reconciliation instruction; re-armed by
    /// [`Session::set_turn_deadline`].
    deadline_reconciled: bool,
    /// Env-fight spend cap (quality guards wave 4): consecutive failed env-provisioning shell
    /// commands this turn + the once-per-turn nudge latch. Reset at each turn start.
    env_fight: EnvFightTracker,
    /// Per-turn guard against repeated failing tools and identical-call doom loops.
    failure_tracker: ToolFailureTracker,
    /// The current git branch (`.git/HEAD` → `refs/heads/<branch>`), cached so the hot per-request
    /// `system_preamble` reads a field instead of doing a blocking `std::fs` syscall on the async
    /// executor. Seeded at construction and refreshed once per turn (via `tokio::fs`, AFTER the
    /// user message is persisted so the refresh await can't reopen the abort-before-persist
    /// window). `None` outside a git repo.
    cached_git_branch: Option<String>,
    /// The project `AGENTS.md` body, read ONCE at construction (sync, off the async executor) so the
    /// first turn can inject it await-free + syscall-free — a `tokio::fs` read at the injection site
    /// deterministically reintroduces the abort-before-persist cancel window on the current-thread
    /// runtime. `None` for a resumed session (already in the transcript) or when no file exists;
    /// `take()`-n on injection.
    cached_agents_md: Option<String>,
    /// What project/codebase this session is operating in (project_context.rs) — read ONCE at
    /// construction, same rationale as `cached_git_branch`, and passed to the mesh router on every
    /// `route`/`route_hinted` call so it can weight self-hosting infrastructure work correctly.
    project: forge_types::ProjectContext,
    /// Audit record of system context injected during the latest executed turn.
    last_context_pack: context_pack::ContextPack,
    /// The explicit completion expectation active during the latest turn.
    last_turn_contract: turn_contract::TurnContract,
    /// In-memory abort handles for this session's currently-running detached children (retained
    /// async subagents), so `cancel_subagent` can stop a live task in THIS process. The durable
    /// half (status/result) lives in the `detached_child` store table and survives a restart;
    /// this registry does not — it's rebuilt empty on every fresh `Session`, which is correct: a
    /// new process has no live tasks to abort, only rows to reconcile (see `Session::resume`).
    detached_registry: subagent::DetachedRegistry,
    /// Completed turns since the last Continual Harness refinement (refinement.rs). Drives
    /// `harness.auto_refine = "turns"`; reset whenever [`Session::refine`] runs, manual or auto.
    turns_since_refine: u32,
}

/// Parse `.git/HEAD` contents into a branch name (`ref: refs/heads/<branch>` → `<branch>`).
/// Returns `None` for a detached HEAD (a raw commit hash) or anything unexpected.
fn parse_git_head(head: &str) -> Option<String> {
    head.strip_prefix("ref: refs/heads/")
        .map(|b| b.trim().to_string())
}

/// Read + parse the current git branch synchronously. Used only at session construction (one-time
/// setup, not on the async turn path); the hot path refreshes the cache via `tokio::fs`.
fn current_git_branch(root: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(root.join(".git/HEAD"))
        .ok()
        .as_deref()
        .and_then(parse_git_head)
}

/// Read the project `AGENTS.md` synchronously (`.forge/AGENTS.md`, then `AGENTS.md`), returning the
/// first non-empty body. Used only at session construction (one-time setup, not on the async turn
/// path) so the first-turn injection site stays await-free + syscall-free.
fn read_project_agents_md(root: &std::path::Path) -> Option<String> {
    for path in [root.join(".forge/AGENTS.md"), root.join("AGENTS.md")] {
        if let Ok(body) = std::fs::read_to_string(path) {
            if !body.trim().is_empty() {
                return Some(body);
            }
        }
    }
    None
}

/// Merge semantics for [`resolved_subscription_plans`], pulled out as a pure function so it's
/// unit-testable without touching the keyring or `~/.codex/auth.json` (see the test module
/// below): `detected` wins per key — it's read live from the account actually in use, so it
/// cannot drift from it — and `config` fills any key `detected` didn't have (`agy-cli` /
/// `xai-oauth`, or a codex surface with no live session).
/// Documented in docs/features/mesh-routing.md.
fn merge_subscription_plans(
    mut config: std::collections::HashMap<String, String>,
    detected: std::collections::HashMap<String, String>,
) -> std::collections::HashMap<String, String> {
    config.extend(detected);
    config
}

/// `config.mesh.subscriptions` merged with live per-account plan detection (Fix 4,
/// docs/design/subscription-efficiency-routing.md). The single source every `SubscriptionQuota::
/// with_plans` call site goes through, so they cannot drift from each other: `live_quota` here,
/// `subagent::route_child`, `duel::run`, and the `forge mesh` / `forge models` inspector in
/// `forge-cli`'s `commands::models` (hence `pub`, not `pub(crate)`).
///
/// If you add a `with_plans` call site, route it through here. A site that passes
/// `config.mesh.subscriptions` directly renders `plan ?` for any surface whose plan is detected
/// rather than configured — which is exactly the D4 defect this function exists to fix.
/// Documented in docs/features/mesh-routing.md.
pub fn resolved_subscription_plans(config: &Config) -> std::collections::HashMap<String, String> {
    merge_subscription_plans(
        config.mesh.subscriptions.clone(),
        forge_provider::detect_subscription_plans(),
    )
}

/// [`resolved_subscription_plans`] enriched by the freshest server-observed Codex plan. The
/// store observation crosses process boundaries (`forge run` → `forge mesh` / TUI) and expires
/// with the shared Codex quota, so it can correct a stale JWT without becoming stale state itself.
pub fn resolved_subscription_plans_with_store(
    config: &Config,
    store: &forge_store::Store,
) -> std::collections::HashMap<String, String> {
    let mut plans = resolved_subscription_plans(config);
    if let Some(plan) = store.fresh_subscription_plan("codex-oauth") {
        plans.insert("codex-oauth".to_string(), plan.clone());
        plans.insert("codex-cli".to_string(), plan);
    }
    plans
}

#[cfg(test)]
mod subscription_plan_merge_tests {
    use super::merge_subscription_plans;
    use std::collections::HashMap;

    #[test]
    fn detected_overrides_config_but_config_only_keys_survive() {
        let config: HashMap<String, String> = [
            ("codex-cli".to_string(), "plus".to_string()),
            ("agy-cli".to_string(), "free".to_string()),
        ]
        .into_iter()
        .collect();
        let detected: HashMap<String, String> = [
            ("codex-cli".to_string(), "pro".to_string()),
            ("codex-oauth".to_string(), "plus".to_string()),
        ]
        .into_iter()
        .collect();

        let merged = merge_subscription_plans(config, detected);

        assert_eq!(
            merged.get("codex-cli"),
            Some(&"pro".to_string()),
            "detected wins"
        );
        assert_eq!(
            merged.get("agy-cli"),
            Some(&"free".to_string()),
            "config-only key survives"
        );
        assert_eq!(
            merged.get("codex-oauth"),
            Some(&"plus".to_string()),
            "detected-only key is added"
        );
    }

    #[test]
    fn empty_detected_keeps_config_untouched() {
        let config: HashMap<String, String> = [("claude-cli".to_string(), "max-20x".to_string())]
            .into_iter()
            .collect();

        let merged = merge_subscription_plans(config.clone(), HashMap::new());

        assert_eq!(merged, config);
    }

    #[test]
    fn empty_config_keeps_detected_untouched() {
        let detected: HashMap<String, String> = [("codex-oauth".to_string(), "plus".to_string())]
            .into_iter()
            .collect();

        let merged = merge_subscription_plans(HashMap::new(), detected.clone());

        assert_eq!(merged, detected);
    }
}

fn add_workspace_default_path(
    tool_name: &str,
    args: serde_json::Value,
    workspace: &std::path::Path,
) -> serde_json::Value {
    if !matches!(tool_name, "list_dir" | "search" | "glob" | "apply_patch") {
        return args;
    }
    let Some(mut object) = args.as_object().cloned() else {
        return args;
    };
    if !object.contains_key("path") && !object.contains_key("cwd") {
        let key = if tool_name == "apply_patch" {
            "cwd"
        } else {
            "path"
        };
        object.insert(
            key.to_string(),
            serde_json::Value::String(workspace.display().to_string()),
        );
    }
    serde_json::Value::Object(object)
}

fn normalize_workspace_target(path: &std::path::Path) -> std::path::PathBuf {
    let absolute = path.to_path_buf();
    let mut prefix = absolute.as_path();
    let mut tail = Vec::new();
    loop {
        if let Ok(real) = prefix.canonicalize() {
            let mut target = real;
            for component in tail.iter().rev() {
                target.push(component);
            }
            return target;
        }
        match prefix.parent() {
            Some(parent) => {
                if let Some(name) = prefix.file_name() {
                    tail.push(name.to_os_string());
                }
                prefix = parent;
            }
            None => return absolute,
        }
    }
}

fn validate_workspace_args(
    args: &serde_json::Value,
    workspace: &WorkspaceContext,
) -> Result<(), CoreError> {
    for key in ["path", "cwd"] {
        let Some(value) = args.get(key).and_then(serde_json::Value::as_str) else {
            continue;
        };
        let target = normalize_workspace_target(std::path::Path::new(value));
        if !target.starts_with(workspace.root()) {
            return Err(CoreError::Workspace(format!(
                "{key} escapes session workspace: {value}"
            )));
        }
    }
    if let Some(paths) = args.get("paths").and_then(serde_json::Value::as_array) {
        for value in paths.iter().filter_map(serde_json::Value::as_str) {
            let target = normalize_workspace_target(std::path::Path::new(value));
            if !target.starts_with(workspace.root()) {
                return Err(CoreError::Workspace(format!(
                    "path escapes session workspace: {}",
                    target.display()
                )));
            }
        }
    }
    Ok(())
}

impl Session {
    /// Run an Assay analysis over `source` (the bundled scope content), emit + persist the report,
    /// and — when `cleanup` — run a permission-gated, **undoable** fix turn (Refine) over the
    /// findings. The crew is read-only; Refine reuses the normal agent loop so its edits go through
    /// the permission broker and are shadow-snapshotted (so `/undo` reverts them).
    pub async fn assay(
        &mut self,
        source: Arc<str>,
        models: assay::TierModels,
        lenses: Vec<forge_types::FindingCategory>,
        scope: forge_types::AssayScope,
        cleanup: bool,
    ) -> Result<(), CoreError> {
        let pricing = Arc::new(self.pricing.clone());
        let lenses = if lenses.is_empty() {
            forge_types::FindingCategory::crew().to_vec()
        } else {
            lenses
        };
        let cooldown = std::time::Duration::from_secs(self.config.mesh.failover_cooldown_secs);
        let provider = Arc::clone(&self.provider);
        let store = Arc::clone(&self.store);

        // U8 — budget pre-estimate: scope down lenses to fit within remaining daily/monthly cap.
        let remaining_usd = {
            let (spent_today, _, spent_month) = self.store.spend_summary_usd().unwrap_or_default();
            let daily = self
                .config
                .mesh
                .daily_budget_usd
                .map(|cap| (cap - spent_today).max(0.0));
            let monthly = self
                .config
                .mesh
                .monthly_cap_usd
                .map(|cap| (cap - spent_month).max(0.0));
            match (daily, monthly) {
                (Some(d), Some(m)) => Some(d.min(m)),
                (Some(d), None) => Some(d),
                (None, Some(m)) => Some(m),
                (None, None) => None,
            }
        };
        let (lenses, dropped, estimated_cost) =
            assay::scope_to_budget(lenses, source.len(), &models, &pricing, remaining_usd);
        if dropped > 0 {
            self.presenter.emit(PresenterEvent::Warning(format!(
                "assay: estimated cost ~${estimated_cost:.3} exceeds remaining budget \
                 ${:.3} — dropped {dropped} expensive lens(es) to fit",
                remaining_usd.unwrap_or(0.0),
            )));
        }
        if lenses.is_empty() {
            self.presenter.emit(PresenterEvent::Warning(
                "assay: estimated cost exceeds remaining budget — \
                 add a free model or raise [mesh] daily_budget_usd / monthly_cap_usd"
                    .to_string(),
            ));
            return Ok(());
        }

        // Surface each critic/verifier as it finishes so the run shows live activity.
        let presenter = &mut self.presenter;
        let mut on_progress = |p: assay::AssayProgress| match &p {
            assay::AssayProgress::CriticQueued {
                lens,
                expected_model,
            } => {
                presenter.emit(PresenterEvent::AssayCriticRow(
                    forge_types::AssayCriticRow {
                        lens: lens.as_str().to_string(),
                        focus: assay::lens_brief(*lens).to_string(),
                        model: Some(expected_model.clone()),
                        cost_usd: 0.0,
                        output: String::new(),
                        status: forge_types::AssayCriticStatus::Queued,
                    },
                ));
            }
            assay::AssayProgress::CriticDone {
                lens,
                candidates,
                model,
                cost_usd,
                output,
            } => {
                presenter.emit(PresenterEvent::AssayCriticRow(
                    forge_types::AssayCriticRow {
                        lens: lens.as_str().to_string(),
                        focus: assay::lens_brief(*lens).to_string(),
                        model: Some(model.clone()),
                        cost_usd: *cost_usd,
                        output: output.clone(),
                        status: forge_types::AssayCriticStatus::Done {
                            candidates: *candidates,
                        },
                    },
                ));
            }
            assay::AssayProgress::CriticSkipped { lens, reason } => {
                presenter.emit(PresenterEvent::AssayCriticRow(
                    forge_types::AssayCriticRow {
                        lens: lens.as_str().to_string(),
                        focus: assay::lens_brief(*lens).to_string(),
                        model: None,
                        cost_usd: 0.0,
                        output: String::new(),
                        status: forge_types::AssayCriticStatus::Skipped {
                            reason: reason.clone(),
                        },
                    },
                ));
            }
            assay::AssayProgress::Verifying { candidates } => {
                presenter.emit(PresenterEvent::AssayVerifying {
                    candidates: *candidates,
                });
            }
            _ => {
                presenter.emit(PresenterEvent::AssayProgress(assay::progress_line(&p)));
            }
        };
        let mut report = assay::run_assay(
            scope,
            source,
            lenses,
            models,
            provider,
            pricing,
            store,
            cooldown,
            &mut on_progress,
        )
        .await;
        if let Ok(run_id) = self
            .store
            .create_assay_run(&report.scope.label(), report.cost_usd)
        {
            report.run_id = run_id.clone();
            for f in &report.findings {
                let _ = self.store.add_finding(&run_id, f);
            }
            // Auto-diff: compare against the prior run for this scope so users see what changed.
            if let Ok(Some(prev_id)) = self
                .store
                .latest_run_for_scope(&report.scope.label(), &run_id)
            {
                if let Ok(prev) = self.store.load_findings(&prev_id) {
                    let note =
                        assay_diff_note(&prev, &report.findings, &prev_id[..8.min(prev_id.len())]);
                    if !note.is_empty() {
                        self.presenter.emit(PresenterEvent::Warning(note));
                    }
                }
            }
        }
        self.presenter
            .emit(PresenterEvent::AssayReport(report.clone()));

        if cleanup && !report.findings.is_empty() {
            self.presenter.emit(PresenterEvent::Warning(format!(
                "⚒ Refine — fixing {} finding(s); edits are permission-gated, /undo to revert",
                report.findings.len()
            )));
            let prompt = refine_prompt(&report);
            self.run_turn(&prompt).await?; // emits its own Done
        } else {
            if cleanup {
                self.presenter.emit(PresenterEvent::Warning(
                    "nothing to clean up — no findings".into(),
                ));
            }
            self.presenter.emit(PresenterEvent::Done {
                final_text: String::new(),
                stop_reason: StopReason::FinalAnswer,
            });
        }
        Ok(())
    }

    /// Read the next user prompt from the attached surface. `None` ends the session.
    pub fn read_line(&mut self) -> Option<String> {
        self.presenter.read_line()
    }

    /// Surface a turn-level failure to the UI (an Error event + a Done marker) so the caller's
    /// loop ends the turn cleanly instead of leaving it hanging.
    ///
    /// Emits [`PresenterEvent::Error`], not [`PresenterEvent::Warning`]: every OTHER genuine
    /// turn-ending failure in this file (chain-exhausted, no-usable-model, empty-response
    /// give-up) already emits `Error` — this was the one caller that mislabeled a real failure as
    /// a mere warning. That mislabeling was a real, user-visible gap: the headless `forge serve`
    /// driver (`run/driver.rs`) specifically latches `PresenterEvent::Error` (not `Warning`) to
    /// detect "this turn ended in failure" for its Web Push trigger AND for pushing a
    /// remote-facing note (`Snapshot::notes`, what the mobile app renders as a toast) — so a turn
    /// that failed via THIS function (e.g. every model in the routed+fallback chain rejecting a
    /// vision-attached prompt) reached neither: no push, no toast, just a scrollback line easy to
    /// miss. `busy` itself always cleared correctly (that's driven independently by the turn
    /// task's completion, not by which presenter event fired) — the gap was purely "no visible
    /// error signal", not "stuck busy forever".
    pub fn notify_error(&mut self, msg: &str) {
        self.presenter.emit(PresenterEvent::Error(msg.to_string()));
        self.presenter.emit(PresenterEvent::Done {
            final_text: String::new(),
            stop_reason: StopReason::FinalAnswer,
        });
    }

    fn next_seq(&mut self) -> i64 {
        let n = self.seq;
        self.seq += 1;
        n
    }

    fn tool_specs(&self) -> Vec<ToolSpec> {
        let scope = self.task_scope.as_ref();
        let mut specs: Vec<ToolSpec> = self
            .tools
            .names()
            .filter(|name| scope.is_none_or(|scope| scope.permits_tool(name)))
            .filter_map(|name| self.tools.get(name))
            .map(|t| ToolSpec {
                name: t.name().to_string(),
                description: t.description().to_string(),
                schema: t.schema(),
            })
            .collect();
        // Advertise the subagent virtual tool to the top-level model (RFC
        // subagent-orchestration). Children may receive it separately only when the explicitly
        // configured recursion depth allows another generation.
        if self.config.mesh.subagents.enabled
            && self
                .task_scope
                .as_ref()
                .is_none_or(|scope| scope.permits_tool(subagent::SPAWN_AGENTS_TOOL))
        {
            specs.push(subagent::spawn_agents_spec(
                self.config.mesh.subagents.max_agents,
            ));
            // Follow-ups to already-spawned children (persistent subagents). Advertised beside
            // spawn_agents — a fresh session simply has no children yet and the tool says so.
            specs.push(subagent::send_to_agent_spec());
            specs.push(workflow::run_workflow_spec());
            // Retained async subagents (RFC retained-async-subagents): status + cancellation for
            // `spawn_agents(detached: true)` children. Advertised alongside spawn_agents — same
            // gate, same "empty until first use" story as send_to_agent above.
            specs.push(subagent::list_subagents_spec());
            specs.push(subagent::cancel_subagent_spec());
        }
        if self
            .task_scope
            .as_ref()
            .is_none_or(|scope| scope.permits_tool(ASK_USER_TOOL))
        {
            specs.push(ask_user_spec());
        }
        // The task-tracking tool — always advertised so the model can keep a live todo list.
        if self
            .task_scope
            .as_ref()
            .is_none_or(|scope| scope.permits_tool(UPDATE_TASKS_TOOL))
        {
            specs.push(update_tasks_spec());
        }
        // The on-demand memory tool — model calls this to persist a durable fact at any
        // point during a turn, not just via end-of-turn auto-capture.
        if self
            .task_scope
            .as_ref()
            .is_none_or(|scope| scope.permits_tool(REMEMBER_TOOL))
        {
            specs.push(remember_spec());
        }
        // Agent-created recurring re-entry prompts — distinct from the user's own `/heartbeat`.
        if self
            .task_scope
            .as_ref()
            .is_none_or(|scope| scope.permits_tool(heartbeat::MANAGE_HEARTBEATS_TOOL))
        {
            specs.push(heartbeat::manage_heartbeats_spec());
        }
        // The plan-presentation tool — offered ONLY in planning mode, so the model proposes a plan
        // (rendered as an interactive card) instead of editing. Gating it to Plan mode also makes
        // the approve→Auto-edit→build flow non-recursive (the build turn can't re-propose a plan).
        if self.mode == PermissionMode::Plan {
            specs.push(present_plan_spec());
        }
        // The skill-loading tool — advertised (with the available-skills list) only when a
        // non-empty catalog is attached, so the model can find + apply Forge's own skills.
        if let Some(cat) = &self.skills {
            if !cat.skill_listing().is_empty() {
                specs.push(use_skill_spec(cat));
            }
        }
        // Fleet agent-to-agent messaging — advertised only when this session is hosted by
        // forge serve and wired to a live daemon fleet (ADR-0004: the callback is the only thing
        // core knows about the daemon; a bare `forge run`/`forge chat` session never sees it).
        if self.fleet.is_some()
            && self
                .task_scope
                .as_ref()
                .is_none_or(|scope| scope.permits_tool(fleet::MESSAGE_SESSION_TOOL))
        {
            specs.push(fleet::message_session_spec());
        }
        // External MCP servers: the meta-tools (search/expose/resources/prompt) + any exposed
        // server tools (deferred loading keeps this bounded). Empty unless servers are connected.
        if let Some(mcp) = &self.mcp {
            specs.extend(mcp.advertised_specs().into_iter().map(|s| ToolSpec {
                name: s.name,
                description: s.description,
                schema: s.schema,
            }));
        }
        // ToolRegistry is backed by a HashMap, whose iteration order is deliberately randomized
        // per process. A resumed `forge run` therefore used to reshuffle the complete tool-schema
        // prefix even when the session, model, and tools were unchanged. Provider prompt caches
        // hash the serialized prefix, so that harmless ordering drift forced a full cache miss on
        // every cross-process resume. Sort the final combined surface (built-ins, virtual tools,
        // skills, and MCP) so every provider receives a byte-stable catalog.
        specs.sort_by(|left, right| left.name.cmp(&right.name));
        specs
    }

    /// Whether this turn should expose callable tools to the routed model.
    ///
    /// Kept deliberately conservative: standard/complex turns retain the full agent surface;
    /// a trivial turn only receives tools when the prompt has a clear workspace or external-action
    /// intent. This prevents small local models from interpreting a requested answer token as a
    /// function name while preserving tool access for genuine simple file/command tasks.
    fn should_advertise_tools(tier: TaskTier, prompt: &str) -> bool {
        if tier != TaskTier::Trivial {
            return true;
        }
        let prompt = prompt.to_ascii_lowercase();
        [
            "read ",
            "inspect ",
            "search ",
            "find ",
            "grep",
            "rg ",
            "file",
            "directory",
            "repo",
            "code",
            "test",
            "build",
            "compile",
            "run ",
            "execute",
            "shell",
            "command",
            "git",
            "commit",
            "diff",
            "write",
            "edit",
            "create",
            "delete",
            "implement",
            "fix ",
            "debug",
            "diagnose",
            "review",
            "refactor",
            "install",
            "web",
            "http",
            "url",
            "browser",
            "fetch",
            "download",
            "mcp",
            "database",
            "query",
        ]
        .iter()
        .any(|intent| prompt.contains(intent))
    }

    /// Run one full turn: route -> (model -> tools)* -> final answer. Returns the outcome.
    pub async fn run_turn(&mut self, prompt: &str) -> Result<LoopOutcome, CoreError> {
        self.run_turn_with(prompt, &[], None).await
    }

    /// Compact the live context: summarize the older messages (everything but the most recent
    /// `COMPACT_KEEP_RECENT`) into a single system message via a cheap model call, shrinking what
    /// subsequent turns send to the model. In-memory only — the full transcript stays in the store
    /// for audit/resume (persisting the compacted view across resume is a follow-up). No-op when
    /// the transcript is already short. Returns `(messages_before, messages_after)`.
    /// Inject command/skill guidance as persisted system messages *without* a model call — for
    /// `/skill <name>` with no prompt, so the methodology primes the next turn the user types.
    pub fn prime_guidance(&mut self, guidance: &[String]) -> Result<(), CoreError> {
        for g in guidance {
            let gseq = self.next_seq();
            self.store
                .add_message(&self.id, gseq, Role::System, g, None)?;
            self.transcript.push(Message::system(g));
        }
        Ok(())
    }

    /// Load the persisted replay entries for any session (not just this one) — used by the
    /// `/replay` chat command to show a transcript inline.
    pub fn load_replay(
        &self,
        session_id: &str,
    ) -> Result<Vec<forge_store::ReplayEntry>, CoreError> {
        self.store.load_replay(session_id).map_err(CoreError::Store)
    }

    /// Resolve a session-id prefix to full ids — allows `/replay abc` to find `abc123…`.
    pub fn matching_session_ids(&self, prefix: &str) -> Result<Vec<String>, CoreError> {
        self.store
            .matching_session_ids(prefix)
            .map_err(CoreError::Store)
    }

    /// Run the completion-verification gate for a turn that reported every tracked task Done.
    /// Emits the user-facing warning, pushes the verify nudge on [`CompletionGate::Reverify`], and
    /// returns the decision so the caller can `continue` (re-verify) or fall through (accept). Both
    /// the CLI-bridge and direct-API paths call this, so the completion authority can't diverge.
    fn run_completion_gate(
        &mut self,
        verify_attempts: &mut usize,
        did_real_work: bool,
        no_change_required: bool,
        inspected_this_turn: bool,
        unresolved_checks: Option<&str>,
    ) -> PostCheckDecision {
        let max_verify_attempts = CompletionContract::production().max_observation_requests();
        // Tool-name-neutral so the SAME nudge works for the bridge (tools are `mcp__forge__*`) and
        // the direct path (`shell`/`read_file`) — the model maps "run a shell command / read a file"
        // to whichever names its toolset exposes.
        let verify_nudge = match unresolved_checks {
            Some(checks) => format!(
                "You reported every task Done, but the latest {checks} check failed. Re-run the \
                 matching {checks} command and fix it until that check succeeds. File reads, `ls`, \
                 `cat`, and `git diff` do NOT clear a failed check. Re-marking the task list is not \
                 verification. Only after the matching check succeeds, state exactly what passed."
            ),
            None => "You reported every task Done. Before this turn can end, prove it with the \
                 relevant build, typecheck, lint, or test command. If no such check applies, inspect \
                 the exact artifact or external state the task changed. Re-marking the task list is \
                 not verification. If the check shows a gap, reopen the task and finish it; otherwise \
                 state exactly what passed."
                .to_string(),
        };
        let intent = self
            .task_scope
            .as_ref()
            .map(|scope| scope.contract.intent())
            .unwrap_or(TaskIntent::Mutating);
        let decision = post_check_decision(
            intent,
            *verify_attempts,
            did_real_work,
            no_change_required,
            inspected_this_turn,
        );
        match decision {
            PostCheckDecision::RequestObservation => {
                *verify_attempts += 1;
                let pending = unresolved_checks
                    .map(|checks| format!("; unresolved: {checks}"))
                    .unwrap_or_default();
                self.presenter.emit(PresenterEvent::Warning(format!(
                    "all tasks reported done — verifying with a real state check before finishing ({}/{max_verify_attempts}){pending}",
                    *verify_attempts
                )));
                let nseq = self.next_seq();
                let _ = self
                    .store
                    .add_message(&self.id, nseq, Role::System, &verify_nudge, None);
                self.transcript.push(Message::system(verify_nudge));
            }
            PostCheckDecision::AcceptNoArtifacts => {
                self.presenter.emit(PresenterEvent::Warning(
                    "completion not tool-verified (no external artifacts to check) — accepting the reported result"
                        .to_string(),
                ));
            }
            PostCheckDecision::AcceptUnverified => {
                self.presenter.emit(PresenterEvent::Warning(
                    "completion could NOT be tool-verified — the model reported done without \
                     inspecting real state. Treat this result as UNVERIFIED."
                        .to_string(),
                ));
            }
            PostCheckDecision::AcceptClean => {}
        }
        decision
    }

    /// Like [`Session::run_turn`], but first prepends `guidance` (an invoked command's or
    /// skill's methodology) as persisted system messages, and biases routing with an optional
    /// `tier_override` (the command/skill `tier:` hint). `run_turn(p)` is exactly
    /// `run_turn_with(p, &[], None)` — the agent loop, tools, permission broker, pricing and
    /// persistence are otherwise unchanged.
    pub async fn run_turn_with(
        &mut self,
        prompt: &str,
        guidance: &[String],
        tier_override: Option<TaskTier>,
    ) -> Result<LoopOutcome, CoreError> {
        self.poll_lattice_watcher();
        // A TUI/serve driver can remain alive while retention prunes its previously empty parent
        // row. Every subsequent persistence write references this id, so restore that minimal
        // parent before routing, command guidance, or the prompt can touch the transcript.
        let mode = format!("{:?}", self.mode);
        self.store
            .ensure_session(&self.id, &self.workspace.display(), &mode)?;
        // Retained async subagents (RFC retained-async-subagents, ADR-0004): deliver any detached
        // child that finished since the last turn as a labeled turn in THIS turn's context, before
        // routing/the prompt — so the model sees it as part of what it's responding to. Delivery
        // goes through the ordinary session queue (a persisted message + transcript push), not a
        // new presenter surface.
        self.deliver_pending_detached_results()?;
        // A completed turn's bulky tool logs remain available in the store/replay and in the
        // working tree they inspected, but repeatedly resending them on every later model step
        // makes long sessions grow quadratically. Reclaim them at every user-turn boundary instead
        // of waiting until the model is already near context exhaustion. The newest messages stay
        // verbatim; current-turn tool results are added only after this boundary.
        let _ = prune_and_inject(&mut self.transcript, COMPACT_KEEP_RECENT);
        let recap_tasks_before = self.tasks.clone();
        let working_tree_baseline = working_tree_status(Some(self.workspace.root()));
        self.last_context_pack = context_pack::ContextPack::default();
        let mut context_pack = context_pack::ContextPack::default();
        // 1. Route the task (deterministic, no model call) and record why. The budget is
        // aggregated across ALL sessions for the current local day + week + month (FR-5), not one
        // session's running total. One combined query instead of three separate ones.
        let (spent_today, spent_week, spent_month) = self.store.spend_summary_usd()?;
        let budget = BudgetState {
            spent_today_usd: spent_today,
            daily_cap_usd: self.config.mesh.daily_budget_usd,
            spent_week_usd: spent_week,
            weekly_cap_usd: self.config.mesh.weekly_budget_usd,
            spent_month_usd: spent_month,
            monthly_cap_usd: self.config.mesh.monthly_cap_usd,
            warn_fraction: self.config.mesh.warn_threshold,
            min_context_tokens: Some(self.routing_min_context()),
        };
        let status = budget.status();

        // Hard stop: once a cap is exceeded, refuse the call before any provider request
        // (the cap is never silently exceeded). Overridable per process via
        // FORGE_BUDGET_OVERRIDE=1.
        if status == BudgetStatus::Exhausted
            && self.config.mesh.budget.hard_stop
            && !budget_override_active()
        {
            let msg = over_budget_message(&budget);
            self.presenter.emit(PresenterEvent::Warning(msg.clone()));
            // Persist the prompt + a system note, make NO provider call, write NO usage row.
            let seq = self.next_seq();
            self.store
                .add_message(&self.id, seq, Role::User, prompt, None)?;
            self.transcript.push(Message::user(prompt));
            // UI-only: the note ends the turn for the USER; a model resuming this session gains
            // nothing from stale budget chrome in its prompt.
            let seq = self.next_seq();
            self.store.add_ui_note(&self.id, seq, Role::System, &msg)?;
            self.transcript.push(Message::system(&msg).ui_only());
            self.presenter.emit(PresenterEvent::Done {
                final_text: msg.clone(),
                stop_reason: StopReason::BudgetExhausted,
            });
            return Ok(LoopOutcome::budget_exhausted(msg));
        }

        // Surface budget pressure before routing (FR-5).
        match status {
            BudgetStatus::Warning => self.presenter.emit(PresenterEvent::Warning(format!(
                "approaching budget cap (today ${:.4}, month ${:.4})",
                budget.spent_today_usd, budget.spent_month_usd
            ))),
            BudgetStatus::Exhausted => self.presenter.emit(PresenterEvent::Warning(format!(
                "budget cap reached (today ${:.4}) — routing to the cheapest tier",
                budget.spent_today_usd
            ))),
            BudgetStatus::Ok => {}
        }

        // Route around any currently-benched models (failover): the snapshot excludes models
        // whose cooldown hasn't elapsed, even across restarts (docs/features/mesh-routing.md).
        let readiness = self.provider_readiness();
        let health = readiness.health;
        // Quota-aware routing (L3): demote/skip a subscription that the bridge reported is near or
        // over its plan limit (recorded after earlier turns from the CLI's rate-limit events).
        let quota = readiness.quota;
        // A per-turn `tier_override` (command/skill `tier:` hint) wins; otherwise the in-session
        // tier pin (set by the `tier_up`/`tier_down` keybinds) biases routing.
        let effective_tier = tier_override.or(self.pinned_tier);
        // Whether THIS turn has image attachments queued (vision input) — route around a
        // text-only model so an image doesn't land on a provider that 404s on it.
        let has_images = !self.pending_images.is_empty();
        // Routing happens before the current user message is appended, so the transcript is
        // exactly the bounded prior-turn context needed to interpret "continue", "fix that", etc.
        let routing_context = RoutingContext::from_messages(&self.transcript)
            .with_session_affinity(
                self.route_affinity.clone(),
                self.estimated_reusable_prefix_tokens(),
            );
        let decision = match self.pinned_model.as_deref() {
            // `/model <id>`/`/model a,b` override: route restricted to the pinned set. The mesh
            // still classifies (for tier stats) but the actual call + failover chain stay within
            // the set — a single pin is strict (no cross-model fallback), a set ranks within
            // itself and fails over only to other set members.
            Some(pin) => {
                self.router
                    .route_with_pin_set(
                        pin,
                        prompt,
                        has_images,
                        budget,
                        &health,
                        &quota,
                        self.pinned_effort,
                        &self.project,
                    )
                    .await
            }
            None => {
                self.router
                    .route_contextual(
                        prompt,
                        has_images,
                        budget,
                        &health,
                        &quota,
                        effective_tier,
                        self.pinned_effort,
                        &self.project,
                        &routing_context,
                    )
                    .await
            }
        };
        let routed_model = decision.model.clone();
        let reuse_response_chain = should_reuse_response_chain(
            prompt,
            &routing_context,
            self.route_affinity.as_ref(),
            &routed_model,
        );

        // No usable model: the router filters unkeyed models out of the chain (is_usable →
        // has_api_key), so the routed pick belongs to a key-needing provider with no key ONLY when
        // nothing usable existed at all — the built-in defaults lead with groq, so a user whose keys
        // are for other providers (or whose auto-discovery came up empty) would otherwise watch the
        // mesh call groq and auth-fail on EVERY turn. Stop with an actionable diagnostic instead of
        // spinning on a key we don't have. (Keyless providers — ollama, the claude/codex bridges,
        // unknown prefixes — return has_api_key=true and pass through untouched.)
        if !forge_config::has_api_key(forge_config::provider_of(&routed_model)) {
            let msg = no_usable_model_message(&routed_model);
            self.presenter.emit(PresenterEvent::Error(msg.clone()));
            let seq = self.next_seq();
            self.store
                .add_message(&self.id, seq, Role::User, prompt, None)?;
            self.transcript.push(Message::user(prompt));
            // UI-only, same reasoning as the budget-stop note above.
            let seq = self.next_seq();
            self.store.add_ui_note(&self.id, seq, Role::System, &msg)?;
            self.transcript.push(Message::system(&msg).ui_only());
            self.presenter.emit(PresenterEvent::Done {
                final_text: msg.clone(),
                stop_reason: StopReason::FinalAnswer,
            });
            return Ok(LoopOutcome::final_answer(msg));
        }

        self.presenter.emit(PresenterEvent::Routing {
            tier: decision.tier.as_str().to_string(),
            model: routed_model.clone(),
            rationale: decision.rationale.clone(),
        });

        // Prepend any command/skill guidance as persisted system messages, so the methodology
        // is in context for this turn and rehydrates verbatim on resume (the skill file is not
        // re-read).
        for g in guidance {
            self.inject_context(
                &mut context_pack,
                context_pack::ContextSource::CommandGuidance,
                "invoked command or skill guidance",
                g,
            )?;
        }

        // White-hot effort (docs/features/whitehot-effort.md): xhigh reasoning PLUS a standing
        // orchestration instruction — injected once per pin (set_effort re-arms on change) so
        // repeated turns don't accumulate identical blocks.
        if self.pinned_effort == Some(EffortLevel::WhiteHot) && !self.whitehot_guidance_injected {
            self.inject_context(
                &mut context_pack,
                context_pack::ContextSource::Workflow,
                "whitehot effort workflow",
                workflow::WHITEHOT_GUIDANCE,
            )?;
            self.whitehot_guidance_injected = true;
        }

        // Inject the project AGENTS.md as a standing system prompt on the first turn of a fresh
        // session. The file was read ONCE at session construction (sync `std::fs`, off the async
        // executor — see `build`) into `cached_agents_md`, so this use-site is await-free AND does
        // no blocking syscall: an abort() between here and the user-message persistence below must
        // not skip the recording (`aborting_a_running_turn_releases_the_session_lock` pins this),
        // and a `tokio::fs` read here would deterministically reintroduce that cancel window on the
        // current-thread runtime (the blocking-pool completion doesn't promptly unpark the driver,
        // so the abort lands on the parked read before persistence runs).
        if !self.project_prompt_injected {
            self.project_prompt_injected = true;
            if let Some(body) = self.cached_agents_md.take() {
                self.inject_context(
                    &mut context_pack,
                    context_pack::ContextSource::ProjectInstructions,
                    "project AGENTS.md",
                    &body,
                )?;
            }

            // Auto-memory RECALL: surface the few durable facts from past sessions in this project
            // most relevant to the prompt (preferences/decisions/conventions). The edge over a
            // dump-everything memory file: only the RELEVANT memories are injected, ranked by the
            // prompt then salience + recency. Once per session, like AGENTS.md.
            if self.config.mesh.auto_memory {
                let scope = memory_scope_at(self.workspace.root());
                let recalled = match embed_one(&self.config.lattice.embeddings, prompt).await {
                    Some(qemb) => self.store.recall_semantic(&scope, &qemb, 6),
                    None => self.store.recall_memories(&scope, prompt, 6),
                };
                if let Ok(mems) = recalled {
                    if !mems.is_empty() {
                        let mut block = String::from(
                            "Remembered from earlier sessions in this project (durable facts — \
                             use them, and don't re-ask what's already settled):\n",
                        );
                        for m in &mems {
                            block.push_str(&format!("- [{}] {}\n", m.kind, m.text));
                        }
                        self.inject_context(
                            &mut context_pack,
                            context_pack::ContextSource::Memory,
                            "relevant durable project memories",
                            &block,
                        )?;
                        // Emit a one-line presenter note so the user sees recall happened.
                        self.presenter.emit(PresenterEvent::Warning(format!(
                            "💭 recalled {} memories from past sessions",
                            mems.len()
                        )));
                    }
                }
            }

            // Continual Harness context injection (refinement.rs): surface the learned
            // prompt/skill/subagent entries this agent has proposed about itself in past
            // sessions, right alongside auto-memory recall above — same one-shot-per-session
            // placement, same "durable context from earlier work" spirit, but scoped to how
            // Forge itself should operate rather than facts about the project.
            if self.config.harness.enabled {
                if let Ok(overview) = self.harness_overview() {
                    if let Some(block) = context_pipeline::harness_context_block(
                        &overview,
                        self.config.harness.max_context_entries,
                        self.config.harness.max_entry_chars,
                    ) {
                        self.inject_context(
                            &mut context_pack,
                            context_pack::ContextSource::Harness,
                            "learned harness context (Continual Harness)",
                            &block,
                        )?;
                    }
                }
            }

            // Auto-orchestrate: inject the resource-routing framework once so the model surveys
            // all available tools on every turn without requiring the user to /orchestrate.
            if self.config.mesh.auto_orchestrate {
                let guidance = forge_skills::orchestrate_system_guidance();
                self.inject_context(
                    &mut context_pack,
                    context_pack::ContextSource::Orchestration,
                    "automatic orchestration guidance",
                    guidance,
                )?;
            }

            // When git co-authoring is on, prime the agent (once) to attribute its work to Forge.
            // Commit trailers are stamped deterministically by the prepare-commit-msg hook; this
            // covers the PR body (which no hook can reach) and tells the model not to add other
            // co-author lines that the hook would only strip.
            if self.config.git.coauthor {
                const GIT_ATTRIBUTION: &str = "Git attribution is enabled for this session. When \
you create commits or pull requests, attribute them to Forge:\n\
- Commits: a `Co-Authored-By: Forge <forge@adulari.dev>` trailer is added automatically by a git \
hook — do NOT add Claude/Codex/Anthropic co-author lines yourself.\n\
- Pull requests: include a line in the PR body crediting Forge, e.g. `🔨 Created with Forge`.";
                self.inject_context(
                    &mut context_pack,
                    context_pack::ContextSource::Attribution,
                    "git co-author attribution enabled",
                    GIT_ATTRIBUTION,
                )?;
            }
        }

        // Reset the per-turn edit counter so the autofix stage only fires when THIS turn wrote
        // something (not a carry-over from a prior turn).
        self.edits_this_turn = 0;
        self.failure_tracker.reset_turn();
        self.env_fight = EnvFightTracker::default();

        // 2. Persist + record the user message. Its seq keys this turn's code-snapshot dir
        // (PR3): files written during the turn are restorable by rewinding to this boundary.
        let contract =
            turn_contract::TurnContract::derive(prompt, self.mode, self.expect_code_change);
        self.last_turn_contract = contract.clone();
        if let Some(guidance) = contract.public_api_guidance() {
            self.inject_context(
                &mut context_pack,
                context_pack::ContextSource::TurnContract,
                "public-API preservation contract",
                guidance,
            )?;
        }
        let pristine_tests_required = prompt_requires_pristine_existing_tests(prompt)
            || session_requires_pristine_existing_tests(&self.transcript);
        let pristine_guidance_present = self.transcript.iter().any(|message| {
            message.role == Role::System && message.content == PRISTINE_TEST_GUIDANCE
        });
        if pristine_tests_required && !pristine_guidance_present {
            self.inject_context(
                &mut context_pack,
                context_pack::ContextSource::TurnContract,
                "pristine existing-test contract",
                PRISTINE_TEST_GUIDANCE,
            )?;
        }
        let fault_seam_required = prompt_requires_fault_seam_audit(prompt);
        let fault_seam_guidance_present = self
            .transcript
            .iter()
            .any(|message| message.role == Role::System && message.content == FAULT_SEAM_GUIDANCE);
        if fault_seam_required && !fault_seam_guidance_present {
            self.inject_context(
                &mut context_pack,
                context_pack::ContextSource::TurnContract,
                "production fault-injection seam contract",
                FAULT_SEAM_GUIDANCE,
            )?;
        }
        if let Some(guidance) = contract.guidance() {
            self.inject_context(
                &mut context_pack,
                context_pack::ContextSource::TurnContract,
                "explicit turn completion contract",
                guidance,
            )?;
        }
        if self.config.mesh.verify_completeness && !forge_provider::is_cli_bridge(&routed_model) {
            if direct_completeness_is_identifier_migration(prompt) {
                self.inject_context(
                    &mut context_pack,
                    context_pack::ContextSource::TurnContract,
                    "identifier-migration production scope",
                    DIRECT_IDENTIFIER_MIGRATION_SCOPE_GUIDANCE,
                )?;
            } else {
                let named_apis = direct_scope_guidance_named_apis(prompt);
                if !named_apis.is_empty() {
                    let guidance = format!(
                        "{DIRECT_NAMED_API_SCOPE_GUIDANCE}\n\nNamed production APIs:\n- {}",
                        named_apis.join("\n- ")
                    );
                    self.inject_context(
                        &mut context_pack,
                        context_pack::ContextSource::TurnContract,
                        "explicit named-API implementation scope",
                        &guidance,
                    )?;
                }
            }
        }
        let seq = self.next_seq();
        self.current_turn_seq = seq;
        self.task_scope = Some(TaskScope::for_turn(
            prompt,
            contract,
            self.mode,
            seq,
            Some(self.workspace.root().to_path_buf()),
            git_head(Some(self.workspace.root())),
            self.id.clone(),
            seq as u64,
        ));
        self.store
            .add_message(&self.id, seq, Role::User, prompt, None)?;
        // Attach any images queued for this turn (vision). They ride on the in-memory transcript
        // for the provider call; the persisted row stays text-only (images are transient input).
        let images = std::mem::take(&mut self.pending_images);
        if images.is_empty() {
            self.transcript.push(Message::user(prompt));
        } else {
            self.transcript
                .push(Message::user_with_images(prompt, images));
        }
        // Auto-checkpoint at the turn boundary, labeled with the prompt preview, so `/undo` can
        // offer a list of past messages to rewind to (no manual /checkpoint needed).
        let _ = self
            .store
            .add_checkpoint(&self.id, Some(&checkpoint_preview(prompt)), seq);
        // This turn's snapshot context (so a CLI-bridge model's file edits, which run in
        // `forge mcp-serve`, a separate process, get snapshotted into THIS turn's dir and are
        // restorable by `/undo`) is built lazily by `checkpoint_context()` and handed to each bridge
        // completion via `CompletionOptions::checkpoint` — no process-global env mutation here. The
        // in-process tool path snapshots directly in `invoke_tool`.

        // Refresh the cached git branch for this turn's env preamble via `tokio::fs` (non-blocking),
        // keeping it current if the branch changed since the last turn. Done HERE — after the user
        // message is persisted — so this await cannot reopen the abort-before-persist window the
        // synchronous-read invariant protects; `system_preamble` (called per model-loop step below)
        // then reads the cached field with no syscall and no `.await`, staying `Send`.
        self.cached_git_branch = tokio::fs::read_to_string(self.workspace.root().join(".git/HEAD"))
            .await
            .ok()
            .as_deref()
            .and_then(parse_git_head);

        // ★ Auto-retrieve relevant code from the Lattice index and inject it as a system message
        // before the first provider call (code-intelligence.md §5.1). Retrieve into an owned value
        // first so the `&self.lattice` borrow is released before we mutate the transcript. The
        // budget shrinks with budget pressure — context spend follows the same discipline as model
        // spend. Empty index / disabled / any error → nothing injected, turn runs as before.
        // Skipped when the routed model is a CLI bridge: claude/codex explore with their OWN
        // agent loop, so the injected snippets are duplicated context they re-ingest every turn
        // of that loop on top of their own reads.
        let injected = {
            if let Some(lat) = self.lattice.as_ref().filter(|_| {
                self.config.lattice.inject && !forge_provider::is_cli_bridge(&routed_model)
            }) {
                let budget = inject_budget(self.config.lattice.inject_token_budget, status);
                let emb = &self.config.lattice.embeddings;
                // Body injection (the big token-saving lever): inject the top hits' full source so
                // the model reads them from context instead of spending a whole-file `read_file`.
                let bodies = self
                    .config
                    .lattice
                    .inject_bodies
                    .then_some(forge_index::BodyOpts {
                        max_tokens: self.config.lattice.body_max_tokens,
                        max_hits: self.config.lattice.inject_body_hits,
                    });
                // Hybrid: blend embedding neighbours of the prompt with structural hits. The
                // backend is chosen by config (auto-picks the cheapest available); any backend
                // error degrades to structural inside `retrieve_hybrid`. No backend → structural.
                match forge_provider::select_embedder(emb) {
                    Some((embedder, _)) => lat
                        .retrieve_hybrid(prompt, budget, bodies, embedder.as_ref())
                        .await
                        .ok(),
                    None => lat.retrieve_async(prompt, budget, bodies).await.ok(),
                }
            } else {
                None
            }
        }
        .filter(|ctx| !ctx.is_empty());
        if let Some(ctx) = injected {
            let files = ctx
                .snippets
                .iter()
                .map(|s| s.rel_path.as_str())
                .collect::<std::collections::HashSet<_>>()
                .len();
            let symbols = ctx.nodes.len();
            let tokens = ctx.est_tokens;
            let body = ctx.render();
            self.inject_context(
                &mut context_pack,
                context_pack::ContextSource::Lattice,
                "relevant code retrieval",
                &body,
            )?;
            self.presenter.emit(PresenterEvent::ContextInjected {
                symbols,
                files,
                tokens,
            });
        }
        self.last_context_pack = context_pack;

        // ── Architect plan phase (architect_mode) ────────────────────────────────────────────────
        // When enabled: call the strong planner model with NO tools advertised; append its plan to
        // the transcript as a persisted assistant message so the editor model sees it below. When
        // disabled (the default) `run_plan` returns Ok(None) immediately — this block is a no-op.
        if let Some(_plan) = self.run_plan().await? {
            // The plan is already in self.transcript (pushed inside run_plan). Nothing else to do
            // here; the editor phase below will see it as the last assistant message in context.
        }

        // Determine the model for the edit phase.  In architect mode the editor model takes over;
        // otherwise we keep the mesh-routed model unchanged.
        let edit_model = if self.config.mesh.architect_mode {
            let editor = self.resolve_editor_model();
            self.presenter.emit(PresenterEvent::Routing {
                tier: decision.tier.as_str().to_string(),
                model: editor.clone(),
                rationale: "architect edit phase".to_string(),
            });
            // Keep the gauge's model + limit in lockstep: emit the edit model's window now, else the
            // limit stays stuck on the plan-phase model (the "1050k under a glm editor" bug) because
            // a short edit-phase transcript that fits never triggers auto_compact's gauge emit.
            self.emit_context_gauge(&editor);
            editor
        } else {
            routed_model.clone()
        };

        // Silent auto-compaction: if the conversation has grown past ~80% of the routed model's
        // (fetched/heuristic) context window, summarize older messages now so the turn doesn't ride
        // the hard-trim floor and lose recent context. Transparent — `compact` emits its own note.
        self.auto_compact_if_needed(&edit_model).await;

        let specs = if Self::should_advertise_tools(decision.tier, prompt) {
            self.tool_specs()
        } else {
            Vec::new()
        };
        let stream_idle = std::time::Duration::from_secs(self.config.mesh.stream_idle_timeout_secs);

        // 3. Model <-> tool loop. The cap is a runaway guard, not a functional limit — the loop
        // ends naturally when the model stops calling tools.
        let max_steps = self.config.mesh.max_steps.max(1);

        // Primary turn: pass the routing decision so failover, step-0 routing record, and quota
        // hints are all active — EXCEPT when architect mode swapped in a different editor model. The
        // routed `decision` describes the ROUTED model's failover chain (ranked for a different
        // model/tier); reusing it here would fail an editor-model error over to nonsensical
        // fallbacks. Match the self-review / autofix re-runs and run without a decision (no
        // cross-model failover) when the model was switched.
        let primary_decision = if edit_model == routed_model {
            Some(&decision)
        } else {
            None
        };
        let primary_transcript_start = self.transcript.len();
        let outcome = self
            .run_model_loop(
                edit_model,
                &specs,
                primary_decision,
                reuse_response_chain,
                max_steps,
                stream_idle,
            )
            .await?;
        let turn_tools_ran = outcome.tools_ran;
        let mut final_text = outcome.final_text;
        let mut context_tokens = outcome.context_tokens;
        let mut active_model = outcome.active_model;
        let mut hit_step_cap = outcome.hit_step_cap;
        let mut halted_by_loop_guard = outcome.halted_by_loop_guard;
        // Wave 7: did ANY bridge completion this turn (primary or a guard re-drive) report that
        // `mcp-serve` failed to start? OR-ed across the re-drives below, then combined with the
        // final tree/tool state to classify a toolless-bridge turn (see below the guards).
        let mut saw_mcp_unavailable = outcome.mcp_tools_unavailable;

        // A CLI-bridge model proposed a plan (the sink already rendered the card). Seed tasks,
        // persist it, and stash it for the approval flow below — the in-process path did this in
        // the `present_plan` handler already.
        if let Some(plan) = outcome.plan {
            self.ingest_plan(plan);
        }

        // Ran the full step budget while the model still wanted tools: pause loudly instead of
        // ending silently mid-task (the #1 "stops responding" bug). The work so far is persisted,
        // so the user can resume by sending `continue`.
        if outcome.hit_step_cap {
            self.presenter.emit(PresenterEvent::Warning(format!(
                "reached the {max_steps}-step limit — turn paused mid-task; send `continue` to keep going \
                 (raise `mesh.max_steps` in config to allow longer turns)"
            )));
        }

        // A CLI-bridge turn may have called `update_tasks` inside `forge mcp-serve` (a separate
        // process), persisting to the store but not touching our in-memory list. Reload and
        // surface it so bridge-driven task updates show in the TUI (the in-process path already
        // emitted live during the turn, so this is a no-op there).
        // Guard: only adopt the store's copy when it has tasks. A bridge that persisted under a
        // different db path/session leaves `persisted` empty — without this, an empty reload would
        // wipe the list we already surfaced live and hide the panel at turn end.
        if let Ok(persisted) = self.store.tasks(&self.id) {
            if !persisted.is_empty() && persisted != self.tasks {
                self.tasks = persisted;
                self.presenter
                    .emit(PresenterEvent::Tasks(self.tasks.clone()));
            }
        }

        // ── Token-budget continuation guard (H8) — empty-diff pushback with a diminishing-returns
        //    stop ────────────────────────────────────────────────────────────────────────────────
        // Code-change contracts (bench marks a whole session via `set_expect_code_change`; an
        // explicit interactive directive such as "fix ..." creates one for that turn) sometimes
        // end with the model having WORKED (tools ran)
        // but changed NOTHING — a `codex-cli::gpt-5.5` SWE-bench Lite sweep submitted 8/15 EMPTY
        // patches (raw codex solved 4 of those), the model describing the fix instead of making it.
        // Push back with a synthetic user message demanding the implementation, then re-drive — and
        // keep re-driving while there's budget headroom and the turn still made no progress, so a
        // single "still describing" reply after one nudge isn't silently accepted (the old one-shot
        // gave up too early). STOP the moment it turns into a spiral: [`continuation_decision`] halts
        // once the model has "continued" ≥ `CONTINUATION_DIMINISHING_MIN` times while emitting almost
        // nothing (`< CONTINUATION_DIMINISHING_TOKEN_FLOOR` tokens of growth), or hits the absolute
        // `CONTINUATION_MAX` ceiling — an honest surfaced halt, never an infinite loop.
        //
        // BRIDGE-AWARE progress compares the real git tree with the start-of-turn baseline, which
        // reflects a CLI bridge's `mcp-serve` edits without counting Forge's pre-existing worktree
        // scaffolding. A bridge that edited a file is accepted; one that only described is nudged.
        // old wave-6 gate counted sink `StreamEvent::ToolStarted`, but explicit mutating contracts
        // now cover even a zero-tool plan-only response. Pairs with compaction: each nudge compacts
        // first if the transcript is near the window, so the re-drive has room to work. Runs BEFORE
        // self-review/autofix so any edits it produces are still lint/test-checked.
        // A direct model that only emits a plan has `turn_tools_ran == 0`; that is still an empty
        // implementation for an explicit mutating contract and must be re-driven. The contract is
        // the safety gate, not prior tool activity. `mesh.nudge_empty_diff = false` disables it.
        if self.config.mesh.nudge_empty_diff
            && (self.expect_code_change || self.last_turn_contract.requires_changed_artifact())
            && self.edits_this_turn == 0
            && !halted_by_loop_guard
        {
            let mut continuation_count = 0usize;
            // No prior continuation yet: a sentinel that keeps the diminishing-returns stop from
            // firing on the first check (it is only consulted once `continuation_count` is high).
            let mut delta_tokens_last = u64::MAX;
            loop {
                // Past the soft deadline there is no budget for a re-drive (and the re-entered loop
                // would end immediately, clobbering the final answer with an empty one).
                if self.past_turn_deadline() {
                    break;
                }
                // Bridge-aware progress: the working tree reflects direct-path AND bridge edits.
                let made_progress = working_tree_changed_since(
                    Some(self.workspace.root()),
                    working_tree_baseline.as_deref(),
                );
                let window = self.effective_context_window(&active_model).max(1) as f64;
                let budget_used = context_tokens as f64 / window;
                match continuation_decision(
                    false,
                    made_progress,
                    budget_used,
                    continuation_count,
                    delta_tokens_last,
                ) {
                    ContinuationDecision::Accept => break,
                    ContinuationDecision::Stop => {
                        self.presenter.emit(PresenterEvent::Warning(format!(
                            "code-change task still shows an empty diff after {continuation_count} \
                             continuation nudge(s), each producing almost nothing — stopping instead \
                             of looping. The fix was described but NOT made."
                        )));
                        break;
                    }
                    ContinuationDecision::Nudge => {
                        self.presenter.emit(PresenterEvent::Warning(format!(
                            "code-change task ended with an empty diff — pushing back \
                             ({}/{CONTINUATION_MAX})",
                            continuation_count + 1
                        )));
                        // Pair with compaction: compact BEFORE the re-drive if the transcript is
                        // near the window, so the nudge has room to actually do the work.
                        self.auto_compact_if_needed(&active_model).await;
                        let seq = self.next_seq();
                        self.store.add_message(
                            &self.id,
                            seq,
                            Role::User,
                            EMPTY_DIFF_NUDGE,
                            None,
                        )?;
                        self.transcript.push(Message::user(EMPTY_DIFF_NUDGE));
                        let tokens_before = context_tokens;
                        let nudge_specs = self.tool_specs();
                        let nudge_outcome = self
                            .run_model_loop(
                                active_model.clone(),
                                &nudge_specs,
                                primary_decision,
                                reuse_response_chain,
                                max_steps,
                                stream_idle,
                            )
                            .await?;
                        adopt_redrive_text(&mut final_text, nudge_outcome.final_text);
                        context_tokens = nudge_outcome.context_tokens;
                        active_model = nudge_outcome.active_model;
                        hit_step_cap = nudge_outcome.hit_step_cap;
                        halted_by_loop_guard = nudge_outcome.halted_by_loop_guard;
                        // The nudge re-drive spawns a FRESH bridge process (a new `mcp-serve`); if it
                        // too failed to start, this stays a toolless turn — carry the signal into the
                        // classification below.
                        saw_mcp_unavailable |= nudge_outcome.mcp_tools_unavailable;
                        // How much the managed transcript grew across this continuation — the
                        // diminishing-returns signal for the NEXT iteration.
                        delta_tokens_last = context_tokens.saturating_sub(tokens_before);
                        continuation_count += 1;
                        if halted_by_loop_guard {
                            break;
                        }
                    }
                }
            }
        }

        // ── Existing-tests-are-spec guard (quality guards wave 4, fix 1) ──────────────────────
        // A headless code-change turn whose working diff MODIFIES existing test files is the
        // xarray-3364 failure shape: the model rewrites test expectations to match its own
        // behavior, the evaluator restores the pristine tests, and the run fails. Before the turn
        // completes: stash exactly the test-file modifications (restoring the pristine tests) and
        // push back ONCE — re-verify against the originals, shrink the fix rather than editing
        // tests, `git stash pop` only if the issue text genuinely demands a test change. NEW test
        // files (git status `A`/`??`) never trip it — writing a reproduction test is normal.
        // One-shot by construction (straight-line code, like the empty-diff nudge above); runs
        // BEFORE self-review/autofix so whatever state the model settles on is still checked.
        // Skipped past the soft deadline: no budget for a guard turn (same rationale as the
        // empty-diff nudge gate above).
        let preserve_existing_tests =
            self.expect_code_change || session_requires_pristine_existing_tests(&self.transcript);
        if self.config.mesh.guard_test_edits
            && preserve_existing_tests
            && !self.past_turn_deadline()
            && !halted_by_loop_guard
        {
            let test_edits = modified_test_files_in_tree(Some(self.workspace.root()));
            if !test_edits.is_empty() && stash_paths(Some(self.workspace.root()), &test_edits) {
                self.presenter.emit(PresenterEvent::Warning(format!(
                    "code-change turn modified {} existing test file(s) — stashed the test edits \
                     and pushing back once: hidden evaluation runs the ORIGINAL tests",
                    test_edits.len()
                )));
                let guard = format!(
                    "{TEST_EDIT_GUARD}\n\nStashed test-file edits:\n- {}",
                    test_edits.join("\n- ")
                );
                let gseq = self.next_seq();
                self.store
                    .add_message(&self.id, gseq, Role::User, &guard, None)?;
                self.transcript.push(Message::user(&guard));
                let guard_specs = self.tool_specs();
                let guard_outcome = self
                    .run_model_loop(
                        active_model.clone(),
                        &guard_specs,
                        primary_decision,
                        reuse_response_chain,
                        max_steps,
                        stream_idle,
                    )
                    .await?;
                // Leave whatever state the model chose; only the answer bookkeeping updates.
                adopt_redrive_text(&mut final_text, guard_outcome.final_text);
                context_tokens = guard_outcome.context_tokens;
                active_model = guard_outcome.active_model;
                hit_step_cap = guard_outcome.hit_step_cap;
                halted_by_loop_guard = guard_outcome.halted_by_loop_guard;
            }
        }

        // ── Direct-provider completeness audit (mesh.verify_completeness) ─────────────────────
        // The loop-gated completeness re-drive inside `run_model_loop` covers CLI bridges, whose
        // subprocess yields back to Forge before the outer turn ends. Direct providers never pass
        // through that bridge-yield branch, so the same opt-in quality lever was silently inert for
        // `codex-oauth`/API models. Run one bounded evidence pass here after the edit/test guard,
        // but only for explicit identifier migrations. Matched same-model evaluation found the
        // direct pass helpful for deprecation/rename work and harmful for unrelated bug fixes; the
        // bridge pass above intentionally remains broad.
        //
        // This is deliberately narrower than `mesh.self_review`: the model must inspect the final
        // diff and perform one related-symbol/call-site search, and may edit only if that search
        // exposes a concrete omitted production path. That targets under-scoped fixes without the
        // unconstrained second-guessing that made always-on self-review regress.
        // Actual writes are authoritative even when a descriptive issue statement does not begin
        // with one of TurnContract's deliberately narrow imperative verbs. Keep the explicit
        // contract paths too: they preserve the intended audit when a provider changed the tree
        // through a mechanism that the per-turn write counter cannot observe.
        let code_change_turn = self.edits_this_turn > 0
            || self.expect_code_change
            || self.last_turn_contract.requires_changed_artifact();
        let primary_changed_paths =
            changed_paths_from_status(working_tree_status(Some(self.workspace.root())).as_deref());
        // Overflow compaction can replace the transcript with a shorter summary while the
        // primary model loop is running. In that case the old start offset is no longer a
        // valid boundary, and the safe result is "no proof that the migration sweep was
        // complete" so the bounded audit remains eligible.
        let primary_turn_messages = self
            .transcript
            .get(primary_transcript_start..)
            .unwrap_or_default();
        let primary_identifier_matches = completeness_production_identifier_search_matches(
            primary_turn_messages,
            self.workspace.root(),
            prompt,
        );
        let primary_migration_sweep_complete = !primary_identifier_matches.is_empty()
            && primary_identifier_matches
                .iter()
                .all(|path| primary_changed_paths.contains(path));
        let primary_opened_identifier_paths = opened_unchanged_production_paths(
            primary_turn_messages,
            self.workspace.root(),
            &primary_changed_paths,
        )
        .into_iter()
        .filter(|path| primary_identifier_matches.contains(path))
        .collect::<std::collections::BTreeSet<_>>();
        if self.config.mesh.verify_completeness
            && code_change_turn
            && direct_completeness_is_identifier_migration(prompt)
            && !primary_migration_sweep_complete
            && !forge_provider::is_cli_bridge(&active_model)
            && !self.past_turn_deadline()
            && !halted_by_loop_guard
            && working_tree_changed_since(
                Some(self.workspace.root()),
                working_tree_baseline.as_deref(),
            )
        {
            self.presenter.emit(PresenterEvent::Warning(
                "completeness check — bounded search for omitted related production paths"
                    .to_string(),
            ));
            self.auto_compact_if_needed(&active_model).await;
            let seq = self.next_seq();
            self.store.add_message(
                &self.id,
                seq,
                Role::System,
                DIRECT_COMPLETENESS_PROMPT,
                None,
            )?;
            self.transcript
                .push(Message::system(DIRECT_COMPLETENESS_PROMPT));
            let audit_transcript_start = self.transcript.len();
            let review_specs = self.tool_specs();
            let review = self
                .run_model_loop(
                    active_model.clone(),
                    &review_specs,
                    None,
                    reuse_response_chain,
                    max_steps,
                    stream_idle,
                )
                .await?;
            adopt_redrive_text(&mut final_text, review.final_text);
            context_tokens = review.context_tokens;
            active_model = review.active_model;
            hit_step_cap = review.hit_step_cap;
            halted_by_loop_guard = review.halted_by_loop_guard;

            let audit_messages = &self.transcript[audit_transcript_start..];
            let repository_search_ran = completeness_repository_search_ran(audit_messages);
            let empty_search = completeness_search_reported_no_matches(audit_messages);
            if !repository_search_ran
                && !self.past_turn_deadline()
                && !hit_step_cap
                && !halted_by_loop_guard
            {
                self.presenter.emit(PresenterEvent::Warning(
                    "completeness audit skipped its repository search — requiring one bounded retry"
                        .to_string(),
                ));
                self.auto_compact_if_needed(&active_model).await;
                let seq = self.next_seq();
                self.store.add_message(
                    &self.id,
                    seq,
                    Role::System,
                    DIRECT_COMPLETENESS_MISSING_SEARCH_RETRY_PROMPT,
                    None,
                )?;
                self.transcript.push(Message::system(
                    DIRECT_COMPLETENESS_MISSING_SEARCH_RETRY_PROMPT,
                ));
                let retry_specs = self.tool_specs();
                let retry = self
                    .run_model_loop(
                        active_model.clone(),
                        &retry_specs,
                        None,
                        reuse_response_chain,
                        max_steps,
                        stream_idle,
                    )
                    .await?;
                adopt_redrive_text(&mut final_text, retry.final_text);
                context_tokens = retry.context_tokens;
                active_model = retry.active_model;
                hit_step_cap = retry.hit_step_cap;
                halted_by_loop_guard = retry.halted_by_loop_guard;
            } else if empty_search
                && !self.past_turn_deadline()
                && !hit_step_cap
                && !halted_by_loop_guard
            {
                self.presenter.emit(PresenterEvent::Warning(
                    "completeness search returned no matches — retrying once from the repository root"
                        .to_string(),
                ));
                self.auto_compact_if_needed(&active_model).await;
                let seq = self.next_seq();
                self.store.add_message(
                    &self.id,
                    seq,
                    Role::System,
                    DIRECT_COMPLETENESS_EMPTY_SEARCH_RETRY_PROMPT,
                    None,
                )?;
                self.transcript.push(Message::system(
                    DIRECT_COMPLETENESS_EMPTY_SEARCH_RETRY_PROMPT,
                ));
                let retry_specs = self.tool_specs();
                let retry = self
                    .run_model_loop(
                        active_model.clone(),
                        &retry_specs,
                        None,
                        reuse_response_chain,
                        max_steps,
                        stream_idle,
                    )
                    .await?;
                adopt_redrive_text(&mut final_text, retry.final_text);
                context_tokens = retry.context_tokens;
                active_model = retry.active_model;
                hit_step_cap = retry.hit_step_cap;
                halted_by_loop_guard = retry.halted_by_loop_guard;
            }

            let changed_paths = changed_paths_from_status(
                working_tree_status(Some(self.workspace.root())).as_deref(),
            );
            let unresolved_paths = unresolved_completeness_production_paths(
                &primary_opened_identifier_paths,
                &self.transcript[audit_transcript_start..],
                self.workspace.root(),
                &changed_paths,
            );
            if !unresolved_paths.is_empty()
                && !self.past_turn_deadline()
                && !hit_step_cap
                && !halted_by_loop_guard
            {
                self.presenter.emit(PresenterEvent::Warning(format!(
                    "completeness audit left {} evidence-backed production path(s) unchanged — \
                     requiring an explicit fix before finishing",
                    unresolved_paths.len()
                )));
                self.auto_compact_if_needed(&active_model).await;
                let prompt = format!(
                    "{DIRECT_COMPLETENESS_UNHANDLED_PATH_PROMPT}\n\nEvidence-backed but unchanged production paths:\n- {}",
                    unresolved_paths.join("\n- ")
                );
                let seq = self.next_seq();
                self.store
                    .add_message(&self.id, seq, Role::System, &prompt, None)?;
                self.transcript.push(Message::system(&prompt));
                let path_specs = self.tool_specs();
                let path_review = self
                    .run_model_loop(
                        active_model.clone(),
                        &path_specs,
                        None,
                        reuse_response_chain,
                        max_steps,
                        stream_idle,
                    )
                    .await?;
                adopt_redrive_text(&mut final_text, path_review.final_text);
                context_tokens = path_review.context_tokens;
                active_model = path_review.active_model;
                hit_step_cap = path_review.hit_step_cap;
                halted_by_loop_guard = path_review.halted_by_loop_guard;

                let post_review_changed_paths = changed_paths_from_status(
                    working_tree_status(Some(self.workspace.root())).as_deref(),
                );
                let still_unresolved = unresolved_paths
                    .iter()
                    .filter(|path| !post_review_changed_paths.contains(path.as_str()))
                    .cloned()
                    .collect::<Vec<_>>();
                if !still_unresolved.is_empty()
                    && !self.past_turn_deadline()
                    && !hit_step_cap
                    && !halted_by_loop_guard
                {
                    self.presenter.emit(PresenterEvent::Warning(format!(
                        "completeness reconciliation left {} named production path(s) unchanged — \
                         retrying once with repository-state enforcement",
                        still_unresolved.len()
                    )));
                    self.auto_compact_if_needed(&active_model).await;
                    let prompt = format!(
                        "{DIRECT_COMPLETENESS_UNRESOLVED_RETRY_PROMPT}\n\nStill-unchanged production paths:\n- {}",
                        still_unresolved.join("\n- ")
                    );
                    let seq = self.next_seq();
                    self.store
                        .add_message(&self.id, seq, Role::System, &prompt, None)?;
                    self.transcript.push(Message::system(&prompt));
                    let retry_specs = self.tool_specs();
                    let retry = self
                        .run_model_loop(
                            active_model.clone(),
                            &retry_specs,
                            None,
                            reuse_response_chain,
                            max_steps,
                            stream_idle,
                        )
                        .await?;
                    adopt_redrive_text(&mut final_text, retry.final_text);
                    context_tokens = retry.context_tokens;
                    active_model = retry.active_model;
                    hit_step_cap = retry.hit_step_cap;
                    halted_by_loop_guard = retry.halted_by_loop_guard;
                }
            }

            // The existing-tests guard runs before this audit, but a weaker reviewer can ignore
            // the audit prompt's "do not modify tests" rule and introduce fresh expectation edits.
            // There is no later model pass that needs those changes, so restore the original tests
            // deterministically before patch capture. New tests remain allowed by
            // `modified_test_files_in_tree`, matching the main guard's contract.
            if self.config.mesh.guard_test_edits && preserve_existing_tests {
                let test_edits = modified_test_files_in_tree(Some(self.workspace.root()));
                if !test_edits.is_empty() && stash_paths(Some(self.workspace.root()), &test_edits) {
                    self.presenter.emit(PresenterEvent::Warning(format!(
                        "completeness audit modified {} existing test file(s) — stashed those \
                         edits so evaluation keeps the original specification",
                        test_edits.len()
                    )));
                }
            }
        }

        // ── Toolless-bridge classification (bridge MCP-tool health guard, wave 7) ─────────────
        // Forge serves the bridged CLI its write tools via `forge mcp-serve`. That server can FAIL
        // TO START under the sandbox — codex logs `resources/list failed: MCP startup failed: No
        // such file or directory` — leaving the model with the filesystem read-only. The turn then
        // "completes" with prose + an empty patch and NO error, so a benchmark scores a silent
        // toolless run as a clean completion (a codex-cli::gpt-5.5 SWE-bench sweep hit this on ~7/15
        // instances). Classify it here (recomputed every turn, so it self-resets): the child showed
        // the MCP-startup signal AND no forge tool ran AND the tree is still empty. The harness
        // (`bench swe` / headless) reads `tools_unavailable()` to RETRY on a fresh process rather
        // than record an empty patch. Gated on `mesh.bridge_require_tools`; kept DISTINCT from the
        // empty-diff nudge above (which handles a NORMAL empty completion, no startup-failure signal).
        // The ENOENT root cause (sandbox vs load) is intermittent and unconfirmed — a respawn on the
        // harness retry usually clears it.
        self.tools_unavailable_run = self.config.mesh.bridge_require_tools
            && classify_tools_unavailable(
                self.expect_code_change,
                forge_provider::is_cli_bridge(&active_model),
                saw_mcp_unavailable,
                turn_tools_ran,
                !working_tree_changed_since(
                    Some(self.workspace.root()),
                    working_tree_baseline.as_deref(),
                ),
            );
        if self.tools_unavailable_run {
            self.presenter.emit(PresenterEvent::Warning(
                "bridge turn ran with NO working tools — Forge's mcp-serve tool server failed to \
                 start (empty tree, zero tool calls); the harness will retry on a fresh process"
                    .to_string(),
            ));
        }

        // ── Self-review pass (mesh.self_review) ───────────────────────────────────────────────
        // One bounded round where the SAME model re-examines the edits it just made against the
        // original task and fixes any bug/incompleteness — the self-correction leverage a
        // single-pass harness lacks, needing no external tools or test env. Fires only on edit
        // turns; runs BEFORE autofix so any fix it makes is then lint/test-checked too.
        if self.config.mesh.self_review && self.edits_this_turn > 0 && !halted_by_loop_guard {
            self.presenter.emit(PresenterEvent::Warning(
                "self-review: re-checking the changes against the task".to_string(),
            ));
            self.transcript.push(Message::system(SELF_REVIEW_PROMPT));
            let rv_specs = self.tool_specs();
            // None decision: no failover/routing churn — keep the same model, like the autofix re-run.
            let rv = self
                .run_model_loop(
                    active_model.clone(),
                    &rv_specs,
                    None,
                    reuse_response_chain,
                    max_steps,
                    stream_idle,
                )
                .await?;
            // Keep the original answer text: the review fixes code, it doesn't re-answer the user.
            context_tokens = rv.context_tokens;
            active_model = rv.active_model;
            hit_step_cap = rv.hit_step_cap;
            halted_by_loop_guard = rv.halted_by_loop_guard;
        }

        // ── Autofix self-healing loop (autofix.md) ────────────────────────────────────────────
        // After the turn's model↔tool loop finishes: if edits were made AND autofix is enabled
        // with at least one non-empty command, run lint/test and feed failures back into the
        // conversation so the model can fix them. Repeat up to `max_iterations`. When autofix is
        // off, or no edits happened, this block is a no-op (zero overhead).
        let mut af = self.config.autofix.clone();

        // Auto-detect only fills commands the user explicitly enabled. It must not silently turn
        // default-off autofix on: a missing project script would otherwise inject its failure and
        // spend a whole additional model loop repairing an unrequested check.
        let needs_detected_lint = af.auto_lint && af.lint_cmd.is_empty();
        let needs_detected_test = af.auto_test && af.test_cmd.is_empty();
        if af.auto_detect
            && self.edits_this_turn > 0
            && (needs_detected_lint || needs_detected_test)
        {
            match Self::detect_project_commands(self.workspace.root()) {
                Ok(Some((lint, test))) => {
                    let detected = Self::fill_detected_autofix_commands(&mut af, lint, test);
                    if !detected.is_empty() {
                        self.presenter.emit(PresenterEvent::Warning(format!(
                            "autofix: auto-detected project command(s): {}",
                            detected.join("; ")
                        )));
                    }
                }
                Ok(None) => {}
                Err(error) => self.presenter.emit(PresenterEvent::Warning(format!(
                    "autofix: could not inspect project checks: {error}"
                ))),
            }
        }

        let autofix_active = self.edits_this_turn > 0
            && !halted_by_loop_guard
            && ((af.auto_lint && !af.lint_cmd.is_empty())
                || (af.auto_test && !af.test_cmd.is_empty()));

        if autofix_active {
            self.presenter.emit(PresenterEvent::Warning(format!(
                "autofix: running checks after {} edit(s)",
                self.edits_this_turn
            )));
            let mut iterations_used = 0u32;
            loop {
                if iterations_used >= af.max_iterations {
                    self.presenter.emit(PresenterEvent::Warning(format!(
                        "autofix: reached iteration cap ({}) — stopping; remaining failures \
                         were not fixed",
                        af.max_iterations
                    )));
                    break;
                }
                iterations_used += 1;

                match self.run_autofix_stage(&af).await {
                    Ok(true) => {
                        self.presenter.emit(PresenterEvent::Warning(
                            "autofix: all checks passed".to_string(),
                        ));
                        break;
                    }
                    Ok(false) => {
                        // Failures already injected into transcript by run_autofix_stage.
                        // Re-run the model↔tool inner loop to let the model fix them.
                        self.presenter.emit(PresenterEvent::Warning(format!(
                            "autofix: iteration {iterations_used}/{} — re-running model loop",
                            af.max_iterations
                        )));
                        // Autofix re-run: pass None for decision so failover, routing record, and
                        // quota hints are all suppressed — the active_model is kept from the
                        // primary turn (or last failover) and is not changed here.
                        let fix_specs = self.tool_specs();
                        let fix_outcome = self
                            .run_model_loop(
                                active_model.clone(),
                                &fix_specs,
                                None,
                                reuse_response_chain,
                                max_steps,
                                stream_idle,
                            )
                            .await?;
                        adopt_redrive_text(&mut final_text, fix_outcome.final_text);
                        context_tokens = fix_outcome.context_tokens;
                        active_model = fix_outcome.active_model;
                        hit_step_cap = fix_outcome.hit_step_cap;
                        halted_by_loop_guard = fix_outcome.halted_by_loop_guard;
                        if halted_by_loop_guard {
                            break;
                        }
                        if fix_outcome.hit_step_cap {
                            self.presenter.emit(PresenterEvent::Warning(format!(
                                "autofix: inner model loop hit the {max_steps}-step limit"
                            )));
                        }
                    }
                    Err(e) => {
                        // Autofix infrastructure failure — surface as warning and abort the loop.
                        self.presenter.emit(PresenterEvent::Warning(format!(
                            "autofix: stage error ({e}) — skipping remaining iterations"
                        )));
                        break;
                    }
                }
            }
        }
        // ── End autofix ───────────────────────────────────────────────────────────────────────

        // ── Auto-review gate (assay.auto_review) ──────────────────────────────────────────────
        // When enabled: build a unified diff of files written THIS turn, run the Assay critic
        // crew over it, and either warn or block depending on gate_mode. Zero overhead when off.
        if self.config.assay.auto_review && self.edits_this_turn > 0 && !halted_by_loop_guard {
            let ar = self.config.assay.clone();
            if let Err(e) = self.auto_review_gate(&ar).await {
                // TurnBlocked propagates up so the caller can surface it; other errors are
                // infrastructure failures we surface as warnings to avoid silently killing the turn.
                match &e {
                    CoreError::TurnBlocked(_) => return Err(e),
                    _ => {
                        self.presenter.emit(PresenterEvent::Warning(format!(
                            "auto-review: gate error ({e}) — skipping"
                        )));
                    }
                }
            }
        }
        // ── End auto-review gate ───────────────────────────────────────────────────────────────

        // ── Plan approval (planning mode → interactive approve → auto-build) ──────────────────
        // If the model proposed a plan this turn (present_plan), ask the user to approve it now —
        // the model loop has ended, so blocking on the presenter is safe (no stream is being read,
        // and bridge turns are fully drained). Approval switches to Auto-edit and recursively runs
        // the build turn through the full machinery (autofix, self-review, gate); typed feedback
        // runs a fresh planning turn; Cancel falls through and ends the turn in planning mode.
        if let Some(plan) = self.pending_plan.take() {
            if !halted_by_loop_guard {
                if let Some(followup) = self.resolve_plan_approval(&plan) {
                    return Box::pin(self.run_turn_with(&followup, &[], Some(TaskTier::Complex)))
                        .await;
                }
            }
        }

        // ── Stop lifecycle hook (Claude-Code parity: "Stop hook can block stopping") ──────────
        // Fire the Stop hook BEFORE finalizing. A hook that returns block ({"decision":"block"} /
        // exit 2) means "don't stop yet": its reason is fed back as a synthetic user message and the
        // model loop re-runs, so the agent keeps working instead of ending the turn. Bounded by
        // MAX_STOP_BLOCKS consecutive blocks (mirroring the codebase's other loop guards) so a hook
        // that always blocks can't wedge the turn — after the cap we force-stop with a warning. The
        // `stop_hook_active` flag (true once we're already in a continuation) lets a well-behaved
        // hook break its own loop by approving when set, exactly like Claude Code.
        const MAX_STOP_BLOCKS: u32 = 3;
        let mut stop_blocks: u32 = 0;
        loop {
            let stop_outcome = self
                .fire_lifecycle(
                    forge_config::HookEvent::Stop,
                    serde_json::json!({
                        "stop_hook_active": stop_blocks > 0,
                        "hit_step_cap": hit_step_cap,
                    }),
                )
                .await;
            let Some(reason) = stop_outcome.blocked else {
                break;
            };
            if halted_by_loop_guard {
                break;
            }
            if stop_blocks >= MAX_STOP_BLOCKS {
                self.presenter.emit(PresenterEvent::Warning(format!(
                    "stop hook blocked {MAX_STOP_BLOCKS}× in a row — forcing the turn to end ({reason})"
                )));
                break;
            }
            stop_blocks += 1;
            self.presenter.emit(PresenterEvent::Warning(format!(
                "stop hook requested continuation ({stop_blocks}/{MAX_STOP_BLOCKS}): {reason}"
            )));
            // Feed the reason back as a synthetic user message and re-drive the model loop (None
            // decision: no cross-model failover for the continuation, like the autofix re-run).
            let cont = format!("[stop hook] {reason}");
            let seq = self.next_seq();
            self.store
                .add_message(&self.id, seq, Role::User, &cont, None)?;
            self.transcript.push(Message::user(&cont));
            let cont_specs = self.tool_specs();
            let cont_outcome = self
                .run_model_loop(
                    active_model.clone(),
                    &cont_specs,
                    None,
                    reuse_response_chain,
                    max_steps,
                    stream_idle,
                )
                .await?;
            adopt_redrive_text(&mut final_text, cont_outcome.final_text);
            context_tokens = cont_outcome.context_tokens;
            active_model = cont_outcome.active_model;
            hit_step_cap = cont_outcome.hit_step_cap;
            halted_by_loop_guard = cont_outcome.halted_by_loop_guard;
            if halted_by_loop_guard {
                break;
            }
        }

        let stop_reason = if hit_step_cap {
            StopReason::MaxSteps
        } else {
            StopReason::FinalAnswer
        };
        // Cache warmth follows the model that actually completed the turn after any failover and
        // guard re-drive. It is intentionally in-memory/session-local and remains only an input to
        // the next contextual mesh decision, never a pin.
        self.route_affinity = Some(SessionAffinity {
            model: active_model.clone(),
            tier: decision.tier,
            code_heavy: RouteHints::from_context(prompt, &routing_context).code_heavy,
        });
        // A channel-backed interactive surface can keep receiving detached auxiliary results after
        // the main turn is done. Remember that capability before launching the side calls so the
        // input is never held hostage by recap/suggestion/memory provider latency.
        let detach_post_turn_work = self.presenter.recap_sink().is_some();
        if detach_post_turn_work {
            self.emit_terminal_events(&final_text, stop_reason, context_tokens, &active_model)?;
        }
        self.generate_recap(prompt, &final_text, &recap_tasks_before)
            .await;
        self.generate_suggestion(prompt, &final_text).await;
        // Continual Harness auto-refine gate (`harness.auto_refine = "turns"`, refinement.rs).
        // Runs inline (unlike recap/suggestion) rather than detaching onto the presenter's sink:
        // it mutates durable harness state and its own presenter note should land in order with
        // the turn it concluded, not race a later turn's output.
        self.auto_refine_after_turns().await;
        // One-shot/headless mode must await memory persistence before the process exits. In the
        // interactive TUI, dropping a Tokio JoinHandle detaches (does not cancel) the capture, so
        // the completed answer and input become usable immediately while persistence finishes.
        if let Some(handle) = self.capture_memories(prompt, &final_text) {
            if detach_post_turn_work {
                drop(handle);
            } else {
                let _ = handle.await;
            }
        }
        if !detach_post_turn_work {
            // Headless callers await auxiliary memory persistence, so emit their terminal usage
            // only after those provider calls are in the Store ledger. Keep Done last: JSONL
            // consumers commonly stop reading at the result event.
            self.emit_terminal_events(&final_text, stop_reason, context_tokens, &active_model)?;
        }
        Ok(if hit_step_cap {
            LoopOutcome::max_steps(final_text)
        } else {
            LoopOutcome::final_answer(final_text)
        })
    }
}

/// The `ToolSpec` advertised to the model for [`ASK_USER_TOOL`].
fn ask_user_spec() -> ToolSpec {
    ToolSpec {
        name: ASK_USER_TOOL.to_string(),
        description: "Ask the user a single focused question when you hit a real decision only \
            they can make (a value choice, a missing requirement). Provide 2–4 suggested \
            `options` with short labels (+ optional descriptions); set `allow_other` (default \
            true) to also accept a free-text answer. Returns the user's choice. Don't use it for \
            things you can decide yourself."
            .to_string(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "question": { "type": "string", "description": "the question to ask" },
                "options": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": { "type": "string" },
                            "description": { "type": "string" }
                        },
                        "required": ["label"]
                    }
                },
                "allow_other": {
                    "type": "boolean",
                    "description": "allow a free-text answer beyond the options (default true)"
                }
            },
            "required": ["question"]
        }),
    }
}

/// The skill-loading virtual tool name.
pub const USE_SKILL_TOOL: &str = "use_skill";

/// The `ToolSpec` advertised for [`USE_SKILL_TOOL`], listing the available Forge skills in its
/// description so the model both *discovers* what exists and can *invoke* one. Shared by the
/// direct path and the CLI-bridge `mcp-serve` handler so a bridged claude/codex sees it too.
pub fn use_skill_spec(catalog: &forge_skills::Catalog) -> ToolSpec {
    let listing = catalog
        .skill_listing()
        .into_iter()
        .map(|(name, desc)| {
            let desc: String = desc.chars().take(100).collect();
            format!("- {name}: {desc}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    ToolSpec {
        name: USE_SKILL_TOOL.to_string(),
        description: format!(
            "Load a Forge skill's methodology into this turn, then follow it. These are Forge's \
             OWN skills — do NOT search the filesystem (~/.claude, ~/.codex) for skills; call this \
             tool with the exact skill name instead. Available skills:\n{listing}"
        ),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "exact skill name from the list" }
            },
            "required": ["name"]
        }),
    }
}

/// The task-tracking virtual tool name.
pub const UPDATE_TASKS_TOOL: &str = "update_tasks";

/// Parse the `update_tasks` arguments into a task list (tolerant of missing/loose fields).
/// Shared by the in-process intercept and the CLI-bridge `mcp-serve` handler.
pub fn parse_tasks(args: &serde_json::Value) -> Vec<forge_types::TodoItem> {
    use forge_types::{TodoItem, TodoStatus};
    args.get("tasks")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    // Claude/Codex task tools use a few equivalent field names. Accept their
                    // native shapes at this compatibility boundary so a bridge cannot silently
                    // replace a non-empty task list with zero parsed tasks.
                    let title = ["title", "content", "description"]
                        .into_iter()
                        .find_map(|key| t.get(key).and_then(|v| v.as_str()))?
                        .trim();
                    (!title.is_empty()).then(|| TodoItem {
                        title: title.to_string(),
                        status: t
                            .get("status")
                            .and_then(|v| v.as_str())
                            .map(TodoStatus::parse_loose)
                            .unwrap_or_default(),
                        // Optional and only ever what the model actually wrote: an omitted (or
                        // blank) `assignee` stays `None` rather than defaulting to some invented
                        // owner, so a client renders the task as unassigned.
                        assignee: t
                            .get("assignee")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Apply an `update_tasks` payload without letting a bridge's status-only subset silently delete
/// the rest of the plan. The advertised contract remains a full ordered replacement, but Claude
/// occasionally sends only the item whose status changed. When every item in a shorter payload
/// matches an existing title, treat it as a patch; a full/new list still replaces, and `[]` still
/// explicitly clears the tracker.
pub fn merge_task_update(
    existing: &[forge_types::TodoItem],
    incoming: Vec<forge_types::TodoItem>,
) -> Vec<forge_types::TodoItem> {
    if incoming.is_empty() || existing.is_empty() || incoming.len() >= existing.len() {
        return incoming;
    }
    if !incoming
        .iter()
        .all(|update| existing.iter().any(|task| task.title == update.title))
    {
        return incoming;
    }

    let mut merged = existing.to_vec();
    for update in incoming {
        if let Some(task) = merged.iter_mut().find(|task| task.title == update.title) {
            task.status = update.status;
            // A patch payload is written to move a STATUS; a bridge that re-sends the item
            // without repeating `assignee` is not unassigning it. Only an explicit owner
            // overwrites — otherwise the delegation recorded by the full list would silently
            // evaporate on the next status tick.
            if update.assignee.is_some() {
                task.assignee = update.assignee;
            }
        }
    }
    merged
}

/// The `ToolSpec` advertised to the model for [`UPDATE_TASKS_TOOL`].
pub fn update_tasks_spec() -> ToolSpec {
    ToolSpec {
        name: UPDATE_TASKS_TOOL.to_string(),
        description: "Maintain a visible task list for multi-step work. Call it when you start a \
            task with 2+ steps and again whenever a step's state changes — pass the FULL ordered \
            list each time (it replaces the previous one). This is non-blocking bookkeeping: NEVER \
            call this tool by itself when an independent read, edit, or check can advance the work; \
            request both in the same response. A standalone update is only appropriate when no \
            substantive next action exists, such as immediately before the final answer. Mark \
            exactly one task `in_progress` while you work it and mark completed steps `done`. Keep \
            titles short and concrete. Skip it for trivial single-step requests."
            .to_string(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "description": "the full ordered task list (replaces the previous list)",
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": { "type": "string", "description": "short task description" },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "done"],
                                "description": "task state (default pending)"
                            },
                            "assignee": {
                                "type": "string",
                                "description": "optional — the subagent or worker responsible \
                                    for this task, when the model is delegating. Omit for work \
                                    you are doing yourself; do not invent an owner."
                            }
                        },
                        "required": ["title"]
                    }
                }
            },
            "required": ["tasks"]
        }),
    }
}

/// The plan-presentation virtual tool name (planning mode).
pub const PRESENT_PLAN_TOOL: &str = "present_plan";

/// The prompt that drives the build turn after a plan is approved (mirrors `/execute`).
const PLAN_BUILD_PROMPT: &str = "Implement the plan you just proposed, step by step — make the \
    edits and run the commands needed to carry it out. Update each task's status (in_progress → \
    done) with update_tasks as you go. If something forces a deviation from the plan, say so and \
    keep going.";

/// Parse `present_plan` arguments into a [`PlanProposal`] (tolerant of missing/loose fields).
/// Shared by the in-process intercept and the CLI-bridge `mcp-serve` handler.
pub fn parse_plan(args: &serde_json::Value) -> forge_types::PlanProposal {
    use forge_types::{PlanProposal, PlanStep};
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Plan")
        .to_string();
    let steps = args
        .get("steps")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| {
                    let title = s.get("title").and_then(|v| v.as_str())?.trim();
                    (!title.is_empty()).then(|| PlanStep {
                        title: title.to_string(),
                        detail: s
                            .get("detail")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let notes = args
        .get("notes")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string);
    PlanProposal {
        title,
        steps,
        notes,
    }
}

/// Persist a proposed plan to `.forge/plans/<session>.md` (human-readable markdown) so it survives
/// the session and the user can open/track it. Called on every `present_plan` — creation, draft,
/// revision. Best-effort: a write failure never breaks the turn.
pub fn persist_plan(session_id: &str, plan: &forge_types::PlanProposal) {
    let dir = std::path::Path::new(".forge").join("plans");
    let mut md = format!("# {}\n\n", plan.title.trim());
    for (i, s) in plan.steps.iter().enumerate() {
        md.push_str(&format!("{}. {}\n", i + 1, s.title.trim()));
        let d = s.detail.trim();
        if !d.is_empty() {
            md.push_str(&format!("   - {d}\n"));
        }
    }
    if let Some(n) = plan
        .notes
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        md.push_str(&format!("\n> Notes: {n}\n"));
    }
    let safe: String = session_id
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .take(48)
        .collect();
    let name = if safe.is_empty() {
        "plan".to_string()
    } else {
        safe
    };
    let file = dir.join(format!("{name}.md"));

    // Best-effort, off the executor: the write is small and infrequent, but a slow/networked FS
    // shouldn't stall the async turn loop. `spawn_blocking` runs it on the blocking pool; when no
    // runtime is active (a plain sync caller, e.g. a unit test) fall back to an inline write.
    let do_write = move || {
        if std::fs::create_dir_all(&dir).is_ok() {
            let _ = std::fs::write(&file, md);
        }
    };
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn_blocking(do_write);
        }
        Err(_) => do_write(),
    }
}

/// The `ToolSpec` advertised for [`PRESENT_PLAN_TOOL`] — offered only in planning mode.
pub fn present_plan_spec() -> ToolSpec {
    ToolSpec {
        name: PRESENT_PLAN_TOOL.to_string(),
        description: "Present your proposed plan for the user to approve (planning mode). Call this \
            ONCE you have investigated enough — pass a short `title`, an ordered `steps` array (each \
            step a `title` + optional one-line `detail`), and optional `notes` (risks/assumptions). \
            It renders an interactive plan card: the user approves to auto-build (you switch to \
            Auto-edit), types changes to revise, or cancels. Do NOT edit anything before presenting."
            .to_string(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "short plan title" },
                "steps": {
                    "type": "array",
                    "description": "the ordered plan steps",
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": { "type": "string", "description": "what this step does" },
                            "detail": { "type": "string", "description": "optional one-line elaboration" }
                        },
                        "required": ["title"]
                    }
                },
                "notes": { "type": "string", "description": "optional risks/assumptions" }
            },
            "required": ["title", "steps"]
        }),
    }
}

/// True if the per-process budget override is set (lets one over-budget run proceed).
/// Scale the Lattice injection token budget by budget pressure: full when Ok, half at Warning, a
/// quarter at Exhausted. Context spend follows the same discipline as model spend (§5.4).
fn inject_budget(base: usize, status: BudgetStatus) -> usize {
    match status {
        BudgetStatus::Ok => base,
        BudgetStatus::Warning => base / 2,
        BudgetStatus::Exhausted => base / 4,
    }
}

/// Await a streaming completion, but abort it if the stream goes silent for `idle` (a half-open /
/// stalled connection) so a turn never hangs forever — the caller treats the synthesized
/// `Unavailable` as retryable and fails over. `activity` is bumped by the completion's event sink;
/// `idle == 0` disables the watchdog. Polls coarsely (every few seconds) — this guards against a
/// hang, it is not a precise deadline.
async fn stream_with_idle_timeout<F>(
    fut: F,
    activity: &std::sync::atomic::AtomicU64,
    active_tools: Option<&std::sync::atomic::AtomicU64>,
    idle: std::time::Duration,
) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError>
where
    F: std::future::Future<
        Output = Result<forge_provider::ModelResponse, forge_provider::ProviderError>,
    >,
{
    tokio::pin!(fut);
    if idle.is_zero() {
        return fut.await;
    }
    let mut last_seen = 0u64;
    let mut last_change = std::time::Instant::now();
    let poll = std::time::Duration::from_secs(3).min(idle);
    loop {
        tokio::select! {
            r = &mut fut => return r,
            _ = tokio::time::sleep(poll) => {
                let now = activity.load(std::sync::atomic::Ordering::Relaxed);
                if now != last_seen {
                    last_seen = now;
                    last_change = std::time::Instant::now();
                } else {
                    // A provider may go quiet while an MCP tool legitimately runs. Forge's shell
                    // tool has its own bounded timeout near the base stream-idle boundary, so give
                    // a positively identified in-flight tool one extra window to report its
                    // ToolFinished event. Ordinary half-open streams keep the original cutoff.
                    let effective_idle = if active_tools.is_some_and(|tools| {
                        tools.load(std::sync::atomic::Ordering::Relaxed) > 0
                    }) {
                        idle.saturating_mul(2)
                    } else {
                        idle
                    };
                    if last_change.elapsed() < effective_idle {
                        continue;
                    }
                    return Err(forge_provider::ProviderError::Unavailable(format!(
                        "stream stalled — no data for {}s",
                        effective_idle.as_secs()
                    )));
                }
            }
        }
    }
}

fn budget_override_active() -> bool {
    matches!(
        std::env::var("FORGE_BUDGET_OVERRIDE").as_deref(),
        Ok("1") | Ok("true")
    )
}

fn over_budget_message(b: &BudgetState) -> String {
    let cap = |c: Option<f64>| c.map(|v| format!("${v:.2}")).unwrap_or_else(|| "∞".into());
    format!(
        "budget cap reached — today ${:.4}/{}, month ${:.4}/{}. Refusing further model calls. \
         Set FORGE_BUDGET_OVERRIDE=1 to proceed.",
        b.spent_today_usd,
        cap(b.daily_cap_usd),
        b.spent_month_usd,
        cap(b.monthly_cap_usd)
    )
}

/// Actionable message when the mesh routed to a model whose provider has no API key and nothing
/// else was usable — instead of silently calling it and auth-failing every turn. Names the dead
/// provider, lists the providers that DO have a usable key, and gives the concrete fixes.
fn no_usable_model_message(routed_model: &str) -> String {
    let provider = forge_config::provider_of(routed_model);
    let keyed: Vec<&str> = forge_config::known_key_providers()
        .filter(|p| forge_config::has_api_key(p))
        .collect();
    let have = if keyed.is_empty() {
        "no provider API keys are configured".to_string()
    } else {
        format!("you have keys for: {}", keyed.join(", "))
    };
    format!(
        "No usable model for this turn: the mesh routed to '{routed_model}', but provider \
         '{provider}' has no API key and no other model was usable ({have}).\n\
         Fix one of:\n  \
         • forge setup     — guided first-run wizard (pick a provider, add a key)\n  \
         • forge auth      — add a provider API key\n  \
         • forge models    — see which models are actually usable right now\n  \
         • /model <id>     — pin a usable model for this session\n  \
         • ollama serve    — run a local model (no key needed)\n\
         If you DO have a key for another provider, run `forge models`: auto-discovery may have \
         failed to reach it, so the mesh fell back to the built-in defaults (which lead with \
         '{provider}')."
    )
}

/// Compare previous and current findings, return a human-readable diff note.
/// Matching is by (file, title) — same issue at the same location.
fn assay_diff_note(
    prev: &[forge_types::Finding],
    current: &[forge_types::Finding],
    prev_id: &str,
) -> String {
    let key = |f: &forge_types::Finding| format!("{}|{}", f.file, f.title);
    let prev_keys: std::collections::HashSet<String> = prev.iter().map(key).collect();
    let curr_keys: std::collections::HashSet<String> = current.iter().map(key).collect();
    let fixed: usize = prev_keys.difference(&curr_keys).count();
    let new_: usize = curr_keys.difference(&prev_keys).count();
    let still_open: usize = prev_keys.intersection(&curr_keys).count();
    if fixed == 0 && new_ == 0 {
        return String::new(); // nothing to say — identical findings
    }
    format!(
        "⚒ vs run {prev_id}: {} fixed · {} new · {} still-open",
        fixed, new_, still_open
    )
}

/// Build the Refine (cleanup) task prompt from an assay report: instruct the agent loop to fix
/// each finding by editing files (gated + snapshotted via the normal turn path).
fn refine_prompt(report: &forge_types::AssayReport) -> String {
    let mut s = String::from(
        "You are Refine, a cleanup crew. An Assay analysis found the issues below in this \
         codebase. Fix each one by editing the relevant files (edit_file/write_file). Be surgical \
         — fix exactly the issue without breaking working code or changing unrelated behavior. If \
         a finding is a false positive, skip it and briefly say why.\n\nIssues:\n",
    );
    for (i, f) in report.findings.iter().enumerate() {
        let loc = match f.line {
            Some(l) => format!("{}:{l}", f.file),
            None => f.file.clone(),
        };
        s.push_str(&format!(
            "{}. [{}] {} — {}\n   why: {}\n   suggested fix: {}\n",
            i + 1,
            f.severity.as_str(),
            loc,
            f.title,
            f.rationale,
            f.suggested_fix
        ));
    }
    s
}

/// A short single-line label for an auto-checkpoint: the prompt's first line, char-truncated.
fn checkpoint_preview(prompt: &str) -> String {
    let first = prompt.lines().next().unwrap_or("").trim();
    if first.chars().count() > 60 {
        format!("{}…", first.chars().take(60).collect::<String>())
    } else {
        first.to_string()
    }
}

fn summarize(s: &str) -> String {
    let first = s.lines().next().unwrap_or("").trim();
    // Truncate by *characters*, not bytes — a byte slice (`&first[..80]`) panics when the
    // cut falls inside a multi-byte UTF-8 char, which real tool output (file contents, shell
    // output, accents/emoji) routinely contains.
    if first.chars().count() > 80 {
        let head: String = first.chars().take(80).collect();
        format!("{head}…")
    } else {
        first.to_string()
    }
}

pub static TEST_CWD_MUTEX: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

pub struct TestCwdGuard {
    prior: std::path::PathBuf,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl Drop for TestCwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.prior);
    }
}

pub fn test_cwd_guard(target: &std::path::Path) -> TestCwdGuard {
    let lock = TEST_CWD_MUTEX
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .expect("locking test cwd mutex");
    let prior = std::env::current_dir().expect("reading test process cwd");
    std::env::set_current_dir(target).expect("entering guarded test cwd");
    TestCwdGuard { prior, _lock: lock }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_mesh::HeuristicRouter;
    use forge_provider::MockProvider;
    use forge_tui::HeadlessPresenter;
    use forge_types::SideEffect;
    use std::sync::{Arc, Mutex};

    #[test]
    fn response_chain_reuse_is_scoped_to_dependent_same_model_continuations() {
        let context = RoutingContext::from_messages(&[
            Message::user("debug the scheduler race condition and prove the fix"),
            Message::assistant("I found the unsafe interleaving and am implementing the fix."),
        ]);
        let affinity = SessionAffinity {
            model: "codex-oauth::gpt-5.6-sol".into(),
            tier: TaskTier::Complex,
            code_heavy: true,
        };

        assert!(should_reuse_response_chain(
            "continue",
            &context,
            Some(&affinity),
            "codex-oauth::gpt-5.6-sol",
        ));
        assert!(
            !should_reuse_response_chain(
                "continue",
                &context,
                Some(&affinity),
                "codex-oauth::gpt-5.6-terra",
            ),
            "a model switch must not inherit another model's response chain"
        );
        assert!(
            !should_reuse_response_chain(
                "How many tasks are due today?",
                &context,
                Some(&affinity),
                "codex-oauth::gpt-5.6-sol",
            ),
            "an unrelated task must start a fresh provider boundary"
        );
        assert!(
            !should_reuse_response_chain(
                "continue",
                &RoutingContext::default(),
                Some(&affinity),
                "codex-oauth::gpt-5.6-sol",
            ),
            "a new session without an active task must not inherit affinity"
        );
    }

    #[tokio::test]
    async fn saved_workflow_persists_a_parent_replay_audit() {
        let dir =
            std::env::temp_dir().join(format!("forge-workflow-replay-{}", forge_types::new_id()));
        let workflows = dir.join(".forge").join("workflows");
        std::fs::create_dir_all(&workflows).unwrap();
        std::fs::write(
            workflows.join("audit.js"),
            r#"await phase("scan"); await log("checked fixture"); return "AUDIT_OK";"#,
        )
        .unwrap();

        let router = Arc::new(FixedRouter {
            model: "m::unused".into(),
            fallbacks: vec![],
        });
        let (store, mut session) = fixed_session(Arc::new(PanicProvider), router);
        session.workspace = WorkspaceContext::new(&dir).unwrap();

        let result = session
            .run_saved_workflow("audit", serde_json::Value::Null)
            .await
            .unwrap();
        assert_eq!(result, "AUDIT_OK");

        let replay = store.load_replay(&session.id).unwrap();
        assert!(replay
            .iter()
            .any(|entry| { entry.role == Role::User && entry.content == "/workflow run audit" }));
        // The same run is recorded as history the workflow library can show, with the counts it
        // actually observed (one `phase()` call, no agents, so no cost).
        let runs = store
            .list_workflow_runs(
                "audit",
                &dir.canonicalize().unwrap().display().to_string(),
                10,
            )
            .unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "ok");
        assert_eq!(runs[0].summary.as_deref(), Some("AUDIT_OK"));
        assert_eq!(runs[0].session_id, session.id);
        assert_eq!((runs[0].phases, runs[0].agents), (1, 0));
        assert_eq!(runs[0].cost_usd, 0.0);
        assert!(runs[0].finished_at.is_some());

        // A run that never gets off the ground is recorded as `failed`, not left open.
        assert!(session
            .run_saved_workflow("missing", serde_json::Value::Null)
            .await
            .is_err());
        let failed = store
            .list_workflow_runs(
                "missing",
                &dir.canonicalize().unwrap().display().to_string(),
                10,
            )
            .unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].status, "failed");
        assert!(failed[0].summary.as_deref().unwrap().contains("missing"));

        let audit = replay
            .iter()
            .find(|entry| {
                entry.role == Role::Assistant && entry.content.contains("workflow 'audit' started")
            })
            .expect("saved workflow audit transcript");
        assert!(audit.content.contains("▶ phase: scan"));
        assert!(audit.content.contains("💬 checked fixture"));
        assert!(audit
            .content
            .contains("⛓ ✓ workflow finished: 'audit': AUDIT_OK"));

        let _ = std::fs::remove_dir_all(dir);
    }

    // ── Next-prompt suggestion sanitizer ────────────────────────────────────────────────────────
    #[test]
    fn sanitize_suggestion_strips_quotes_and_newlines() {
        let raw = "\"add a test for this\"\nextra chatter the model shouldn't have written";
        assert_eq!(
            sanitize_suggestion(raw, "fix the bug").as_deref(),
            Some("add a test for this")
        );
    }

    #[test]
    fn sanitize_suggestion_truncates_to_160_chars() {
        let raw = "a".repeat(300);
        let out = sanitize_suggestion(&raw, "unrelated").unwrap();
        assert_eq!(out.chars().count(), 160);
    }

    #[test]
    fn sanitize_suggestion_discards_empty() {
        assert_eq!(sanitize_suggestion("", "fix the bug"), None);
        assert_eq!(sanitize_suggestion("   \n  ", "fix the bug"), None);
        assert_eq!(sanitize_suggestion("\"\"", "fix the bug"), None);
    }

    #[test]
    fn sanitize_suggestion_discards_repeat_of_previous_prompt_case_insensitive() {
        assert_eq!(sanitize_suggestion("Fix The Bug", "  fix the bug  "), None);
        assert_eq!(
            sanitize_suggestion("fix the bug now", "fix the bug"),
            Some("fix the bug now".to_string())
        );
    }

    #[test]
    fn output_planning_reserve_does_not_turn_unbounded_output_into_a_cap() {
        assert_eq!(
            output_planning_reserve_tokens(0),
            UNBOUNDED_OUTPUT_PLANNING_RESERVE
        );
        assert_eq!(output_planning_reserve_tokens(256), 1_024);
        assert_eq!(output_planning_reserve_tokens(12_345), 12_345);
    }

    // ── Routing context floor — mesh auto-rotation must never pick a too-small window ──────────
    #[test]
    fn routing_min_context_floors_at_coding_baseline_when_transcript_is_small() {
        // Fresh/short session: transcript demand is tiny, so the absolute coding floor governs —
        // this is exactly the case that let a 4k model (allam-2-7b) get routed then rejected.
        assert_eq!(routing_min_context_tokens(0, 4096), MIN_CODING_CONTEXT);
        assert_eq!(routing_min_context_tokens(2_000, 4096), MIN_CODING_CONTEXT);
    }

    #[test]
    fn routing_min_context_tracks_transcript_plus_reserve_once_it_grows() {
        // Long session: floor must exceed `transcript·5/4 + reserve` so the router never admits a
        // window `admit_failover_model` would immediately reject (the churning-consent-prompt bug).
        assert_eq!(
            routing_min_context_tokens(40_000, 8_192),
            40_000 * 5 / 4 + 8_192
        );
        // Result must clear transcript_fits' bar: window·0.8 - reserve·0.8 ≥ transcript.
        let (transcript, reserve) = (40_000u32, 8_192u32);
        let win = routing_min_context_tokens(transcript, reserve) as u64;
        let usable = (win - reserve as u64) * 8 / 10;
        assert!(
            usable >= transcript as u64,
            "chosen window must fit the transcript"
        );
    }

    #[test]
    fn routing_min_context_saturates_on_absurd_transcript() {
        // No panic/overflow on a pathological transcript size.
        assert_eq!(routing_min_context_tokens(u32::MAX, u32::MAX), u32::MAX);
    }

    // ── Token-budget continuation guard (H8) — pure decision, offline-unit-tested ──────────────
    #[test]
    fn continuation_nudges_when_under_budget_no_progress_unverified() {
        // (turn under budget + no progress + goal unverified) → Nudge.
        assert_eq!(
            continuation_decision(false, false, 0.10, 0, u64::MAX),
            ContinuationDecision::Nudge
        );
        // Just below the budget ceiling still nudges.
        assert_eq!(
            continuation_decision(false, false, 0.89, 1, 900),
            ContinuationDecision::Nudge
        );
    }

    #[test]
    fn continuation_stops_on_diminishing_returns() {
        // (continuation_count >= MIN && dtok < FLOOR) → Stop.
        assert_eq!(
            continuation_decision(
                false,
                false,
                0.10,
                CONTINUATION_DIMINISHING_MIN,
                CONTINUATION_DIMINISHING_TOKEN_FLOOR - 1
            ),
            ContinuationDecision::Stop
        );
        // Under the min continuations it does NOT stop yet even with a tiny delta — it nudges.
        assert_eq!(
            continuation_decision(false, false, 0.10, CONTINUATION_DIMINISHING_MIN - 1, 0),
            ContinuationDecision::Nudge
        );
        // Above the min but still producing real output (>= floor) keeps nudging (not diminishing)…
        assert_eq!(
            continuation_decision(
                false,
                false,
                0.10,
                CONTINUATION_DIMINISHING_MIN,
                CONTINUATION_DIMINISHING_TOKEN_FLOOR
            ),
            ContinuationDecision::Nudge
        );
        // …until the absolute ceiling, which stops the loop regardless of output size.
        assert_eq!(
            continuation_decision(false, false, 0.10, CONTINUATION_MAX, 10_000),
            ContinuationDecision::Stop
        );
    }

    #[test]
    fn continuation_accepts_on_progress_or_verified_or_no_budget() {
        // Real progress made this turn → never nudge, even under budget and unverified.
        assert_eq!(
            continuation_decision(false, true, 0.10, 0, u64::MAX),
            ContinuationDecision::Accept
        );
        // Goal verified → never nudge.
        assert_eq!(
            continuation_decision(true, false, 0.10, 0, u64::MAX),
            ContinuationDecision::Accept
        );
        // No budget headroom (>= ceiling) → accept rather than nudge into the window wall.
        assert_eq!(
            continuation_decision(false, false, CONTINUATION_BUDGET_CEILING, 0, u64::MAX),
            ContinuationDecision::Accept
        );
        // Progress wins even when the diminishing-returns counters would otherwise stop.
        assert_eq!(
            continuation_decision(false, true, 0.10, CONTINUATION_MAX, 0),
            ContinuationDecision::Accept
        );
    }

    // ── compact's trivial-tier failover chain — pure decision, offline-unit-tested ─────────────
    fn routed(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn compact_candidate_chain_filters_benched_models() {
        let trivial = vec!["ollama::llama3.2".to_string(), "groq::fast".to_string()];
        let chain =
            compact_candidate_chain(trivial, routed(&["aux::fallback"]), "session::model", |m| {
                m == "ollama::llama3.2"
            });
        assert_eq!(chain, vec!["groq::fast", "aux::fallback", "session::model"]);
    }

    #[test]
    fn compact_candidate_chain_caps_trivial_at_three_then_appends_routed_and_guaranteed() {
        let trivial = vec![
            "a::one".to_string(),
            "b::two".to_string(),
            "c::three".to_string(),
            "d::four".to_string(),
        ];
        let chain = compact_candidate_chain(
            trivial,
            routed(&["aux::fallback"]),
            "session::model",
            |_| false,
        );
        assert_eq!(
            chain,
            vec![
                "a::one",
                "b::two",
                "c::three",
                "aux::fallback",
                "session::model"
            ]
        );
    }

    #[test]
    fn compact_candidate_chain_includes_routed_fallbacks_when_trivial_is_empty() {
        // Regression: an empty trivial shortlist must still fail over down the ROUTED chain (the
        // rate-limited-summarizer path), not collapse to just the guaranteed model.
        let chain = compact_candidate_chain(
            Vec::new(),
            routed(&["bad::model", "good::model"]),
            "bad::model",
            |_| false,
        );
        assert_eq!(chain, vec!["bad::model", "good::model"]);
    }

    #[test]
    fn compact_candidate_chain_does_not_duplicate_an_already_present_guaranteed_model() {
        let trivial = vec!["a::one".to_string(), "session::model".to_string()];
        let chain = compact_candidate_chain(
            trivial,
            routed(&["session::model"]),
            "session::model",
            |_| false,
        );
        assert_eq!(chain, vec!["a::one", "session::model"]);
    }

    #[test]
    fn fit_compaction_payload_keeps_both_ends_and_elides_the_middle() {
        let entries: Vec<String> = (0..40)
            .map(|i| format!("user: message {i} {}", "x ".repeat(400)))
            .collect();

        let untouched = fit_compaction_payload(&entries, usize::MAX);
        assert!(
            untouched.contains("message 20 "),
            "a payload that already fits must be passed through whole"
        );
        assert!(!untouched.contains("omitted"));

        let budget = 2_000;
        let fitted = fit_compaction_payload(&entries, budget);
        assert!(
            tokens::count_message(&fitted) <= budget,
            "fitted payload must respect the budget: {} > {budget}",
            tokens::count_message(&fitted)
        );
        assert!(
            fitted.contains("message 0 "),
            "the opening of the conversation anchors the summary and must survive"
        );
        assert!(
            fitted.contains("message 39 "),
            "the end of the older stretch is what the kept-recent messages continue from"
        );
        assert!(
            !fitted.contains("message 20 "),
            "the MIDDLE is what gets dropped — not the oldest, which is the point of compacting"
        );
        assert!(
            fitted.contains("omitted"),
            "the hole must be declared so the summarizer doesn't invent continuity"
        );
    }

    #[test]
    fn tool_failure_tracker_trips_at_threshold() {
        let mut tracker = ToolFailureTracker::new();

        assert!(tracker
            .record_failure("read_file", "permission denied")
            .is_none());
        assert!(tracker
            .record_failure("read_file", "permission denied")
            .is_none());
        let warning = tracker
            .record_failure("read_file", "permission denied")
            .expect("third matching failure should trip");

        assert!(warning.contains("stuck: `read_file` failed 3 times"));
        assert!(warning.contains("Permission"));
    }

    #[test]
    fn tool_failure_tracker_resets_on_success() {
        let mut tracker = ToolFailureTracker::new();

        assert!(tracker
            .record_failure("edit_file", "invalid patch")
            .is_none());
        assert!(tracker
            .record_failure("edit_file", "invalid patch")
            .is_none());
        tracker.record_success("edit_file");
        assert!(tracker
            .record_failure("edit_file", "invalid patch")
            .is_none());
        assert!(tracker
            .record_failure("edit_file", "invalid patch")
            .is_none());
    }

    #[test]
    fn doom_loop_tracker_trips_consecutive() {
        let mut tracker = ToolFailureTracker::new();

        assert!(tracker
            .record_call("shell", r#"{"command":"cargo check"}"#)
            .is_none());
        assert!(tracker
            .record_call("shell", r#"{"command":"cargo check"}"#)
            .is_none());
        let warning = tracker
            .record_call("shell", r#"{"command":"cargo check"}"#)
            .expect("third identical call should trip");

        assert!(warning.contains("doom-loop: `shell` called identically 3 times"));
    }

    #[test]
    fn doom_loop_resets_on_different_call() {
        let mut tracker = ToolFailureTracker::new();

        assert!(tracker
            .record_call("read_file", r#"{"path":"a"}"#)
            .is_none());
        assert!(tracker
            .record_call("read_file", r#"{"path":"b"}"#)
            .is_none());
        assert!(tracker
            .record_call("read_file", r#"{"path":"a"}"#)
            .is_none());
        assert!(tracker
            .record_call("read_file", r#"{"path":"a"}"#)
            .is_none());
    }

    #[test]
    fn fit_messages_keeps_everything_when_it_fits() {
        let msgs = vec![
            Message::system("rules"),
            Message::user("hi"),
            Message::assistant("hello"),
        ];
        assert_eq!(fit_messages(&msgs, 10_000).len(), 3);
    }

    #[test]
    fn prune_tool_results_trims_only_old_large_tool_output() {
        let big = "x".repeat(PRUNE_TOOL_RESULT_MAX + 500);
        let small = "ok".to_string();
        let mut msgs = vec![
            Message::user("do it"),                    // 0  (old)
            Message::tool_result("c1", big.clone()),   // 1  old + large  → pruned
            Message::tool_result("c2", small.clone()), // 2  old + small  → kept
            Message::assistant("working"),             // 3  protected window starts here (last 6)
            Message::tool_result("c3", big.clone()),   // 4  protected
            Message::user("more"),                     // 5
            Message::assistant("a"),                   // 6
            Message::user("b"),                        // 7
            Message::tool_result("c4", big.clone()),   // 8  recent + large → protected
        ];
        let reclaimed = prune_tool_results(&mut msgs, COMPACT_KEEP_RECENT);
        assert!(reclaimed > 0);
        assert!(msgs[1].content.ends_with(PRUNE_MARKER) && msgs[1].content.len() < big.len());
        assert_eq!(msgs[2].content, small, "small old result untouched");
        assert_eq!(
            msgs[4].content, big,
            "result inside the recent window protected"
        );
        assert_eq!(msgs[8].content, big, "most-recent result protected");
        // The pruned result keeps its tool_call_id (valid round-trip) and its role.
        assert_eq!(msgs[1].tool_call_id.as_deref(), Some("c1"));
        assert_eq!(msgs[1].role, Role::Tool);
        // Idempotent: a second pass reclaims nothing.
        assert_eq!(prune_tool_results(&mut msgs, COMPACT_KEEP_RECENT), 0);
    }

    #[test]
    fn fit_messages_keeps_system_and_recent_drops_oldest() {
        let msgs = vec![
            Message::system("SYS"),
            Message::user(format!("OLD {}", "a".repeat(500))),
            Message::user(format!("MID {}", "b".repeat(500))),
            Message::user("NEWEST request"),
        ];
        // Budget fits the system + the newest one or two, not the 500-char olds.
        let out = fit_messages(&msgs, 16 + 4 + 16 + "NEWEST request".len() + 16);
        assert_eq!(out[0].role, Role::System, "system always kept");
        assert!(
            out.iter().any(|m| m.content.contains("NEWEST")),
            "newest kept"
        );
        assert!(
            !out.iter().any(|m| m.content.contains("OLD")),
            "oldest dropped: {out:?}"
        );
        // System stays at the front; the surviving recent tail follows in order.
        assert_eq!(out.first().unwrap().content, "SYS");
    }

    #[test]
    fn fit_messages_truncates_a_single_oversized_message() {
        let msgs = vec![
            Message::system("SYS"),
            Message::user(format!("{}TAIL-WORDS", "z".repeat(5_000))),
        ];
        let out = fit_messages(&msgs, 200);
        let last = out.last().unwrap();
        assert!(
            last.content.contains("TAIL-WORDS"),
            "keeps the latest words"
        );
        assert!(last.content.contains("truncated"), "marks the cut");
        assert!(last.content.chars().count() < 5_000, "shrunk");
    }

    #[test]
    fn validate_tool_args_catches_missing_required_and_non_objects() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {"path": {}, "content": {}},
            "required": ["path", "content"]
        });
        assert!(
            validate_tool_args(&schema, &serde_json::json!({"path": "a", "content": "b"})).is_ok()
        );
        let err = validate_tool_args(&schema, &serde_json::json!({"path": "a"})).unwrap_err();
        assert!(err.contains("content"), "names the missing field: {err}");
        assert!(validate_tool_args(&schema, &serde_json::json!("nope")).is_err());
        let truncated = serde_json::json!({
            forge_provider::TRUNCATED_TOOL_ARGS_KEY: "native output limit interrupted the call"
        });
        let err = validate_tool_args(&schema, &truncated).unwrap_err();
        assert!(
            err.contains("output limit"),
            "preserves actionable cause: {err}"
        );
        // A schema with no `required` accepts any object.
        assert!(validate_tool_args(
            &serde_json::json!({"type": "object"}),
            &serde_json::json!({})
        )
        .is_ok());
    }

    #[test]
    fn fit_messages_drops_orphan_leading_tool_result() {
        // A trim that cuts between an assistant tool-call and its result must NOT leave the result
        // dangling (a tool_call_id with no call → the provider 400s the whole request). The leading
        // orphan tool result is dropped.
        let big = "context line ".repeat(400);
        let msgs = vec![
            Message::assistant_tool_calls(
                big,
                vec![forge_types::ToolCall {
                    id: "c1".into(),
                    name: "read_file".into(),
                    args: serde_json::json!({"path": "a.rs"}),
                }],
            ),
            Message::tool_result("c1", "the file contents"),
            Message::user("continue"),
        ];
        // Budget fits the tool result + the user turn, but not the big assistant before them.
        let budget = message_tokens(&msgs[1]) + message_tokens(&msgs[2]) + 4;
        let out = fit_messages(&msgs, budget);
        assert!(
            out.iter().all(|m| m.role != Role::Tool),
            "dangling tool result dropped: {:?}",
            out.iter().map(|m| m.role).collect::<Vec<_>>()
        );
        assert_eq!(out.last().unwrap().content, "continue");
    }

    #[test]
    fn request_includes_base_system_prompt_and_env() {
        let provider = Arc::new(FlakyProvider {
            bad: std::collections::HashSet::new(),
            err: rate_limited,
        });
        let router = Arc::new(FixedRouter {
            model: "m".into(),
            fallbacks: vec![],
        });
        let (_store, session) = fixed_session(provider, router);
        let msgs = session.transcript_with_preamble("m");
        assert_eq!(msgs[0].role, Role::System);
        assert!(
            msgs[0].content.contains("You are Forge"),
            "base coding-agent prompt is prepended"
        );
        assert!(msgs[1].content.contains("<env>"), "env block present");
        assert!(msgs[1].content.contains("platform:"));
    }

    #[tokio::test]
    async fn readonly_batch_runs_concurrently_and_preserves_order() {
        let provider = Arc::new(FlakyProvider {
            bad: std::collections::HashSet::new(),
            err: rate_limited,
        });
        let router = Arc::new(FixedRouter {
            model: "m".into(),
            fallbacks: vec![],
        });
        let dir = std::env::temp_dir().join(format!("forge-batch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config = Config::default();
        let mut session = Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            provider,
            router,
            ToolRegistry::with_core_tools_in(&dir),
            Box::new(HeadlessPresenter::new(false)),
            config,
            dir.to_str().expect("temporary workspace path is UTF-8"),
        )
        .unwrap();

        let mut calls = Vec::new();
        for i in 0..3 {
            let p = dir.join(format!("f{i}.txt"));
            std::fs::write(&p, format!("content-{i}")).unwrap();
            calls.push(forge_types::ToolCall {
                id: format!("c{i}"),
                name: "read_file".into(),
                args: serde_json::json!({ "path": p.to_str().unwrap() }),
            });
        }
        // All three reads qualify for the concurrent fast path.
        assert!(calls
            .iter()
            .all(|c| session.is_concurrent_readonly(&c.name)));

        let msg_id = session
            .store
            .add_message_full(session.id(), 0, Role::Assistant, "", None, &[], None)
            .unwrap();
        session.run_readonly_batch(&msg_id, &calls).await.unwrap();

        // Every call is answered, in the ORIGINAL order, paired by tool_call_id.
        let tools: Vec<&Message> = session
            .transcript
            .iter()
            .filter(|m| m.role == Role::Tool)
            .collect();
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0].tool_call_id.as_deref(), Some("c0"));
        assert!(tools[0].content.contains("content-0"));
        assert_eq!(tools[1].tool_call_id.as_deref(), Some("c1"));
        assert_eq!(tools[2].tool_call_id.as_deref(), Some("c2"));
        assert!(tools[2].content.contains("content-2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A presenter that records every event so tests can assert on what was shown.
    #[derive(Clone, Default)]
    struct CapturePresenter {
        events: Arc<Mutex<Vec<PresenterEvent>>>,
    }
    impl Presenter for CapturePresenter {
        fn emit(&mut self, event: PresenterEvent) {
            self.events.lock().unwrap().push(event);
        }
        fn confirm(
            &mut self,
            _tool: &str,
            _side_effect: SideEffect,
        ) -> forge_types::ConfirmOutcome {
            forge_types::ConfirmOutcome::Deny
        }
        fn ask(
            &mut self,
            _q: &str,
            options: &[forge_types::QChoice],
            _allow_other: bool,
        ) -> String {
            // Deterministic: pick the first option (or empty) so tests don't block on input.
            options.first().map(|o| o.label.clone()).unwrap_or_default()
        }
        fn read_line(&mut self) -> Option<String> {
            None
        }
    }

    /// A presenter whose `ask` always returns a scripted label, counting how many times it was
    /// asked — for the auto-compact-on-switch consent tests.
    #[derive(Clone)]
    struct ScriptedPresenter {
        answer: String,
        asks: Arc<Mutex<usize>>,
    }
    impl Presenter for ScriptedPresenter {
        fn emit(&mut self, _event: PresenterEvent) {}
        fn confirm(
            &mut self,
            _tool: &str,
            _side_effect: SideEffect,
        ) -> forge_types::ConfirmOutcome {
            forge_types::ConfirmOutcome::Allow
        }
        fn ask(
            &mut self,
            _q: &str,
            _options: &[forge_types::QChoice],
            _allow_other: bool,
        ) -> String {
            *self.asks.lock().unwrap() += 1;
            self.answer.clone()
        }
        fn read_line(&mut self) -> Option<String> {
            None
        }
    }

    fn scripted_session(answer: &str, asks: Arc<Mutex<usize>>) -> Session {
        let config = Config::default();
        Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(ScriptedPresenter {
                answer: answer.to_string(),
                asks,
            }),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn small_transcript_fits_any_window_no_prompt() {
        let asks = Arc::new(Mutex::new(0));
        let mut s = scripted_session("No", asks.clone());
        s.transcript.push(Message::user("hi there"));
        assert!(s.transcript_fits("ollama::tiny")); // unknown → 32k floor, easily fits
        assert!(
            s.admit_failover_model("ollama::tiny").await.unwrap(),
            "a fitting model is admitted"
        );
        assert_eq!(*asks.lock().unwrap(), 0, "no consent prompt when it fits");
    }

    #[tokio::test]
    async fn oversized_transcript_prompts_and_no_skips() {
        let asks = Arc::new(Mutex::new(0));
        let mut s = scripted_session("No", asks.clone());
        // One giant message: over 80% of the 32k floor in tokens, but too few messages for
        // compact() to do real work (so the gate's decision is what we're testing).
        s.transcript.push(Message::user("data ".repeat(40_000)));
        assert!(
            !s.transcript_fits("ollama::tiny"),
            "overflows the small window"
        );
        assert!(
            !s.admit_failover_model("ollama::tiny").await.unwrap(),
            "\"No\" skips the model"
        );
        assert_eq!(*asks.lock().unwrap(), 1, "asked exactly once");
    }

    #[tokio::test]
    async fn always_answer_silences_further_prompts() {
        let asks = Arc::new(Mutex::new(0));
        let mut s = scripted_session("Always", asks.clone());
        s.transcript.push(Message::user("data ".repeat(40_000)));
        assert!(
            s.admit_failover_model("ollama::tiny").await.unwrap(),
            "Always → admit"
        );
        assert!(s.always_compact_on_switch, "the session flag is set");
        // A second over-window switch proceeds silently (no further prompt).
        s.transcript.push(Message::user("data ".repeat(40_000)));
        assert!(s.admit_failover_model("ollama::tiny").await.unwrap());
        assert_eq!(*asks.lock().unwrap(), 1, "asked only the first time");
    }

    /// A provider that calls `ask_user` once, then answers using whatever came back.
    #[derive(Default)]
    struct AskingProvider;

    #[async_trait::async_trait]
    impl Provider for AskingProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_provider::ModelResponse;
            use forge_types::{new_id, ToolCall, Usage};
            let usage = Usage::default();
            if messages.iter().any(|m| m.role == Role::Tool) {
                return Ok(ModelResponse {
                    content: "done".into(),
                    tool_calls: vec![],
                    usage,
                    quotas: Vec::new(),
                });
            }
            Ok(ModelResponse {
                content: "asking".into(),
                tool_calls: vec![ToolCall {
                    id: new_id(),
                    name: "ask_user".into(),
                    args: serde_json::json!({
                        "question": "which database?",
                        "options": [{"label": "Postgres"}, {"label": "SQLite"}]
                    }),
                }],
                usage,
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn ask_user_round_trips_the_answer_into_the_turn() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(AskingProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            // CapturePresenter::ask returns the first option ("Postgres").
            Box::new(CapturePresenter::default()),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        let id = session.id().to_string();
        let answer = session.run_turn("set up the db").await.unwrap();
        assert_eq!(
            answer, "done",
            "turn completes after the question is answered"
        );
        // The chosen answer was fed back as the tool result.
        let tool_msgs: Vec<_> = store
            .load_messages(&id)
            .unwrap()
            .into_iter()
            .filter(|m| m.role == Role::Tool)
            .collect();
        assert!(
            tool_msgs.iter().any(|m| m.content == "Postgres"),
            "ask_user answer fed back as tool result: {tool_msgs:?}"
        );
    }

    /// A provider that calls the namespaced MCP tool `test__echo` once, then answers.
    #[derive(Default)]
    struct McpProvider;

    #[async_trait::async_trait]
    impl Provider for McpProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_provider::ModelResponse;
            use forge_types::{new_id, ToolCall, Usage};
            let usage = Usage::default();
            if messages.iter().any(|m| m.role == Role::Tool) {
                return Ok(ModelResponse {
                    content: "done".into(),
                    tool_calls: vec![],
                    usage,
                    quotas: Vec::new(),
                });
            }
            Ok(ModelResponse {
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: new_id(),
                    name: "mcp_call".into(),
                    args: serde_json::json!({ "name": "test__echo", "arguments": { "msg": "hi" } }),
                }],
                usage,
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn mcp_tools_are_advertised_and_routed_through_the_broker() {
        // A config that allowlists `test__echo` so it's eagerly exposed (advertised), in Bypass
        // mode so the External call auto-allows without a prompt.
        let mcp = forge_config::McpConfig {
            allow: forge_config::McpAllowlist {
                servers: vec!["test".into()],
                tools: vec!["test__echo".into()],
            },
            ..Default::default()
        };
        let config = Config {
            permission_mode: PermissionMode::Bypass,
            mcp: mcp.clone(),
            ..Config::default()
        };
        let mgr = std::sync::Arc::new(forge_mcp::testsupport::manager_with_echo(&mcp).await);

        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(McpProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(CapturePresenter::default()),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        session.set_mcp(Some(mgr));

        // tool_specs advertises the MCP meta-tools (search + call); server tools are reached
        // through mcp_call, never advertised individually.
        let names: Vec<String> = session.tool_specs().into_iter().map(|s| s.name).collect();
        assert!(names.iter().any(|n| n == "mcp_search_tools"));
        assert!(
            names.iter().any(|n| n == "mcp_call"),
            "mcp_call advertised: {names:?}"
        );
        assert!(
            names.iter().all(|n| n != "test__echo"),
            "server tool NOT advertised directly"
        );
        // …and built-ins are still there (additive, no regression).
        assert!(names.iter().any(|n| n == "read_file"));

        let id = session.id().to_string();
        let answer = session.run_turn("echo something").await.unwrap();
        assert_eq!(answer, "done");
        let tool_msgs: Vec<_> = store
            .load_messages(&id)
            .unwrap()
            .into_iter()
            .filter(|m| m.role == Role::Tool)
            .collect();
        assert!(
            tool_msgs.iter().any(|m| m.content == "echo: hi"),
            "MCP tool result fed back into the turn: {tool_msgs:?}"
        );
    }

    #[test]
    fn no_mcp_means_tool_specs_unchanged() {
        // Regression guard: with no manager attached, the advertised set has zero MCP entries.
        let store = Arc::new(Store::open_in_memory().unwrap());
        let session = Session::start(
            store,
            Arc::new(McpProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(CapturePresenter::default()),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        let names: Vec<String> = session.tool_specs().into_iter().map(|s| s.name).collect();
        assert!(names
            .iter()
            .all(|n| !n.starts_with("mcp_") && !n.contains("__")));
        assert!(
            names.windows(2).all(|pair| pair[0] <= pair[1]),
            "advertised tools must be deterministic for cross-process prompt caching: {names:?}"
        );
    }

    #[test]
    fn trivial_explicitly_tool_free_reply_hides_tools() {
        assert!(
            !Session::should_advertise_tools(
                TaskTier::Trivial,
                "Reply exactly: daemon-stability-check. Do not use tools."
            ),
            "a direct tool-free reply must not let a small model hallucinate an MCP call"
        );
        assert!(Session::should_advertise_tools(
            TaskTier::Trivial,
            "Read README.md and summarize the installation steps."
        ));
        assert!(Session::should_advertise_tools(
            TaskTier::Standard,
            "Reply exactly: daemon-stability-check. Do not use tools."
        ));
    }

    /// Provider that always calls `mcp_call { name: "test__echo", arguments: { "msg": "hi" } }`.
    /// Reused for the inner-gate deny test.
    struct McpCallEchoProvider;

    #[async_trait::async_trait]
    impl Provider for McpCallEchoProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_provider::ModelResponse;
            use forge_types::{new_id, ToolCall, Usage};
            if messages.iter().any(|m| m.role == Role::Tool) {
                return Ok(ModelResponse {
                    content: "done".into(),
                    tool_calls: vec![],
                    usage: Usage::default(),
                    quotas: Vec::new(),
                });
            }
            Ok(ModelResponse {
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: new_id(),
                    name: "mcp_call".into(),
                    args: serde_json::json!({
                        "name": "test__echo",
                        "arguments": { "msg": "hi" }
                    }),
                }],
                usage: Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn mcp_inner_tool_deny_rule_honored_on_direct_path() {
        // Bypass mode: the outer mcp_call wrapper is auto-allowed. A Configured deny rule
        // on the inner tool "test__echo" must still block the call so per-tool
        // allow/ask/deny rules are honored on the direct path (fix/mcp-percall-inner-gate).
        let mcp_cfg = forge_config::McpConfig {
            allow: forge_config::McpAllowlist {
                servers: vec!["test".into()],
                tools: vec!["test__echo".into()],
            },
            ..Default::default()
        };
        let deny_rule = forge_config::RuleConfig {
            tool: "test__echo".into(),
            deny: Some(forge_config::OneOrMany::One("*".into())),
            allow: None,
            ask: None,
            reason: None,
        };
        let config = Config {
            permission_mode: PermissionMode::Bypass,
            mcp: mcp_cfg.clone(),
            permissions: forge_config::PermissionsConfig {
                rules: vec![deny_rule],
            },
            ..Config::default()
        };

        let mgr = std::sync::Arc::new(forge_mcp::testsupport::manager_with_echo(&mcp_cfg).await);
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(McpCallEchoProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(CapturePresenter::default()),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        session.set_mcp(Some(mgr));

        let id = session.id().to_string();
        let _ = session.run_turn("call echo").await.unwrap();

        let tool_msgs: Vec<_> = store
            .load_messages(&id)
            .unwrap()
            .into_iter()
            .filter(|m| m.role == Role::Tool)
            .collect();
        assert!(
            tool_msgs
                .iter()
                .any(|m| m.content.contains("permission denied by policy")),
            "inner deny rule must block mcp_call on direct path; got: {tool_msgs:?}"
        );
        // Confirm the allowed tool (no deny rule) is NOT blocked — regression guard.
        assert!(
            tool_msgs.iter().all(|m| m.content != "echo: hi"),
            "denied tool must not produce output: {tool_msgs:?}"
        );
    }

    /// A provider that calls `update_tasks` once with a 2-item list, then finishes.
    #[derive(Default)]
    struct TaskingProvider;

    #[async_trait::async_trait]
    impl Provider for TaskingProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_provider::ModelResponse;
            use forge_types::{new_id, ToolCall, Usage};
            let usage = Usage::default();
            if messages.iter().any(|m| m.role == Role::Tool) {
                return Ok(ModelResponse {
                    content: "done".into(),
                    tool_calls: vec![],
                    usage,
                    quotas: Vec::new(),
                });
            }
            Ok(ModelResponse {
                content: "planning".into(),
                tool_calls: vec![ToolCall {
                    id: new_id(),
                    name: "update_tasks".into(),
                    args: serde_json::json!({"tasks": [
                        {"title": "design the api", "status": "done"},
                        {"title": "implement it", "status": "in_progress"}
                    ]}),
                }],
                usage,
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn update_tasks_sets_persists_and_emits_the_list() {
        use forge_types::TodoStatus;
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(TaskingProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        let id = session.id().to_string();

        session.run_turn("build the feature").await.unwrap();

        // Live state updated.
        assert_eq!(session.tasks().len(), 2);
        assert_eq!(session.tasks()[0].status, TodoStatus::Done);
        assert_eq!(session.tasks()[1].status, TodoStatus::InProgress);

        // Persisted for resume.
        let stored = store.tasks(&id).unwrap();
        assert_eq!(stored, session.tasks());

        // Emitted to the UI.
        let emitted = events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, PresenterEvent::Tasks(t) if t.len() == 2));
        assert!(emitted, "a Tasks event was emitted for the TUI");
    }

    /// A read-only completion that explains why no change is needed must not receive a redundant
    /// verification re-drive.
    struct VerifyByInspectingProvider {
        calls: std::sync::atomic::AtomicUsize,
    }
    #[async_trait::async_trait]
    impl Provider for VerifyByInspectingProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_provider::ModelResponse;
            use forge_types::{new_id, ToolCall, Usage};
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let usage = Usage::default();
            let read = || ToolCall {
                id: new_id(),
                name: "read_file".into(),
                args: serde_json::json!({"path": "Cargo.toml"}),
            };
            let resp = match n {
                // Read-only evidence + mark the only task Done.
                0 => ModelResponse {
                    content: "starting".into(),
                    tool_calls: vec![
                        read(),
                        ToolCall {
                            id: new_id(),
                            name: "update_tasks".into(),
                            args: serde_json::json!({"tasks": [{"title": "the task", "status": "done"}]}),
                        },
                    ],
                    usage,
                    quotas: Vec::new(),
                },
                // Completion explicitly explains that this read-only task needs no change.
                1 => ModelResponse {
                    content: "Goal complete: no changes are needed; Cargo.toml exists.".into(),
                    tool_calls: vec![],
                    usage,
                    quotas: Vec::new(),
                },
                _ => unreachable!("a read-only completion must not be re-driven"),
            };
            Ok(resp)
        }
    }

    #[tokio::test]
    async fn direct_gate_accepts_read_only_completion_without_redrive() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(VerifyByInspectingProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;

        session.run_turn("do the task").await.unwrap();

        let warnings: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                PresenterEvent::Warning(w) => Some(w.clone()),
                _ => None,
            })
            .collect();
        // The read-only inspection is sufficient evidence: never add a duplicate completion turn.
        assert!(
            !warnings
                .iter()
                .any(|w| w.contains("verifying with a real state check")),
            "a read-only completion must not be re-driven; warnings: {warnings:?}"
        );
        assert!(
            !warnings.iter().any(|w| w.contains("UNVERIFIED")),
            "a read-only completion must not be flagged UNVERIFIED; warnings: {warnings:?}"
        );
    }

    /// A prior read-only inspection plus a no-op explanation is sufficient completion evidence.
    struct ClaimsDoneNeverInspectsProvider {
        calls: std::sync::atomic::AtomicUsize,
    }
    #[async_trait::async_trait]
    impl Provider for ClaimsDoneNeverInspectsProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_provider::ModelResponse;
            use forge_types::{new_id, ToolCall, Usage};
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let usage = Usage::default();
            let resp = if n == 0 {
                ModelResponse {
                    content: "working".into(),
                    tool_calls: vec![
                        ToolCall {
                            id: new_id(),
                            name: "read_file".into(),
                            args: serde_json::json!({"path": "Cargo.toml"}),
                        },
                        ToolCall {
                            id: new_id(),
                            name: "update_tasks".into(),
                            args: serde_json::json!({"tasks": [{"title": "the task", "status": "done"}]}),
                        },
                    ],
                    usage,
                    quotas: Vec::new(),
                }
            } else {
                // The initial read is read-only completion evidence; later turns only state no change is needed.
                ModelResponse {
                    content: "no changes are required; it's already satisfied".into(),
                    tool_calls: vec![],
                    usage,
                    quotas: Vec::new(),
                }
            };
            Ok(resp)
        }
    }

    #[tokio::test]
    async fn direct_gate_accepts_prior_read_only_evidence() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(ClaimsDoneNeverInspectsProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        session.run_turn("do the task").await.unwrap();

        let warnings: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                PresenterEvent::Warning(w) => Some(w.clone()),
                _ => None,
            })
            .collect();
        assert!(
            !warnings.iter().any(|w| w.contains("UNVERIFIED")),
            "a prior read-only inspection plus a no-op explanation is sufficient completion evidence; warnings: {warnings:?}"
        );
    }

    // --- Stop-hook enforcement (Claude-Code "Stop hook can block stopping") ---

    /// A provider that always returns a final text answer with no tool calls, counting how many
    /// times it was called. Each model-loop run = one call, so the count == 1 + (stop continuations).
    #[derive(Default)]
    struct CountingFinalProvider {
        calls: std::sync::atomic::AtomicUsize,
    }
    #[async_trait::async_trait]
    impl Provider for CountingFinalProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(forge_provider::ModelResponse {
                content: "all done".into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    /// Config with a single `stop` hook running `command`, and recap/auto-memory off so the only
    /// provider calls are the model-loop runs (keeps the continuation count exact).
    fn stop_hook_config(command: &str) -> Config {
        let mut config = Config::default();
        config.recap.enabled = false;
        config.suggest.enabled = false;
        config.mesh.auto_memory = false;
        config.hooks = vec![forge_config::HookConfig {
            event: forge_config::HookEvent::Stop,
            matcher: None,
            command: command.into(),
            timeout_secs: 10,
            cc_compat: false,
        }];
        config
    }

    fn counting_session(
        provider: Arc<CountingFinalProvider>,
        config: Config,
        capture: CapturePresenter,
    ) -> Session {
        Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            provider,
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap()
    }

    // The block-once script inspects `stop_hook_active` on stdin — a shell-specific test, so Unix-only
    // (the cap and non-blocking tests below are cross-platform via plain exit codes).
    #[cfg(unix)]
    #[tokio::test]
    async fn stop_hook_block_once_triggers_one_continuation_then_proceeds() {
        let provider = Arc::new(CountingFinalProvider::default());
        // Blocks (exit 2) while stop_hook_active is false; approves (exit 0) once it's true — so the
        // turn re-runs exactly once, then stops. This is Claude Code's stop_hook_active loop-breaker.
        let config = stop_hook_config(r#"grep -q '"stop_hook_active":true' || exit 2"#);
        let mut session = counting_session(provider.clone(), config, CapturePresenter::default());
        session.run_turn("do the task").await.unwrap();
        assert_eq!(
            provider.calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "one block → exactly one extra model-loop run, then the turn proceeds"
        );
    }

    #[tokio::test]
    async fn stop_hook_consecutive_block_cap_is_enforced() {
        let provider = Arc::new(CountingFinalProvider::default());
        let config = stop_hook_config("exit 2"); // always blocks (cross-platform: sh & cmd both exit 2)
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = counting_session(provider.clone(), config, capture);
        session.run_turn("do the task").await.unwrap();
        // primary + MAX_STOP_BLOCKS (3) continuations = 4 model-loop runs, then a forced stop.
        assert_eq!(
            provider.calls.load(std::sync::atomic::Ordering::SeqCst),
            4,
            "the safety cap bounds continuations so an always-blocking hook can't wedge the turn"
        );
        let warnings: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                PresenterEvent::Warning(w) => Some(w.clone()),
                _ => None,
            })
            .collect();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("forcing the turn to end")),
            "a force-stop warning must be surfaced when the cap is hit; warnings: {warnings:?}"
        );
    }

    #[tokio::test]
    async fn stop_hook_non_blocking_does_not_continue() {
        let provider = Arc::new(CountingFinalProvider::default());
        let config = stop_hook_config("exit 0"); // observe-only: never blocks
        let mut session = counting_session(provider.clone(), config, CapturePresenter::default());
        session.run_turn("do the task").await.unwrap();
        assert_eq!(
            provider.calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a non-blocking stop hook leaves the turn unaffected (no continuation)"
        );
    }

    /// Always issues the exact same tool call (a fresh id each time, but identical name + args, so
    /// `tool_batch_signature` sees a repeat). Models a stuck model re-reading the same file forever.
    struct DoomLoopProvider;
    #[async_trait::async_trait]
    impl Provider for DoomLoopProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_types::{new_id, ToolCall, Usage};
            Ok(forge_provider::ModelResponse {
                content: "let me read it again".into(),
                tool_calls: vec![ToolCall {
                    id: new_id(),
                    name: "read_file".into(),
                    args: serde_json::json!({"path": "Cargo.toml"}),
                }],
                usage: Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn doom_loop_halts_a_model_repeating_the_same_call() {
        // The doom-loop guard must stop a model that emits the EXACT same tool call step after step
        // (identical args → identical result → no progress) rather than burning the whole step budget
        // + quota. It nudges once to change approach, then halts loudly if the repeat continues.
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(DoomLoopProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        // Must RETURN (not hang / not run forever); the guard breaks the loop.
        session.run_turn("read the file").await.unwrap();

        let warnings: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                PresenterEvent::Warning(w) | PresenterEvent::Error(w) => Some(w.clone()),
                _ => None,
            })
            .collect();
        // The guard fired: first a "change approach" nudge, then a loud halt — assert the halt so we
        // know it actually STOPPED the loop (not merely nudged and then hit the step cap).
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("kept repeating the same tool call")),
            "the doom-loop guard should halt a repeating model; warnings: {warnings:?}"
        );
    }

    #[tokio::test]
    async fn doom_loop_halt_is_not_restarted_by_empty_diff_recovery() {
        // A hard loop halt is terminal for this turn. The outer empty-diff recovery must not treat
        // it as an ordinary model sign-off and start fresh model loops with reset guard state.
        let dir = clean_git_repo();
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(DoomLoopProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.workspace = WorkspaceContext::new(&dir).unwrap();
        session.set_expect_code_change(true);

        session.run_turn("fix the bug").await.unwrap();

        let warnings: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                PresenterEvent::Warning(message) | PresenterEvent::Error(message) => {
                    Some(message.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            warnings
                .iter()
                .filter(|message| message.contains("kept repeating the same tool call"))
                .count(),
            1,
            "a hard loop halt must be surfaced once; warnings: {warnings:?}"
        );
        assert!(
            !warnings
                .iter()
                .any(|message| message.contains("empty diff")),
            "empty-diff recovery must not restart a loop-guard halt; warnings: {warnings:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn doom_loop_halt_is_not_restarted_by_blocking_stop_hook() {
        // Stop hooks still run for lifecycle observation, but a blocking hook cannot override a
        // hard model-loop halt and re-enter the same guarded turn with fresh counters.
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut config = stop_hook_config("exit 2");
        config.recap.enabled = false;
        config.suggest.enabled = false;
        config.mesh.auto_memory = false;
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(DoomLoopProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        session.run_turn("read the file").await.unwrap();

        let warnings: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                PresenterEvent::Warning(message) | PresenterEvent::Error(message) => {
                    Some(message.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            warnings
                .iter()
                .filter(|message| message.contains("kept repeating the same tool call"))
                .count(),
            1,
            "a blocking Stop hook must not restart a hard loop halt; warnings: {warnings:?}"
        );
        assert!(
            !warnings
                .iter()
                .any(|message| message.contains("stop hook requested continuation")),
            "a hard loop halt must override Stop-hook continuation; warnings: {warnings:?}"
        );
    }

    /// Alternates two DIFFERENT calls forever: a failing read of a missing path, then a succeeding
    /// read of a real file. Neither the consecutive doom-loop (each step differs from the one before)
    /// NOR the failure-loop (the interleaved success clears the read_file failure streak) can see it —
    /// only the oscillation window catches the A,B,A,B cycle. Models the real bug where a model
    /// alternated an empty failing `shell({})` with a trivial `ls -la`, looping until timeout.
    struct OscillatingProvider {
        calls: std::sync::atomic::AtomicUsize,
    }
    #[async_trait::async_trait]
    impl Provider for OscillatingProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_types::{new_id, ToolCall, Usage};
            let n = self
                .calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let args = if n.is_multiple_of(2) {
                serde_json::json!({"path": "does-not-exist-xyz.txt"}) // fails NotFound
            } else {
                serde_json::json!({"path": "Cargo.toml"}) // succeeds → clears failure streak
            };
            Ok(forge_provider::ModelResponse {
                content: "still poking at it".into(),
                tool_calls: vec![ToolCall {
                    id: new_id(),
                    name: "read_file".into(),
                    args,
                }],
                usage: Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn doom_loop_halts_a_model_oscillating_between_two_calls() {
        // Regression for the alternation-evasion bug: a model that ping-pongs between a failing call
        // and a succeeding one evades BOTH the consecutive doom-loop (no two steps alike) and the
        // failure-loop (the success clears the failure streak), so without the oscillation window it
        // runs to the step cap / timeout. The guard must still halt it.
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(OscillatingProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        session.run_turn("keep going").await.unwrap();

        let warnings: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                PresenterEvent::Warning(w) | PresenterEvent::Error(w) => Some(w.clone()),
                _ => None,
            })
            .collect();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("kept alternating between the same tool calls")),
            "the oscillation guard should halt a model ping-ponging between two calls with an \
             ALTERNATING-specific message; warnings: {warnings:?}"
        );
    }

    /// Reads a UNIQUE non-existent path each call. Every call fails the same WAY (`NotFound`) but with
    /// DIFFERENT args, so the identical-call doom-loop never fires — only the failure-loop guard,
    /// which tracks failures by (tool, error-kind) across the turn, can catch it.
    struct FailureLoopProvider {
        calls: std::sync::atomic::AtomicUsize,
    }
    #[async_trait::async_trait]
    impl Provider for FailureLoopProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_types::{new_id, ToolCall, Usage};
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(forge_provider::ModelResponse {
                content: "let me try a different file".into(),
                tool_calls: vec![ToolCall {
                    id: new_id(),
                    name: "read_file".into(),
                    args: serde_json::json!({"path": format!("does-not-exist-{n}.rs")}),
                }],
                usage: Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn failure_loop_halts_a_model_failing_the_same_way() {
        // The failure-loop guard must stop a model that keeps hitting the same KIND of error with
        // different arguments (edits that never match, reads of paths that don't exist) — which the
        // identical-call doom-loop can't see, because the call signature keeps changing.
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(FailureLoopProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        session.run_turn("find the config").await.unwrap();

        let warnings: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                PresenterEvent::Warning(w) => Some(w.clone()),
                _ => None,
            })
            .collect();
        assert!(
            warnings.iter().any(|w| w.contains("kept failing") && w.contains("after a nudge")),
            "the failure-loop guard should halt a model failing the same way; warnings: {warnings:?}"
        );
    }

    #[test]
    fn auxiliary_calls_escape_explicit_subscription_pin() {
        let mut config = Config::default();
        config.mesh.models.insert(
            TaskTier::Trivial.as_str().to_string(),
            forge_config::OneOrMany::Many(vec!["ollama::qwen3:4b".into()]),
        );
        let store = Arc::new(Store::open_in_memory().unwrap());
        let session = Session::start(
            store,
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        let pinned = forge_mesh::RoutingDecision {
            tier: TaskTier::Trivial,
            model: "codex-oauth::gpt-5.6-sol".into(),
            rationale: "explicit pin".into(),
            fallbacks: vec![],
            pinned: true,
        };
        let routed = forge_mesh::RoutingDecision {
            pinned: false,
            ..pinned.clone()
        };
        assert_eq!(session.auxiliary_model(&pinned), "ollama::qwen3:4b");
        assert_eq!(session.auxiliary_model(&routed), "codex-oauth::gpt-5.6-sol");
        assert_eq!(
            session.post_turn_auxiliary_model(&pinned).as_deref(),
            Some("ollama::qwen3:4b"),
            "a lightweight configured model remains suitable for optional side calls"
        );

        let claude_bridge = forge_mesh::RoutingDecision {
            model: "claude-cli::sonnet".into(),
            pinned: false,
            ..pinned
        };
        assert_eq!(
            session.post_turn_auxiliary_model(&claude_bridge),
            None,
            "a completed turn must not launch another full CLI agent for optional side work"
        );
    }

    #[test]
    fn auxiliary_calls_use_low_effort_stable_cache_keys_and_bounded_outputs() {
        let opts = Session::auxiliary_completion_options("session-7", "shell-diagnose");
        assert_eq!(opts.effort, Some(EffortLevel::Low));
        assert_eq!(
            opts.prompt_cache_key.as_deref(),
            Some("session-7:shell-diagnose")
        );
        assert_eq!(opts.max_output_tokens, Some(256));
        assert!(opts.response_format.is_none());

        assert_eq!(
            Session::auxiliary_completion_options("session-7", "recap").max_output_tokens,
            Some(128)
        );
        assert_eq!(
            Session::auxiliary_completion_options("session-7", "suggest").max_output_tokens,
            Some(128)
        );
        assert_eq!(
            Session::auxiliary_completion_options("session-7", "memory").max_output_tokens,
            Some(256)
        );
        assert_eq!(
            Session::auxiliary_completion_options("session-7", "compact").max_output_tokens,
            None,
            "conversation summaries scale with history and must keep the native limit"
        );
    }

    #[test]
    fn completion_verification_empty_only_accepts_completed_turn_with_prior_answer() {
        let done = vec![forge_types::TodoItem {
            title: "ship".into(),
            status: forge_types::TodoStatus::Done,
            assignee: None,
        }];
        let open = vec![forge_types::TodoItem {
            title: "ship".into(),
            status: forge_types::TodoStatus::InProgress,
            assignee: None,
        }];

        assert!(completion_verification_empty_is_terminal(1, &done, true));
        assert!(!completion_verification_empty_is_terminal(0, &done, true));
        assert!(!completion_verification_empty_is_terminal(1, &done, false));
        assert!(!completion_verification_empty_is_terminal(1, &open, true));
    }

    #[test]
    fn completion_claims_no_change_recognizes_no_op_justifications() {
        assert!(completion_claims_no_change(
            "Goal complete: no changes are needed because README.md already exists."
        ));
        assert!(completion_claims_no_change(
            "No fix is applicable; the request is already satisfied."
        ));
        assert!(!completion_claims_no_change(
            "Goal complete: implemented the requested change."
        ));
    }

    #[test]
    fn completion_followup_intent_catches_open_ended_agent_promises_only() {
        assert!(completion_promises_followup(
            "Syntax passes. Let me verify a few runtime issues that syntax would not catch:"
        ));
        assert!(completion_promises_followup(
            "The file is written. I'll now run the browser checks..."
        ));
        assert!(!completion_promises_followup(
            "Implemented and verified the fix: all targeted tests pass."
        ));
        assert!(!completion_promises_followup("What changed:"));
        assert!(!completion_promises_followup(
            "I will keep this constraint in mind."
        ));
    }

    #[test]
    fn completion_gate_accepts_read_only_and_bounds_unverified_claims() {
        const MAX: usize = 1;
        // An explicit "no change needed" completion is accepted immediately — the read-only escape.
        assert_eq!(
            completion_gate(0, MAX, true, true, false),
            CompletionGate::AcceptNoArtifacts
        );
        assert_eq!(
            completion_gate(0, MAX, true, true, true),
            CompletionGate::AcceptClean
        );
        // A bare reasoning-only claim (no no_change statement) must survive ONE forced pass first,
        // then be accepted calmly — it does NOT short-circuit at attempt 0.
        assert_eq!(
            completion_gate(0, MAX, false, false, false),
            CompletionGate::Reverify
        );
        assert_eq!(
            completion_gate(1, MAX, false, false, false),
            CompletionGate::AcceptNoArtifacts
        );
        // Work that produced state is verified once, then flagged UNVERIFIED if never re-checked.
        assert_eq!(
            completion_gate(0, MAX, true, false, false),
            CompletionGate::Reverify
        );
        assert_eq!(
            completion_gate(1, MAX, true, false, false),
            CompletionGate::AcceptUnverified
        );
    }

    #[test]
    fn observational_scopes_are_terminal_and_cannot_request_implementation() {
        for intent in [
            TaskIntent::ReadOnlyReview,
            TaskIntent::PlanOnly,
            TaskIntent::Verification,
        ] {
            assert_ne!(
                post_check_decision(intent, 0, true, false, false),
                PostCheckDecision::RequestObservation,
                "{intent:?} completion must not be re-driven"
            );
        }
    }

    #[test]
    fn observational_scope_denies_mutating_capabilities() {
        let scope = TaskScope::for_test(
            "audit the current implementation",
            TaskIntent::ReadOnlyReview,
            PermissionMode::Bypass,
            7,
            Some(std::path::PathBuf::from("/repo")),
        );
        for tool in [
            "write_file",
            "shell",
            "spawn_agents",
            "run_workflow",
            "update_tasks",
        ] {
            assert!(!scope.permits_tool(tool), "{tool} must be denied");
        }
        assert!(scope.permits_tool("read_file"));
    }

    /// Yields TWO read_file calls (a concurrent read-only batch) with DIFFERENT missing paths every
    /// step — so the identical-call doom-loop never fires (signature changes) and, before the fix,
    /// the concurrent batch path didn't feed the failure-loop guard either, letting it burn to the
    /// step cap. The failure-loop guard must now catch it.
    struct ConcurrentFailureProvider {
        calls: std::sync::atomic::AtomicUsize,
    }
    #[async_trait::async_trait]
    impl Provider for ConcurrentFailureProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_types::{new_id, ToolCall, Usage};
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mk = |suffix: &str| ToolCall {
                id: new_id(),
                name: "read_file".into(),
                args: serde_json::json!({"path": format!("does-not-exist-{n}-{suffix}.rs")}),
            };
            Ok(forge_provider::ModelResponse {
                content: "reading two more files".into(),
                tool_calls: vec![mk("a"), mk("b")],
                usage: Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn concurrent_batch_failure_loop_is_caught() {
        // Regression for the concurrent-batch failure-tracking gap: two read_file calls run as a
        // concurrent read-only batch, both NotFound, different paths each step. Must halt via the
        // failure-loop guard, not run to the step cap.
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(ConcurrentFailureProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        session.run_turn("read the files").await.unwrap();

        let warnings: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                PresenterEvent::Warning(w) => Some(w.clone()),
                _ => None,
            })
            .collect();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("kept failing") && w.contains("after a nudge")),
            "the failure-loop guard must catch a concurrent batch failing the same way; warnings: {warnings:?}"
        );
    }

    /// Yields the SAME two successful read-only calls every step (a concurrent batch with a constant
    /// signature) — trips the doom-loop, not the failure-loop. Used to prove the nudge is delivered.
    struct ConcurrentRepeatProvider;
    #[async_trait::async_trait]
    impl Provider for ConcurrentRepeatProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_types::{new_id, ToolCall, Usage};
            let mk = || ToolCall {
                id: new_id(),
                name: "read_file".into(),
                args: serde_json::json!({"path": "Cargo.toml"}),
            };
            Ok(forge_provider::ModelResponse {
                content: "reading again".into(),
                tool_calls: vec![mk(), mk()],
                usage: Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn concurrent_batch_doom_nudge_is_delivered_to_the_model() {
        // Regression: the doom-loop nudge is pushed to pending_hints, but the concurrent read-only
        // batch path didn't drain them — so the model was halted "after a nudge" it never saw. The
        // nudge must reach the transcript.
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(ConcurrentRepeatProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        session.run_turn("read it").await.unwrap();

        assert!(
            session.transcript.iter().any(|m| m.role == Role::System
                && m.content.contains("cycled through the same tool calls")),
            "the doom-loop nudge must be delivered to the transcript on the concurrent batch path"
        );
    }

    /// Yields a tool call every single step forever (unique args so no doom/failure guard fires) —
    /// only the step cap can stop it.
    struct EndlessToolProvider;
    #[async_trait::async_trait]
    impl Provider for EndlessToolProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_types::{new_id, ToolCall, Usage};
            Ok(forge_provider::ModelResponse {
                content: "still working".into(),
                tool_calls: vec![ToolCall {
                    id: new_id(),
                    // A real successful read each step with a UNIQUE range → no doom/failure guard,
                    // forcing the step cap to be the thing that stops the turn.
                    name: "read_file".into(),
                    args: serde_json::json!({"path": "Cargo.toml", "start_line": 1, "end_line": 1}),
                }],
                usage: Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn step_cap_halts_a_runaway_turn() {
        // The step cap is the primary infinite-loop backstop. Pin it: with max_steps=2 and a model
        // that always wants another tool call, the turn must stop at the cap (not spin to default 100).
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut config = Config::default();
        config.mesh.max_steps = 2;
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(EndlessToolProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        // Must RETURN (the cap stops it) rather than loop forever.
        session.run_turn("keep reading").await.unwrap();

        let warnings: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                PresenterEvent::Warning(w) => Some(w.clone()),
                _ => None,
            })
            .collect();
        assert!(
            warnings.iter().any(|w| w.contains("step limit")),
            "the step cap should stop a runaway turn; warnings: {warnings:?}"
        );
    }

    /// Stalls on the 2nd call (text, no tool call) while a task is still in_progress, then — once
    /// the harness nudges it to continue — marks the task Done and finishes.
    struct StallThenFinishProvider {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Provider for StallThenFinishProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_provider::ModelResponse;
            use forge_types::{new_id, ToolCall, Usage};
            use std::sync::atomic::Ordering;
            let usage = Usage::default();
            let task = |status: &str| {
                vec![ToolCall {
                    id: new_id(),
                    name: "update_tasks".into(),
                    args: serde_json::json!({"tasks": [{"title": "do the thing", "status": status}]}),
                }]
            };
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            let resp = match n {
                0 => ModelResponse {
                    content: "starting".into(),
                    tool_calls: task("in_progress"),
                    usage,
                    quotas: Vec::new(),
                },
                // Premature stall: narrates, no tool call, task still unfinished. The harness must
                // NOT accept this as the final answer — it should nudge and drive on.
                1 => ModelResponse {
                    content: "I'll keep going on this.".into(),
                    tool_calls: vec![],
                    usage,
                    quotas: Vec::new(),
                },
                2 => ModelResponse {
                    content: "finishing".into(),
                    tool_calls: task("done"),
                    usage,
                    quotas: Vec::new(),
                },
                _ => ModelResponse {
                    content: "all done".into(),
                    tool_calls: vec![],
                    usage,
                    quotas: Vec::new(),
                },
            };
            Ok(resp)
        }
    }

    #[tokio::test]
    async fn harness_drives_on_when_model_stalls_with_unfinished_tasks() {
        use forge_types::TodoStatus;
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(StallThenFinishProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        let answer = session.run_turn("do the thing").await.unwrap();

        // The turn did NOT end at the stall — it continued until the task was Done.
        assert_eq!(
            answer, "all done",
            "drove past the premature text-only stall"
        );
        assert_eq!(session.tasks().len(), 1);
        assert_eq!(session.tasks()[0].status, TodoStatus::Done);
        // A continue-nudge was surfaced.
        let nudged = events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, PresenterEvent::Warning(w) if w.contains("unfinished")));
        assert!(
            nudged,
            "emitted a continue-nudge warning for the unfinished task"
        );
    }

    /// Registers a task in_progress on call 0, then narrates with NO tool call forever — the task
    /// never closes, so the continue-nudge budget is spent and the turn must give up (not loop).
    struct NeverFinishesProvider {
        calls: std::sync::atomic::AtomicUsize,
    }
    #[async_trait::async_trait]
    impl Provider for NeverFinishesProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_types::{new_id, ToolCall, Usage};
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let tool_calls = if n == 0 {
                vec![ToolCall {
                    id: new_id(),
                    name: "update_tasks".into(),
                    args: serde_json::json!({"tasks": [{"title": "do the thing", "status": "in_progress"}]}),
                }]
            } else {
                Vec::new() // narrate, never finish
            };
            Ok(forge_provider::ModelResponse {
                content: "still working on it".into(),
                tool_calls,
                usage: Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn direct_continue_nudge_exhaustion_warns_when_giving_up() {
        // Regression for a SILENT exit: when a direct model narrates forever with a task still open,
        // the harness nudges it a bounded number of times then GIVES UP. That give-up must be
        // surfaced (the bridge path always warned; the direct path used to fall through silently,
        // leaving the user to wonder why the turn stopped mid-plan).
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(NeverFinishesProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        session.run_turn("do the thing").await.unwrap();

        let warnings: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                PresenterEvent::Warning(w) => Some(w.clone()),
                _ => None,
            })
            .collect();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("giving up") && w.contains("unfinished")),
            "exhausting the continue-nudge budget must surface a give-up warning; warnings: {warnings:?}"
        );
    }

    #[tokio::test]
    async fn cli_bridge_no_progress_stall_halts_loudly_without_spiraling() {
        use forge_types::TodoStatus;
        // A CLI-bridge turn that yields with a task still unfinished AND made no progress on that
        // turn (no tool ran, no task closed) must HALT — not be re-driven into a narration loop
        // (the old spiral). But it must NOT pretend success: it stops LOUDLY, naming the unfinished
        // work, so the half-done state is visible. (A bridge that DID make progress is re-driven to
        // completion — see the `bridge_re_drives_*` tests.)
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(StallThenFinishProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            Arc::new(FixedRouter {
                model: "claude-cli::opus".into(),
                fallbacks: vec![],
            }),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        let answer = session.run_turn("do the thing").await.unwrap();

        // The stall (call 1) made no progress, so the turn ends there — NOT driven into a loop.
        assert_eq!(answer, "I'll keep going on this.");
        assert_eq!(session.tasks()[0].status, TodoStatus::InProgress);
        // ...but it halted LOUDLY: an honest "stopped with unfinished tasks" warning was surfaced.
        let warned_unfinished = events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, PresenterEvent::Warning(w) if w.contains("unfinished")));
        assert!(
            warned_unfinished,
            "a half-done bridge turn must stop loudly, not silently report success"
        );
    }

    /// Bridge provider for the completeness conformance test: call 0 runs a read-only tool (so the
    /// turn did real work), then every later call yields (content, no tool call) — the model thinks
    /// it's done.
    struct CompletenessYieldProvider {
        calls: std::sync::atomic::AtomicUsize,
    }
    #[async_trait::async_trait]
    impl Provider for CompletenessYieldProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            let n = self
                .calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let tool_calls = if n == 0 {
                vec![forge_types::ToolCall {
                    id: "1".into(),
                    name: "read_file".into(),
                    args: serde_json::json!({ "path": "Cargo.toml" }),
                }]
            } else {
                vec![]
            };
            Ok(forge_provider::ModelResponse {
                content: if n == 0 {
                    "checking".into()
                } else {
                    "all done".into()
                },
                tool_calls,
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn completeness_redrive_fires_once_when_verify_completeness_on() {
        // Opt-in `mesh.verify_completeness`: when a CLI-bridge turn that did real work yields, the
        // harness injects ONE completeness re-drive (a final diff-review nudge) before accepting done,
        // and only ONCE — the `completeness_checked` one-shot guard prevents a loop.
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut config = Config::default();
        config.mesh.verify_completeness = true;
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(CompletenessYieldProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            Arc::new(FixedRouter {
                model: "claude-cli::opus".into(),
                fallbacks: vec![],
            }),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        let _ = session.run_turn("fix the bug").await.unwrap();

        let fired = events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, PresenterEvent::Warning(w) if w.contains("completeness check")))
            .count();
        assert_eq!(
            fired, 1,
            "completeness re-drive must fire exactly once (one-shot)"
        );
    }

    #[tokio::test]
    async fn completeness_redrive_silent_when_verify_completeness_off() {
        // Explicit opt-out: no completeness re-drive when the quality policy is disabled.
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let base_mesh = Config::default().mesh;
        let config = Config {
            mesh: forge_config::MeshConfig {
                verify_completeness: false,
                ..base_mesh
            },
            ..Config::default()
        };
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(CompletenessYieldProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            Arc::new(FixedRouter {
                model: "claude-cli::opus".into(),
                fallbacks: vec![],
            }),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        let _ = session.run_turn("fix the bug").await.unwrap();

        let fired =
            events.lock().unwrap().iter().any(
                |e| matches!(e, PresenterEvent::Warning(w) if w.contains("completeness check")),
            );
        assert!(
            !fired,
            "completeness must not fire when verify_completeness is off"
        );
    }

    /// Always returns an empty response (no text, no tool call) — a model glitch / narrate-then-stall.
    struct EmptyResponseProvider;
    #[async_trait::async_trait]
    impl Provider for EmptyResponseProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            Ok(forge_provider::ModelResponse {
                content: String::new(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn empty_response_is_nudged_then_stops_not_loops() {
        // A response with neither text nor a tool call is a silent dead-end. The harness nudges it to
        // continue a BOUNDED number of times (so a transient glitch recovers), then stops — it must
        // never spin forever on an endlessly-empty model.
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(EmptyResponseProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        // Must RETURN — an always-empty model must not loop to the step cap or hang.
        session.run_turn("do something").await.unwrap();

        let warnings: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                PresenterEvent::Warning(w) | PresenterEvent::Error(w) => Some(w.clone()),
                _ => None,
            })
            .collect();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("empty response") && w.contains("nudging")),
            "an empty response should be nudged; warnings: {warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("stopping the turn")),
            "after the bounded nudges, an endlessly-empty model must stop; warnings: {warnings:?}"
        );
    }

    struct EmptyThenRoleStrictProvider {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Provider for EmptyThenRoleStrictProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call > 0 && messages.last().map(|m| m.role) != Some(Role::User) {
                return Err(forge_provider::ProviderError::Request(
                    "last message role must be 'user'".into(),
                ));
            }
            Ok(forge_provider::ModelResponse {
                content: if call == 0 {
                    String::new()
                } else {
                    "recovered after empty response".into()
                },
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn empty_response_recovery_ends_with_a_user_message() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = Session::start(
            store,
            Arc::new(EmptyThenRoleStrictProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        let answer = session.run_turn("do something").await.unwrap();
        assert_eq!(answer, "recovered after empty response");
    }

    /// Empty (no text/tool) for the `bad` models, echoes the model id otherwise — to prove an
    /// empty-responding model FAILS OVER to the next chain model instead of dead-ending the turn.
    struct EmptyForModelProvider {
        bad: std::collections::HashSet<String>,
    }
    #[async_trait::async_trait]
    impl Provider for EmptyForModelProvider {
        async fn complete(
            &self,
            model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            if self.bad.contains(model) {
                return Ok(forge_provider::ModelResponse {
                    content: String::new(),
                    tool_calls: vec![],
                    usage: forge_types::Usage::default(),
                    quotas: Vec::new(),
                });
            }
            on_event(StreamEvent::Text(model.into()));
            Ok(forge_provider::ModelResponse {
                content: model.into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn empty_response_fails_over_to_the_next_model() {
        // Dogfooding bug: an empty-responding model (e.g. kimi-k2.6 via NIM streaming empty) used to
        // stop the turn after the bounded nudges, dead-ending short of a working model. It must now
        // bench the empty model and FAIL OVER to the next chain model instead.
        let provider = Arc::new(EmptyForModelProvider {
            bad: ["empty::model".to_string()].into_iter().collect(),
        });
        let router = Arc::new(FixedRouter {
            model: "empty::model".into(),
            fallbacks: vec!["good::model".into()],
        });
        let (store, mut session) = fixed_session(provider, router);
        let answer = session.run_turn("do it").await.unwrap();
        assert_eq!(
            answer, "good::model",
            "an empty response must fail over to the next model, not stop the turn"
        );
        assert!(
            store.current_benched().unwrap().is_benched("empty::model"),
            "the empty-responding model must be benched"
        );
    }

    /// Writes a tool call as TEXT (markup the provider didn't decode into a structured call) with NO
    /// real tool_calls — so nothing executes. Models the phantom-release failure mode.
    struct ToolCallAsTextProvider;
    #[async_trait::async_trait]
    impl Provider for ToolCallAsTextProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            Ok(forge_provider::ModelResponse {
                // `<invoke …>` markup is detected by `looks_like_unexecuted_tool_call`, but with no
                // structured `tool_calls` it never runs — the honest-failure guard must catch it.
                content: "I'll do it now: <invoke name=\"shell\">git push</invoke>".into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn tool_call_written_as_text_never_silently_succeeds() {
        // A model that writes a tool call as text (and the provider didn't recover it) did NOTHING —
        // accepting that narration as a final answer is the phantom-success bug. The honest-failure
        // guard must nudge it to actually call the tool, then — if it persists — end LOUDLY rather
        // than report success.
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(ToolCallAsTextProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        session.run_turn("push the commit").await.unwrap();

        let warnings: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                PresenterEvent::Warning(w) => Some(w.clone()),
                _ => None,
            })
            .collect();
        assert!(
            warnings.iter().any(|w| w.contains("tool call as text")),
            "a narrated tool call should be nudged to actually execute; warnings: {warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("never executed")),
            "if it persists, the turn must end loudly (not a phantom success); warnings: {warnings:?}"
        );
    }

    #[test]
    fn parse_plan_reads_fields_and_filters_empty_steps() {
        let v = serde_json::json!({
            "title": "  Refactor main.rs  ",
            "steps": [
                {"title": "Extract args", "detail": "  clap defs  "},
                {"title": "   "},
                {"title": "Split dispatch"}
            ],
            "notes": "  keep the API stable  "
        });
        let p = parse_plan(&v);
        assert_eq!(p.title, "Refactor main.rs");
        assert_eq!(p.steps.len(), 2, "the blank-title step is dropped");
        assert_eq!(p.steps[0].title, "Extract args");
        assert_eq!(p.steps[0].detail, "clap defs");
        assert_eq!(p.steps[1].detail, "");
        assert_eq!(p.notes.as_deref(), Some("keep the API stable"));

        let empty = parse_plan(&serde_json::json!({}));
        assert_eq!(empty.title, "Plan");
        assert!(empty.steps.is_empty());
        assert!(empty.notes.is_none());
    }

    #[test]
    fn parse_tasks_accepts_bridge_title_aliases_and_preserves_explicit_clear() {
        let tasks = parse_tasks(&serde_json::json!({
            "tasks": [
                {"title": "  canonical  ", "status": "pending"},
                {"content": "Codex shape", "status": "in_progress"},
                {"description": "Claude shape", "status": "completed"},
                {"title": "   ", "description": "title takes precedence"}
            ]
        }));

        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].title, "canonical");
        assert_eq!(tasks[0].status, forge_types::TodoStatus::Pending);
        assert_eq!(tasks[1].title, "Codex shape");
        assert_eq!(tasks[1].status, forge_types::TodoStatus::InProgress);
        assert_eq!(tasks[2].title, "Claude shape");
        assert_eq!(tasks[2].status, forge_types::TodoStatus::Done);
        assert!(parse_tasks(&serde_json::json!({"tasks": []})).is_empty());
        assert!(
            tasks.iter().all(|t| t.assignee.is_none()),
            "an omitted assignee stays absent — the pre-existing call shape is unchanged"
        );
    }

    #[test]
    fn parse_tasks_reads_an_optional_assignee_without_inventing_one() {
        let tasks = parse_tasks(&serde_json::json!({
            "tasks": [
                {"title": "port the store", "status": "in_progress", "assignee": " builder-a3 "},
                {"title": "review the diff", "status": "pending"},
                {"title": "ship it", "status": "pending", "assignee": "   "}
            ]
        }));
        assert_eq!(tasks[0].assignee.as_deref(), Some("builder-a3"), "trimmed");
        assert_eq!(tasks[1].assignee, None, "omitted stays unassigned");
        assert_eq!(tasks[2].assignee, None, "blank is not an owner");

        // The advertised schema must actually offer the field, or the model can never send it.
        let schema = update_tasks_spec().schema;
        let props = &schema["properties"]["tasks"]["items"]["properties"];
        assert!(
            props.get("assignee").is_some(),
            "schema advertises assignee"
        );
        let required = schema["properties"]["tasks"]["items"]["required"]
            .as_array()
            .expect("items.required is an array");
        assert_eq!(
            required.len(),
            1,
            "assignee must stay optional — title is the only required field"
        );
    }

    #[test]
    fn partial_task_update_keeps_an_assignee_it_does_not_mention() {
        // A status-only patch is not an unassignment: the owner recorded by the full list must
        // survive, and an explicit new owner must still win.
        let existing = parse_tasks(&serde_json::json!({
            "tasks": [
                {"title": "port the store", "status": "in_progress", "assignee": "builder-a3"},
                {"title": "review the diff", "status": "pending", "assignee": "reviewer"},
                {"title": "ship it", "status": "pending"}
            ]
        }));
        let merged = merge_task_update(
            &existing,
            parse_tasks(&serde_json::json!({
                "tasks": [
                    {"title": "port the store", "status": "done"},
                    {"title": "review the diff", "status": "in_progress", "assignee": "someone-else"}
                ]
            })),
        );
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].status, forge_types::TodoStatus::Done);
        assert_eq!(merged[0].assignee.as_deref(), Some("builder-a3"));
        assert_eq!(merged[1].assignee.as_deref(), Some("someone-else"));
        assert_eq!(merged[2].assignee, None);
    }

    #[test]
    fn partial_task_status_update_preserves_omitted_bridge_tasks() {
        let existing = parse_tasks(&serde_json::json!({
            "tasks": [
                {"description": "Create p1.txt", "status": "in_progress"},
                {"description": "Create p2.txt", "status": "pending"},
                {"description": "Create p3.txt", "status": "pending"},
                {"description": "Create p4.txt", "status": "pending"},
                {"description": "Create p5.txt", "status": "pending"}
            ]
        }));
        let partial = parse_tasks(&serde_json::json!({
            "tasks": [{"description": "Create p1.txt", "status": "done"}]
        }));
        let merged = merge_task_update(&existing, partial);
        assert_eq!(
            merged.len(),
            5,
            "a partial status call must not erase p2-p5"
        );
        assert_eq!(merged[0].status, forge_types::TodoStatus::Done);
        assert!(merged[1..]
            .iter()
            .all(|task| task.status == forge_types::TodoStatus::Pending));

        assert!(
            merge_task_update(&existing, Vec::new()).is_empty(),
            "an explicit empty list still clears the tracker"
        );
    }

    fn one_step_plan() -> forge_types::PlanProposal {
        forge_types::PlanProposal {
            title: "T".into(),
            steps: vec![forge_types::PlanStep {
                title: "a".into(),
                detail: String::new(),
            }],
            notes: None,
        }
    }

    #[test]
    fn plan_approval_build_falls_back_to_auto_edit_without_a_captured_temper() {
        let mut s = scripted_session("Build it", Arc::new(Mutex::new(0)));
        s.mode = PermissionMode::Plan;
        s.config.permission_mode = PermissionMode::Plan;
        let next = s.resolve_plan_approval(&one_step_plan());
        assert_eq!(next.as_deref(), Some(PLAN_BUILD_PROMPT));
        assert_eq!(
            s.mode,
            PermissionMode::AcceptEdits,
            "build flips to Auto-edit"
        );
    }

    #[test]
    fn plan_approval_build_restores_the_exact_previous_temper() {
        for previous in [PermissionMode::Default, PermissionMode::Bypass] {
            let mut s = scripted_session("Build it", Arc::new(Mutex::new(0)));
            s.set_temper(previous);
            s.set_temper(PermissionMode::Plan);

            let next = s.resolve_plan_approval(&one_step_plan());

            assert_eq!(next.as_deref(), Some(PLAN_BUILD_PROMPT));
            assert_eq!(s.mode, previous, "Build must restore {previous:?}");
        }
    }

    #[test]
    fn plan_approval_cancel_restores_the_previous_temper() {
        let mut s = scripted_session("Cancel", Arc::new(Mutex::new(0)));
        s.set_temper(PermissionMode::Bypass);
        s.set_temper(PermissionMode::Plan);
        assert!(s.resolve_plan_approval(&one_step_plan()).is_none());
        assert_eq!(s.mode, PermissionMode::Bypass, "cancel restores Full");
    }

    #[test]
    fn proposed_plan_does_not_activate_tasks_until_build_is_approved() {
        let mut s = scripted_session("Build it", Arc::new(Mutex::new(0)));
        let plan = one_step_plan();

        s.ingest_plan(plan.clone());

        assert!(s.tasks.is_empty(), "a proposal is not active work yet");
        assert!(
            s.store.tasks(s.id()).unwrap().is_empty(),
            "unapproved steps must not reach persistent tasks"
        );

        s.set_temper(PermissionMode::Plan);
        assert_eq!(
            s.resolve_plan_approval(&plan).as_deref(),
            Some(PLAN_BUILD_PROMPT)
        );
        assert_eq!(s.tasks.len(), 1, "Build activates the approved plan");
        assert_eq!(s.store.tasks(s.id()).unwrap().len(), 1);
    }

    #[test]
    fn presenting_a_plan_enters_plan_mode_and_captures_the_current_temper() {
        let mut s = scripted_session("Build it", Arc::new(Mutex::new(0)));
        s.set_temper(PermissionMode::Bypass);

        s.ingest_plan(one_step_plan());

        assert_eq!(s.mode, PermissionMode::Plan);
        assert_eq!(s.pre_plan_mode, Some(PermissionMode::Bypass));
        assert_eq!(
            s.resolve_plan_approval(&one_step_plan()).as_deref(),
            Some(PLAN_BUILD_PROMPT)
        );
        assert_eq!(s.mode, PermissionMode::Bypass);
    }

    #[test]
    fn plan_approval_free_text_revises_without_switching() {
        let mut s = scripted_session("make it shorter", Arc::new(Mutex::new(0)));
        s.set_temper(PermissionMode::Plan);
        let next = s
            .resolve_plan_approval(&one_step_plan())
            .expect("revision prompt");
        assert!(
            next.contains("make it shorter"),
            "carries the user's feedback"
        );
        assert!(
            next.contains("present_plan"),
            "asks the model to re-present"
        );
        assert_eq!(
            s.mode,
            PermissionMode::Plan,
            "revise does not switch to Auto-edit"
        );
    }

    /// Requests a `list_dir` tool call once, then answers `done` after the tool result.
    struct ListDirProvider;
    #[async_trait::async_trait]
    impl Provider for ListDirProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_provider::ModelResponse;
            use forge_types::{new_id, ToolCall, Usage};
            if messages.iter().any(|m| m.role == Role::Tool) {
                return Ok(ModelResponse {
                    content: "done".into(),
                    tool_calls: vec![],
                    usage: Usage::default(),
                    quotas: Vec::new(),
                });
            }
            Ok(ModelResponse {
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: new_id(),
                    name: "list_dir".into(),
                    args: serde_json::json!({ "path": "." }),
                }],
                usage: Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    /// Returns a fixed summary for compaction; never requests tools.
    struct SummarizingProvider;
    #[async_trait::async_trait]
    impl Provider for SummarizingProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            Ok(forge_provider::ModelResponse {
                content: "SUMMARY: built the parser, wired the CLI.".into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    /// Reports, as its final answer, whether the transcript it received carried a Lattice
    /// auto-injection system message — lets a test assert injection happened.
    struct InjectionProbeProvider;
    #[async_trait::async_trait]
    impl Provider for InjectionProbeProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            let saw = messages.iter().any(|m| {
                m.role == Role::System && m.content.starts_with("Relevant code (Lattice):")
            });
            Ok(forge_provider::ModelResponse {
                content: if saw { "SAW_INJECTION" } else { "NO_INJECTION" }.into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    fn probe_session(store: Arc<Store>, config: Config) -> Session {
        Session::start(
            store,
            Arc::new(InjectionProbeProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn lattice_injects_relevant_code_into_the_turn() {
        let dir = std::env::temp_dir().join(format!(
            "forge-inj-{}-{}",
            std::process::id(),
            forge_types::new_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("probe.rs"), "pub fn lattice_probe_symbol() {}\n").unwrap();

        let store = Arc::new(Store::open_in_memory().unwrap());
        let lat = forge_index::Lattice::new(Arc::clone(&store), &dir);
        lat.update().unwrap();

        let mut session = probe_session(Arc::clone(&store), Config::default());
        session.set_lattice(Some(Arc::new(lat)));
        // Pin a non-bridge model: injection is intentionally skipped for CLI bridges, and the
        // default mesh routes this prompt's tier to claude-cli::.
        session.pin_model(Some("ollama::probe".into()));
        let answer = session
            .run_turn("explain lattice_probe_symbol please")
            .await
            .unwrap();
        assert_eq!(
            answer, "SAW_INJECTION",
            "the symbol was retrieved + injected"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Bridged CLIs run their own exploration loop, so lattice injection is duplicated context
    /// there — the gate must skip it for `*-cli::` models while direct models keep it.
    #[tokio::test]
    async fn lattice_injection_is_skipped_for_cli_bridge_models() {
        let dir = std::env::temp_dir().join(format!(
            "forge-inj-bridge-{}-{}",
            std::process::id(),
            forge_types::new_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("probe.rs"), "pub fn lattice_probe_symbol() {}\n").unwrap();

        let store = Arc::new(Store::open_in_memory().unwrap());
        let lat = forge_index::Lattice::new(Arc::clone(&store), &dir);
        lat.update().unwrap();

        let mut session = probe_session(Arc::clone(&store), Config::default());
        session.set_lattice(Some(Arc::new(lat)));
        session.pin_model(Some("claude-cli::sonnet".into()));
        let answer = session
            .run_turn("explain lattice_probe_symbol please")
            .await
            .unwrap();
        assert_eq!(answer, "NO_INJECTION", "bridge models get no injection");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shell_command_failed_reads_the_exit_status() {
        assert!(!shell_command_failed("shell: exit 0 in 5ms\n\nhi"));
        assert!(shell_command_failed("shell: exit 1 in 5ms"));
        assert!(shell_command_failed("shell: exit 127 in 5ms"));
        assert!(shell_command_failed("shell: timed out after 1s (killed)"));
        assert!(shell_command_failed("shell: failed to start (cwd .): x"));
        assert!(shell_command_failed("shell: exit signal in 5ms"));
        // Not a shell result at all → not treated as a shell failure.
        assert!(!shell_command_failed("read 3 files"));
    }

    #[test]
    fn pattern_diagnose_matches_common_failures() {
        assert!(pattern_diagnose("bash: docker: command not found").is_some());
        assert!(pattern_diagnose("ls: /tmp/missing: No such file or directory").is_some());
        assert!(pattern_diagnose("chmod: cannot access 'x.sh': Permission denied").is_some());
        assert!(pattern_diagnose("bind: address already in use").is_some());
        assert!(pattern_diagnose("curl: (7) Failed to connect: Connection refused").is_some());
        assert!(pattern_diagnose("cp: error writing 'x': No space left on device").is_some());
        assert!(pattern_diagnose("Cannot allocate memory").is_some());
    }

    #[test]
    fn pattern_diagnose_returns_none_for_unrecognised_errors() {
        assert!(
            pattern_diagnose("shell: exit 1 in 2ms\n\ntest failed: assertion `left == right`")
                .is_none()
        );
        assert!(
            pattern_diagnose("shell: exit 2 in 1ms\n\nmake: *** [Makefile:5: build] Error 2")
                .is_none()
        );
    }

    #[test]
    fn pattern_diagnose_is_case_insensitive() {
        assert!(pattern_diagnose("COMMAND NOT FOUND").is_some());
        assert!(pattern_diagnose("PERMISSION DENIED").is_some());
    }

    /// First call emits a failing `shell` command; the diagnosis call (identified by its system
    /// prompt) returns a fix; after the tool result it answers `done`. Unix-only: the `shell`
    /// tool shells out to `sh`, so the e2e tests using it are gated to Unix.
    #[cfg(unix)]
    struct ShellFailProvider;
    #[cfg(unix)]
    #[async_trait::async_trait]
    impl Provider for ShellFailProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_provider::ModelResponse;
            use forge_types::{new_id, ToolCall, Usage};
            let usage = Usage::default();
            if messages
                .iter()
                .any(|m| m.role == Role::System && m.content.starts_with("A shell command run by"))
            {
                return Ok(ModelResponse {
                    content: "The command is not installed. Fix: install it first.".into(),
                    tool_calls: vec![],
                    usage,
                    quotas: Vec::new(),
                });
            }
            if messages.iter().any(|m| m.role == Role::Tool) {
                return Ok(ModelResponse {
                    content: "done".into(),
                    tool_calls: vec![],
                    usage,
                    quotas: Vec::new(),
                });
            }
            Ok(ModelResponse {
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: new_id(),
                    name: "shell".into(),
                    args: serde_json::json!({
                        "command": "printf opaque_failure_xyz >&2; exit 9"
                    }),
                }],
                usage,
                quotas: Vec::new(),
            })
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_shell_command_is_auto_diagnosed() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        // Bypass auto-allows the shell call so the interceptor path is reached.
        let config = Config {
            permission_mode: forge_types::PermissionMode::Bypass,
            ..Config::default()
        };
        let presenter = CapturePresenter::default();
        let events = presenter.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(ShellFailProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(presenter),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        session.run_turn("build the project").await.unwrap();

        let events = events.lock().unwrap();
        let started = events.iter().position(|event| {
            matches!(event, PresenterEvent::AuxiliaryRequest { purpose, .. }
                if purpose.contains("diagnosing"))
        });
        let diagnosed = events.iter().position(|e| {
            matches!(e, PresenterEvent::ShellDiagnosis { command, diagnosis, .. }
                if command.contains("opaque_failure_xyz") && diagnosis.contains("install"))
        });
        assert!(
            started.is_some(),
            "the UI is told why an internal model call started"
        );
        assert!(
            diagnosed.is_some(),
            "a ShellDiagnosis event was emitted for the failed command"
        );
        assert!(
            started < diagnosed,
            "the start event precedes the completed diagnosis"
        );
    }

    /// The main loop responds normally, but the optional shell-diagnosis request never emits an
    /// event and never returns. This reproduces a real TUI run that remained stuck in
    /// "auxiliary model work" for more than six minutes after a failing test command.
    #[cfg(unix)]
    struct StallingShellDiagnosisProvider;
    #[cfg(unix)]
    #[async_trait::async_trait]
    impl Provider for StallingShellDiagnosisProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_provider::ModelResponse;
            use forge_types::{new_id, ToolCall, Usage};
            if messages
                .iter()
                .any(|m| m.role == Role::System && m.content.starts_with("A shell command run by"))
            {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                unreachable!("the auxiliary watchdog must abort this request")
            }
            if messages.iter().any(|m| m.role == Role::Tool) {
                return Ok(ModelResponse {
                    content: "done".into(),
                    tool_calls: vec![],
                    usage: Usage::default(),
                    quotas: Vec::new(),
                });
            }
            Ok(ModelResponse {
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: new_id(),
                    name: "shell".into(),
                    args: serde_json::json!({
                        "command": "printf opaque_auxiliary_stall >&2; exit 23"
                    }),
                }],
                usage: Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stalled_shell_diagnosis_times_out_and_main_turn_continues() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut config = Config {
            permission_mode: forge_types::PermissionMode::Bypass,
            ..Config::default()
        };
        config.mesh.stream_idle_timeout_secs = 1;
        let presenter = CapturePresenter::default();
        let events = presenter.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(StallingShellDiagnosisProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(presenter),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            session.run_turn("run the failing test and repair it"),
        )
        .await;
        assert!(
            result.is_ok(),
            "the optional diagnosis held the turn hostage"
        );
        assert_eq!(result.unwrap().unwrap(), "done");

        let events = events.lock().unwrap();
        assert!(events.iter().any(|event| {
            matches!(event, PresenterEvent::AuxiliaryRequest { purpose, .. }
                if purpose.contains("diagnosing"))
        }));
        assert!(events.iter().any(|event| {
            matches!(event, PresenterEvent::Warning(message)
                if message.contains("optional shell diagnosis")
                    && message.contains("continuing without it"))
        }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn successful_shell_command_is_not_diagnosed() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let config = Config {
            permission_mode: forge_types::PermissionMode::Bypass,
            ..Config::default()
        };
        let presenter = CapturePresenter::default();
        let events = presenter.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(EchoShellProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(presenter),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        session.run_turn("say hi").await.unwrap();

        let diagnosed = events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, PresenterEvent::ShellDiagnosis { .. }));
        assert!(
            !diagnosed,
            "a succeeding command must not trigger the interceptor"
        );
    }

    /// Emits a succeeding `shell` command once, then answers `done`. Unix-only (see above).
    #[cfg(unix)]
    struct EchoShellProvider;
    #[cfg(unix)]
    #[async_trait::async_trait]
    impl Provider for EchoShellProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_provider::ModelResponse;
            use forge_types::{new_id, ToolCall, Usage};
            if messages.iter().any(|m| m.role == Role::Tool) {
                return Ok(ModelResponse {
                    content: "done".into(),
                    tool_calls: vec![],
                    usage: Usage::default(),
                    quotas: Vec::new(),
                });
            }
            Ok(ModelResponse {
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: new_id(),
                    name: "shell".into(),
                    args: serde_json::json!({ "command": "echo hi" }),
                }],
                usage: Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    /// Calls `use_skill("demoskill")` once, then reports whether the tool result carried the
    /// skill's methodology marker — lets a test assert the skill was found + loaded.
    struct UseSkillProvider;
    #[async_trait::async_trait]
    impl Provider for UseSkillProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_provider::ModelResponse;
            use forge_types::{new_id, ToolCall, Usage};
            if let Some(t) = messages.iter().rev().find(|m| m.role == Role::Tool) {
                let saw = t.content.contains("DEMO_SKILL_MARKER");
                return Ok(ModelResponse {
                    content: if saw { "SAW_SKILL" } else { "NO_SKILL" }.into(),
                    tool_calls: vec![],
                    usage: Usage::default(),
                    quotas: Vec::new(),
                });
            }
            Ok(ModelResponse {
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: new_id(),
                    name: USE_SKILL_TOOL.into(),
                    args: serde_json::json!({ "name": "demoskill" }),
                }],
                usage: Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn use_skill_tool_loads_a_real_skills_methodology() {
        let dir = std::env::temp_dir().join(format!("forge-useskill-{}", forge_types::new_id()));
        std::fs::create_dir_all(dir.join("skills/demoskill")).unwrap();
        std::fs::write(
            dir.join("skills/demoskill/SKILL.md"),
            "---\nname: demoskill\ndescription: a demo skill\n---\nDEMO_SKILL_MARKER: do the steps.",
        )
        .unwrap();
        let catalog = forge_skills::Catalog::load(&forge_skills::Sources {
            commands: vec![],
            skills: vec![forge_skills::ScopedDir {
                scope: forge_skills::Scope::User,
                path: dir.join("skills"),
            }],
        });

        let store = Arc::new(Store::open_in_memory().unwrap());
        let config = Config::default();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(UseSkillProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        session.set_skills(Some(Arc::new(catalog)));

        // The tool is advertised to the model...
        assert!(
            session
                .tool_specs()
                .iter()
                .any(|s| s.name == USE_SKILL_TOOL),
            "use_skill is advertised when a non-empty catalog is attached"
        );
        // ...and invoking it returns the skill's methodology as the tool result.
        let answer = session.run_turn("use the demo skill").await.unwrap();
        assert_eq!(
            answer, "SAW_SKILL",
            "use_skill returned the methodology to the model"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Calls `write_file` once (to `path`), then answers `done`.
    struct WriteFileProvider {
        path: String,
    }
    #[async_trait::async_trait]
    impl Provider for WriteFileProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_provider::ModelResponse;
            use forge_types::{new_id, ToolCall, Usage};
            if messages.iter().any(|m| m.role == Role::Tool) {
                return Ok(ModelResponse {
                    content: "done".into(),
                    tool_calls: vec![],
                    usage: Usage::default(),
                    quotas: Vec::new(),
                });
            }
            Ok(ModelResponse {
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: new_id(),
                    name: "write_file".into(),
                    args: serde_json::json!({ "path": self.path, "content": "hi from auto-edit" }),
                }],
                usage: Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn auto_edit_allows_file_writes_without_prompting() {
        // AcceptEdits must auto-allow a `write_file` (Write side effect) end to end through the
        // live session. CapturePresenter::confirm returns false, so if the turn wrongly PROMPTS
        // the write is denied and the file never appears — making a regression observable.
        let path = std::env::temp_dir()
            .join(format!("forge-autoedit-{}.txt", forge_types::new_id()))
            .to_string_lossy()
            .to_string();
        let workspace = std::path::Path::new(&path).parent().unwrap().to_path_buf();
        let config = Config {
            permission_mode: forge_types::PermissionMode::AcceptEdits,
            ..Config::default()
        };
        let mut session = Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(WriteFileProvider { path: path.clone() }),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(&workspace),
            Box::new(CapturePresenter::default()),
            config,
            workspace.to_str().unwrap(),
        )
        .unwrap();

        session.run_turn("write the file").await.unwrap();
        assert!(
            std::path::Path::new(&path).exists(),
            "auto-edit allowed the write without prompting"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn outside_workspace_write_is_a_recorded_tool_error_not_a_stalled_turn() {
        let root =
            std::env::temp_dir().join(format!("forge-workspace-reject-{}", forge_types::new_id()));
        let workspace = root.join("workspace");
        let outside = root.join("outside.txt");
        std::fs::create_dir_all(&workspace).unwrap();

        let config = Config {
            permission_mode: forge_types::PermissionMode::Bypass,
            ..Config::default()
        };
        let mut session = Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(WriteFileProvider {
                path: outside.to_string_lossy().to_string(),
            }),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(&workspace),
            Box::new(CapturePresenter::default()),
            config,
            workspace.to_str().unwrap(),
        )
        .unwrap();

        let answer = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            session.run_turn("try an invalid write"),
        )
        .await
        .expect("workspace rejection must not stall the turn")
        .expect("workspace rejection must be returned as a tool result");
        assert_eq!(answer, "done");
        assert!(!outside.exists(), "outside path must remain untouched");

        let envelope = session
            .transcript
            .iter()
            .find_map(|message| message.tool_calls.first())
            .expect("provider tool envelope is retained");
        let result = session
            .transcript
            .iter()
            .find(|message| {
                message.role == Role::Tool
                    && message.tool_call_id.as_deref() == Some(envelope.id.as_str())
            })
            .expect("rejected tool envelope receives a matching result");
        assert!(
            result.content.contains("escapes session workspace"),
            "model receives an actionable confinement error: {}",
            result.content
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Never streams an event and never returns — simulates a half-open / stalled connection.
    struct StallingProvider;
    #[async_trait::async_trait]
    impl Provider for StallingProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            unreachable!("the idle watchdog must abort this before it ever returns")
        }
    }

    /// Emits content-free provider heartbeats while a buffered tool/result payload is arriving,
    /// then completes. This models genai's OpenAI-compatible tool-call stream, where partial JSON
    /// must not be surfaced as a ToolStarted event before the final envelope is valid.
    struct HeartbeatThenFinishProvider;
    #[async_trait::async_trait]
    impl Provider for HeartbeatThenFinishProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            for _ in 0..4 {
                on_event(StreamEvent::ProviderActivity);
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            Ok(forge_provider::ModelResponse {
                content: "buffered stream completed".to_string(),
                ..Default::default()
            })
        }
    }

    /// Emits a real tool start, stays quiet longer than the base idle window while that tool runs,
    /// then reports the result and completes. The watchdog should distinguish this bounded tool
    /// execution from a half-open provider stream.
    struct SlowToolThenFinishProvider;
    #[async_trait::async_trait]
    impl Provider for SlowToolThenFinishProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            on_event(StreamEvent::ToolStarted {
                name: "shell".to_string(),
                args: r#"{"command":"build"}"#.to_string(),
            });
            tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
            on_event(StreamEvent::ToolFinished {
                name: "shell".to_string(),
                ok: true,
                summary: "built".to_string(),
            });
            Ok(forge_provider::ModelResponse {
                content: "tool completed".to_string(),
                ..Default::default()
            })
        }
    }

    #[tokio::test]
    async fn stalled_stream_times_out_instead_of_hanging() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut config = Config::default();
        config.mesh.stream_idle_timeout_secs = 1; // trip fast in the test
        config.mesh.failover = false; // no fallback → the error surfaces directly
        let mut session = Session::start(
            store,
            Arc::new(StallingProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        // The whole call must return well within this bound — if it hangs, the test fails here.
        let res = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            session.run_turn("anything"),
        )
        .await;
        assert!(
            res.is_ok(),
            "run_turn hung instead of timing out the stream"
        );
        assert!(
            res.unwrap().is_err(),
            "a stalled stream should surface an error, not a silent hang"
        );
    }

    #[tokio::test]
    async fn provider_activity_keeps_a_buffered_stream_alive() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut config = Config::default();
        config.mesh.stream_idle_timeout_secs = 1;
        config.mesh.failover = false;
        let mut session = Session::start(
            store,
            Arc::new(HeartbeatThenFinishProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            session.run_turn("wait for the buffered provider stream"),
        )
        .await
        .expect("heartbeat stream should finish within the test bound")
        .expect("provider heartbeats must prevent a false idle timeout");
        assert_eq!(result, "buffered stream completed");
    }

    #[tokio::test]
    async fn in_flight_tool_gets_one_bounded_extra_stream_idle_window() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut config = Config::default();
        config.mesh.stream_idle_timeout_secs = 1;
        config.mesh.failover = false;
        let mut session = Session::start(
            store,
            Arc::new(SlowToolThenFinishProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            session.run_turn("wait for the in-flight build tool"),
        )
        .await
        .expect("bounded slow tool should finish")
        .expect("the stream watchdog must not race a known in-flight tool");
        assert_eq!(result, "tool completed");
    }

    #[tokio::test]
    async fn turn_runs_unchanged_without_a_lattice() {
        // Additive guarantee: no index attached → no injection, turn proceeds as before.
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = probe_session(store, Config::default());
        let answer = session
            .run_turn("explain lattice_probe_symbol")
            .await
            .unwrap();
        assert_eq!(answer, "NO_INJECTION");
    }

    #[test]
    fn overflow_window_cap_only_shrinks_never_inflates() {
        // The context-overflow self-heal arms `overflow_window_cap` so the sent transcript trims
        // below a model's real window even when our token estimate diverges from its tokenizer.
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = probe_session(store, Config::default());
        let model = "nvidia::z-ai/glm-5.2";
        // No fetched window + not a CLI bridge → the conservative default.
        let base = forge_mesh::pricing::CONSERVATIVE_CONTEXT_WINDOW;
        assert_eq!(session.effective_context_window(model), base);
        // A cap below the window shrinks the usable window (the retry path).
        session.overflow_window_cap = Some((model.to_string(), base / 4));
        assert_eq!(session.effective_context_window(model), base / 4);
        // A cap above the window never inflates it.
        session.overflow_window_cap = Some((model.to_string(), base.saturating_mul(10)));
        assert_eq!(session.effective_context_window(model), base);
        // A cap armed for a DIFFERENT model is ignored (failover to a larger-window model).
        session.overflow_window_cap = Some(("some::other-model".to_string(), base / 8));
        assert_eq!(session.effective_context_window(model), base);
    }

    #[test]
    fn authoritative_context_window_beats_stale_cached_metadata() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let model = "qwencloud::qwen3.8-max-preview";
        store.set_model_context(model, 32_000).unwrap();
        let session = probe_session(store, Config::default());

        assert_eq!(session.base_context_window(model), 1_000_000);
    }

    #[tokio::test]
    async fn compact_folds_older_messages_into_a_summary() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(SummarizingProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        // 12 messages → compact keeps the last 6, folds the first 6 into one summary.
        for i in 0..12 {
            session
                .transcript
                .push(Message::user(format!("message {i}")));
        }
        let (before, after) = session.compact(false).await.unwrap();
        assert_eq!(before, 12);
        assert_eq!(
            after,
            COMPACT_KEEP_RECENT + 1,
            "summary + the kept recent messages"
        );
        assert!(session.transcript[0].content.contains("SUMMARY:"));
        assert!(session.transcript[0].content.contains("summarized"));
        // The most recent message is preserved verbatim at the tail.
        assert_eq!(session.transcript.last().unwrap().content, "message 11");
    }

    #[tokio::test]
    async fn ask_btw_writes_nothing_to_the_message_table() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = fresh_session(Arc::clone(&store), Config::default());
        let id = session.session_id().to_string();

        let before = store.load_all_messages(&id).unwrap().len();
        assert_eq!(before, 0, "fresh session starts with no persisted rows");

        session.ask_btw("what is 2+2?").await;

        let after = store.load_all_messages(&id).unwrap().len();
        assert_eq!(
            after, 0,
            "/btw must not write ANY row (active or soft-deleted) to the message table"
        );
        // The transcript used for the NEXT real turn's context must also be untouched.
        assert!(session.transcript.is_empty());
    }

    #[tokio::test]
    async fn ask_btw_emits_a_btw_answer_event_not_a_transcript_message() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        session.ask_btw("  what is forge?  ").await;

        let captured = events.lock().unwrap();
        let answer = captured.iter().find_map(|e| match e {
            PresenterEvent::BtwAnswer {
                question, answer, ..
            } => Some((question.clone(), answer.clone())),
            _ => None,
        });
        let (question, answer) = answer.expect("a BtwAnswer event was emitted");
        assert_eq!(question, "what is forge?", "the question is trimmed");
        assert!(!answer.is_empty());
        assert!(session.transcript.is_empty());
    }

    #[tokio::test]
    async fn ask_btw_on_blank_question_warns_and_makes_no_call() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        session.ask_btw("   ").await;

        let captured = events.lock().unwrap();
        assert!(matches!(
            captured.last(),
            Some(PresenterEvent::Warning(msg)) if msg.contains("usage: /btw")
        ));
        assert!(!captured
            .iter()
            .any(|e| matches!(e, PresenterEvent::BtwAnswer { .. })));
    }

    #[tokio::test]
    async fn compact_fails_over_when_the_summarizer_is_rate_limited() {
        // Regression: a rate-limited compaction summarizer must NOT kill the turn. It also runs
        // mid-failover (admit_failover_model), so a dead model here would otherwise abort an
        // otherwise-recoverable turn. It must walk the routed fallback chain instead.
        let provider = Arc::new(FlakyProvider {
            bad: ["bad::model".to_string()].into_iter().collect(),
            err: rate_limited,
        });
        let router = Arc::new(FixedRouter {
            model: "bad::model".into(),
            fallbacks: vec!["good::model".into()],
        });
        let (store, mut session) = fixed_session(provider, router);
        for i in 0..12 {
            session
                .transcript
                .push(Message::user(format!("message {i}")));
        }
        let (before, after) = session.compact(false).await.unwrap();
        assert_eq!(before, 12);
        assert_eq!(after, COMPACT_KEEP_RECENT + 1);
        // The fallback produced the summary, and the rate-limited primary was benched.
        assert!(session.transcript[0].content.contains("recovered"));
        let report = store.current_benched_report().unwrap();
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].0, "bad::model");
    }

    /// Records the payload the summarizer was actually handed, so a test can assert the request was
    /// fitted to the model's window BEFORE dispatch rather than after a round of overflow errors.
    #[derive(Default)]
    struct CapturePayloadProvider {
        payload: std::sync::Mutex<Option<String>>,
    }
    #[async_trait::async_trait]
    impl Provider for CapturePayloadProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            *self.payload.lock().unwrap() = messages
                .iter()
                .find(|m| m.role == Role::User)
                .map(|m| m.content.clone());
            Ok(forge_provider::ModelResponse {
                content: "SUMMARY: fitted".into(),
                ..Default::default()
            })
        }
    }

    #[tokio::test]
    async fn compaction_payload_is_fitted_to_the_summarizer_window_before_dispatch() {
        // The defect: `compact` sent the entire untrimmed older transcript, so on a long session
        // every trivial-tier summarizer returned a context-overflow error before the summary could
        // be produced at all.
        let provider = Arc::new(CapturePayloadProvider::default());
        let router = Arc::new(FixedRouter {
            model: "small::summarizer".into(),
            fallbacks: vec![],
        });
        let (store, mut session) = fixed_session(provider.clone(), router);
        // A small FETCHED window, so the budget is deterministic instead of riding the conservative
        // floor, and far smaller than the transcript below.
        store.set_model_context("small::summarizer", 8_000).unwrap();
        let filler = "alpha bravo charlie delta echo foxtrot ".repeat(200);
        for i in 0..46 {
            session
                .transcript
                .push(Message::user(format!("message {i} {filler}")));
        }

        let (before, after) = session.compact(false).await.unwrap();
        assert_eq!(before, 46);
        assert_eq!(after, COMPACT_KEEP_RECENT + 1, "the summary landed");

        let payload = provider
            .payload
            .lock()
            .unwrap()
            .clone()
            .expect("the summarizer was called exactly once");
        let budget = session.compact_input_budget("small::summarizer");
        assert!(
            tokens::count_message(&payload) <= budget,
            "the summarizer must never be handed more than its window holds: {} > {budget}",
            tokens::count_message(&payload)
        );
        assert!(
            payload.contains("message 0 ") && payload.contains("message 39 "),
            "both ends of the older stretch must survive the fit"
        );
        assert!(
            !payload.contains("message 20 "),
            "the middle is what was dropped"
        );
    }

    #[tokio::test]
    async fn a_context_overflow_during_compaction_records_no_health_penalty() {
        // The second half of the defect: an over-window compaction payload benched every healthy
        // trivial model it was walked into, degrading routing for the ordinary turns afterwards.
        // An oversized request is the caller's fault and must not touch the model's health record.
        let provider = Arc::new(FlakyProvider {
            bad: ["over::a".to_string(), "over::b".to_string()]
                .into_iter()
                .collect(),
            err: context_overflow,
        });
        let router = Arc::new(FixedRouter {
            model: "over::a".into(),
            fallbacks: vec!["over::b".into()],
        });
        let (store, mut session) = fixed_session(provider, router);
        for i in 0..12 {
            session
                .transcript
                .push(Message::user(format!("message {i}")));
        }

        let err = session
            .compact(false)
            .await
            .expect_err("every candidate overflowed, so the chain is exhausted");
        assert!(matches!(err, CoreError::Provider(_)));
        let benched = store.current_benched_report().unwrap();
        assert!(
            benched.is_empty(),
            "an oversized request is not a health signal about the model: {benched:?}"
        );
    }

    #[tokio::test]
    async fn full_history_survives_compaction_for_the_user_view() {
        // After compaction the model sees a summary, but the USER must still be able to view the
        // entire original conversation, and can opt to reload it into the model's context.
        let provider = Arc::new(SummarizingProvider);
        let router = Arc::new(HeuristicRouter::new(Config::default()));
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = Session::start(
            Arc::clone(&store),
            provider,
            router,
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        let sid = session.id().to_string();
        for i in 0..10 {
            store
                .add_message(&sid, i, Role::User, &format!("turn {i}"), None)
                .unwrap();
        }
        store
            .compact_session_store(&sid, "SUMMARY of turns 0..6", 3)
            .unwrap();

        session.reset_resumed(&sid).unwrap();
        // Model context is the compacted view…
        assert!(
            session.history().len() < 10,
            "model sees the compacted view"
        );
        // …but the user's full replay shows all 10 original turns.
        let full_users = session
            .replay_items_full()
            .into_iter()
            .filter(|i| matches!(i, forge_types::ReplayItem::User(_)))
            .count();
        assert_eq!(full_users, 10, "full replay shows every original user turn");
        assert!(session.was_compacted());

        // Reloading the full history puts all 10 turns back into the model context.
        session.reload_full_context().unwrap();
        let model_users = session
            .transcript
            .iter()
            .filter(|m| m.role == Role::User)
            .count();
        assert_eq!(
            model_users, 10,
            "reload_full_context restores the uncompacted context"
        );
    }

    #[tokio::test]
    async fn compact_undo_restores_the_live_transcript() {
        // Modeled on `full_history_survives_compaction_for_the_user_view`: seed real store rows
        // (not just in-memory transcript) so `reload_full_context` after undo has something to
        // rehydrate from.
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(SummarizingProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        let sid = session.id().to_string();
        for i in 0..10 {
            store
                .add_message(&sid, i, Role::User, &format!("turn {i}"), None)
                .unwrap();
        }
        store
            .compact_session_store(&sid, "SUMMARY of turns 0..6", 3)
            .unwrap();
        session.reset_resumed(&sid).unwrap();

        assert!(session.transcript.len() < 10, "transcript shrank");
        assert!(session.was_compacted());
        let compacted_len = session.transcript.len();

        let (undo_before, undo_after) = session.uncompact().unwrap();
        assert_eq!(
            undo_before, compacted_len,
            "uncompact starts from the compacted view"
        );
        assert_eq!(undo_after, 10, "full transcript restored");
        let model_users = session
            .transcript
            .iter()
            .filter(|m| m.role == Role::User)
            .count();
        assert_eq!(model_users, 10, "every original turn back in context");
        assert!(
            !session.was_compacted(),
            "the compaction row is gone after undo"
        );
    }

    #[tokio::test]
    async fn compact_undo_is_a_noop_without_a_prior_compaction() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(SummarizingProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        session.transcript.push(Message::user("just one"));
        let (before, after) = session.uncompact().unwrap();
        assert_eq!((before, after), (1, 1), "nothing to undo");
    }

    #[tokio::test]
    async fn compact_is_a_noop_for_a_short_transcript() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(SummarizingProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        session.transcript.push(Message::user("just one"));
        let (before, after) = session.compact(false).await.unwrap();
        assert_eq!((before, after), (1, 1), "nothing to compact");
    }

    #[tokio::test]
    async fn a_pretooluse_hook_blocks_the_tool_call() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        // Bypass so the only thing that can stop the (ReadOnly) tool is the hook itself.
        let config = Config {
            permission_mode: forge_types::PermissionMode::Bypass,
            hooks: vec![forge_config::HookConfig {
                event: forge_config::HookEvent::PreToolUse,
                matcher: Some("list_dir".into()),
                #[cfg(not(windows))]
                command: "echo blocked-by-test 1>&2; exit 1".into(),
                #[cfg(windows)]
                command: "echo blocked-by-test 1>&2 & exit /b 1".into(),
                timeout_secs: 10,
                cc_compat: false,
            }],
            ..Config::default()
        };
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(ListDirProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        session.run_turn("list the files").await.unwrap();

        let evs = events.lock().unwrap();
        let blocked = evs.iter().any(|e| {
            matches!(e, PresenterEvent::ToolResult { name, ok, summary }
                if name == "list_dir" && !ok && summary.contains("blocked by hook"))
        });
        assert!(
            blocked,
            "the list_dir call was blocked by the PreToolUse hook"
        );
    }

    #[tokio::test]
    async fn resume_restores_the_pinned_effort_level() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let id = store.create_session(".", "default").unwrap();

        let resumed = |store: Arc<Store>| {
            Session::resume(
                store,
                Arc::new(MockProvider),
                Arc::new(HeuristicRouter::new(Config::default())),
                ToolRegistry::with_core_tools_in(test_workspace()),
                Box::new(CapturePresenter::default()),
                Config::default(),
                &id,
            )
            .unwrap()
        };

        // A session that never pinned effort must resume unpinned, not at some default level:
        // the absence of a pin is what lets the provider default apply.
        assert_eq!(resumed(Arc::clone(&store)).pinned_effort(), None);

        let mut session = resumed(Arc::clone(&store));
        session.set_effort(Some(forge_types::EffortLevel::WhiteHot));
        assert_eq!(
            resumed(Arc::clone(&store)).pinned_effort(),
            Some(forge_types::EffortLevel::WhiteHot),
            "a raised effort must survive resume rather than silently dropping to the default"
        );

        // Lowering replaces rather than being ignored.
        let mut session = resumed(Arc::clone(&store));
        session.set_effort(Some(forge_types::EffortLevel::Low));
        assert_eq!(
            resumed(Arc::clone(&store)).pinned_effort(),
            Some(forge_types::EffortLevel::Low)
        );

        // Clearing returns the session to the provider default.
        let mut session = resumed(Arc::clone(&store));
        session.set_effort(None);
        assert_eq!(resumed(Arc::clone(&store)).pinned_effort(), None);
    }

    #[tokio::test]
    async fn resume_restores_the_task_list() {
        use forge_types::{TodoItem, TodoStatus};
        let store = Arc::new(Store::open_in_memory().unwrap());
        let id = store.create_session(".", "default").unwrap();
        store
            .set_tasks(
                &id,
                &[TodoItem {
                    title: "earlier work".into(),
                    status: TodoStatus::InProgress,
                    assignee: None,
                }],
            )
            .unwrap();

        let session = Session::resume(
            Arc::clone(&store),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(CapturePresenter::default()),
            Config::default(),
            &id,
        )
        .unwrap();
        assert_eq!(session.tasks().len(), 1, "task list restored on resume");
        assert_eq!(session.tasks()[0].title, "earlier work");
    }

    #[tokio::test]
    async fn full_turn_routes_calls_tool_and_persists() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let config = Config::default();
        let mut session = Session::start(
            store,
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            // non-interactive: side-effect tools would be denied, but the mock uses read_file
            Box::new(HeadlessPresenter::new(false)),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        let answer = session
            .run_turn("check the project manifest")
            .await
            .unwrap();
        assert!(answer.contains("healthy"));

        // user + assistant + tool(read) + assistant(final) = 4 messages persisted.
        let count = session_message_count(&session);
        assert!(count >= 4, "expected >=4 messages, got {count}");
    }

    fn session_message_count(s: &Session) -> i64 {
        s.store.message_count(s.id()).unwrap()
    }

    #[tokio::test]
    async fn cost_accumulates_for_a_priced_model() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let config = priced_complex_config();
        let mut session = Session::start(
            store,
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        // "refactor ... concurrency" routes to the complex tier (a priced model),
        // so the mock's token counts must turn into a non-zero session cost.
        session
            .run_turn("refactor the architecture for concurrency")
            .await
            .unwrap();
        let cost = session.store.session_cost(session.id()).unwrap();
        assert!(cost > 0.0, "expected a non-zero cost, got {cost}");
    }

    #[derive(Default)]
    struct TerminalUsageProvider {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Provider for TerminalUsageProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (content, input_tokens, cached_input_tokens, output_tokens) = if call == 0 {
                ("main answer", 72, 20, 30)
            } else {
                ("", 30, 10, 12)
            };
            on_event(StreamEvent::Text(content.to_string()));
            Ok(forge_provider::ModelResponse {
                content: content.to_string(),
                tool_calls: Vec::new(),
                usage: forge_types::Usage {
                    input_tokens,
                    output_tokens,
                    cached_input_tokens,
                    cost_usd: 0.0,
                },
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn headless_terminal_cost_includes_awaited_memory_and_done_is_last() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut config = Config::default();
        config.recap.enabled = false;
        config.suggest.enabled = false;
        config.mesh.auto_memory = true;
        let capture = CapturePresenter::default();
        let captured_events = Arc::clone(&capture.events);
        let provider = Arc::new(TerminalUsageProvider::default());
        let mut session = Session::start(
            Arc::clone(&store),
            provider.clone(),
            Arc::new(FixedRouter {
                model: "mock::terminal-usage".to_string(),
                fallbacks: Vec::new(),
            }),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        let outcome = session
            .run_turn("check the project manifest")
            .await
            .unwrap();
        assert!(outcome.contains("main answer"));
        assert_eq!(
            provider.calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "one main completion plus one awaited memory completion"
        );

        // The provider spends 72/30 tokens on the main turn and 30/12 on the awaited memory
        // extraction side call. The terminal event must reflect the complete Store ledger.
        assert_eq!(
            store.session_tokens(session.id()).unwrap(),
            (72, 30),
            "active-transcript usage intentionally excludes synthetic side calls"
        );
        let consumed = store.session_token_usage(session.id()).unwrap();
        assert_eq!((consumed.input_tokens, consumed.output_tokens), (102, 42));
        assert_eq!(consumed.cached_input_tokens, 30);
        assert_eq!(session.session_usage_db(), (102, 42, 0.0));
        let events = captured_events.lock().unwrap();
        let done_index = events
            .iter()
            .rposition(|event| matches!(event, PresenterEvent::Done { .. }))
            .expect("turn emits Done");
        let (cost_index, event_tokens) = events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| match event {
                PresenterEvent::Cost {
                    session_in,
                    session_cached_in,
                    session_out,
                    ..
                } => Some((index, (*session_in, *session_cached_in, *session_out))),
                _ => None,
            })
            .next_back()
            .expect("turn emits Cost");
        assert_eq!(event_tokens, (102, 30, 42));
        assert!(cost_index < done_index, "terminal Cost must precede Done");
        assert_eq!(
            done_index,
            events.len() - 1,
            "Done must remain the final headless event"
        );
    }

    #[tokio::test]
    async fn warns_when_budget_threshold_reached() {
        // Complex turn costs (30+12)/1k + (42+18)/1k = 0.102 USD (keyless priced model, so
        // provider-fallback can't re-route and change the cost).
        let mut config = priced_complex_config();
        config.mesh.daily_budget_usd = Some(0.12); // 80% = 0.096

        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        // Turn 1 spends ~0.102 -> into the warning band (>= 0.096, < 0.12).
        session
            .run_turn("refactor the architecture for concurrency")
            .await
            .unwrap();
        // Turn 2 starts already in the warning band, so it must warn.
        session
            .run_turn("refactor the concurrency design again")
            .await
            .unwrap();

        let warned = events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, PresenterEvent::Warning(_)));
        assert!(warned, "expected a budget Warning event");
    }

    /// A config whose complex tier points at a keyless (always-available) model with a fixed
    /// 1.0/1k price, so budget/cost tests are deterministic regardless of which API keys the
    /// host happens to have — otherwise provider-fallback would re-route to an available model
    /// and change the cost out from under the test.
    fn priced_complex_config() -> Config {
        let mut config = Config::default();
        config.mesh.models.insert(
            "complex".to_string(),
            forge_config::OneOrMany::One("ollama::opus-sim".to_string()),
        );
        config.mesh.pricing.insert(
            "ollama::opus-sim".to_string(),
            forge_config::PriceOverride {
                input_per_1k: 1.0,
                output_per_1k: 1.0,
            },
        );
        config
    }

    fn test_workspace() -> &'static std::path::Path {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    fn fresh_session(store: Arc<Store>, config: Config) -> Session {
        let workspace = test_workspace();
        Session::start(
            store,
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(workspace),
            Box::new(HeadlessPresenter::new(false)),
            config,
            workspace.to_str().expect("workspace path is UTF-8"),
        )
        .unwrap()
    }

    #[test]
    fn fresh_session_uses_a_durable_explicit_workspace() {
        let session = fresh_session(
            Arc::new(Store::open_in_memory().unwrap()),
            Config::default(),
        );
        assert_eq!(
            session.workspace_root(),
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .canonicalize()
                .as_deref()
                .expect("manifest directory exists")
        );
    }

    #[cfg(unix)]
    #[test]
    fn fresh_session_ignores_a_deleted_ambient_cwd() {
        let base =
            std::env::temp_dir().join(format!("forge-deleted-cwd-{}", forge_types::new_id()));
        let deleted_cwd = base.join("deleted-cwd");
        std::fs::create_dir_all(&deleted_cwd).expect("creating temporary cwd");

        {
            let _cwd_guard = test_cwd_guard(&deleted_cwd);
            std::fs::remove_dir(&deleted_cwd).expect("removing ambient cwd");

            let session = fresh_session(
                Arc::new(Store::open_in_memory().unwrap()),
                Config::default(),
            );
            assert_eq!(
                session.workspace_root(),
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .canonicalize()
                    .as_deref()
                    .expect("manifest directory exists")
            );
        }

        std::fs::remove_dir_all(base).expect("removing temporary workspace parent");
    }

    /// Part C (mobile "stuck busy, no error" bug): a turn-ending failure must surface as an
    /// `Error` event, not a mere `Warning` — the headless `forge serve` driver only latches
    /// `Error` for its push-notification trigger and remote toast note (`Snapshot::notes`), so a
    /// `Warning` here was silently invisible to the mobile app even though `busy` itself always
    /// cleared correctly.
    #[test]
    fn notify_error_emits_an_error_event_not_just_a_warning() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let config = Config::default();
        let mut s = Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(CapturePresenter {
                events: events.clone(),
            }),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        s.notify_error("turn failed: no endpoints found that support image input");
        let captured = events.lock().unwrap();
        assert!(
            captured.iter().any(|e| matches!(
                e,
                PresenterEvent::Error(m) if m.contains("no endpoints found")
            )),
            "notify_error must emit a PresenterEvent::Error carrying the real failure: {captured:?}"
        );
        assert!(
            !captured
                .iter()
                .any(|e| matches!(e, PresenterEvent::Warning(_))),
            "notify_error must not ALSO downgrade to a Warning: {captured:?}"
        );
        assert!(
            captured
                .iter()
                .any(|e| matches!(e, PresenterEvent::Done { .. })),
            "notify_error must still end the turn with a Done marker so busy clears: {captured:?}"
        );
    }

    #[tokio::test]
    async fn recap_is_skipped_when_the_turn_produced_no_final_text() {
        // A stalled turn (empty-response give-up / failover exhaustion) leaves final_text empty.
        // MockProvider always returns non-empty content, so without the guard a recap WOULD be
        // emitted from the request alone — inventing success for a turn that did nothing. The
        // guard must suppress it entirely.
        let events = Arc::new(Mutex::new(Vec::new()));
        let config = Config::default();
        assert!(
            config.recap.enabled,
            "recap on by default — guard, not disable"
        );
        let mut s = Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(CapturePresenter {
                events: events.clone(),
            }),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        s.generate_recap("Fix buggy.py so average([]) returns 0.0", "", &[])
            .await;
        s.generate_recap("Fix buggy.py", "   \n\t ", &[]).await;
        let recaps = events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, PresenterEvent::Recap { .. }))
            .count();
        assert_eq!(recaps, 0, "empty/whitespace turn must not be recapped");
    }

    #[test]
    fn completed_task_recap_is_grounded_in_this_turns_state_change() {
        let before = vec![forge_types::TodoItem {
            title: "Write alpha.txt containing ALPHA".to_string(),
            status: forge_types::TodoStatus::InProgress,
            assignee: None,
        }];
        let after = vec![forge_types::TodoItem {
            title: "Write alpha.txt containing ALPHA".to_string(),
            status: forge_types::TodoStatus::Done,
            assignee: None,
        }];
        assert_eq!(
            completed_tasks_recap(
                &before,
                &after,
                "Verified alpha.txt contains ALPHA. The task is confirmed complete."
            )
            .as_deref(),
            Some("Completed the tracked task")
        );

        let three_done = (1..=3)
            .map(|i| forge_types::TodoItem {
                title: format!("Task {i}"),
                status: forge_types::TodoStatus::Done,
                assignee: None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            completed_tasks_recap(
                &[],
                &three_done,
                "All 3 tasks confirmed complete after verification."
            )
            .as_deref(),
            Some("Completed all 3 tracked tasks")
        );
    }

    #[test]
    fn completed_task_recap_does_not_reuse_stale_or_unconfirmed_tasks() {
        let done = vec![forge_types::TodoItem {
            title: "Old task".to_string(),
            status: forge_types::TodoStatus::Done,
            assignee: None,
        }];
        assert_eq!(
            completed_tasks_recap(&done, &done, "Answered the unrelated follow-up"),
            None,
            "an unchanged task list belongs to an earlier turn"
        );
        assert_eq!(
            completed_tasks_recap(&[], &done, "I could not complete the task"),
            None,
            "a negative final answer must not be rewritten as success"
        );
    }

    #[test]
    fn no_usable_model_message_names_the_dead_provider_and_the_fixes() {
        let msg = no_usable_model_message("groq::llama-3.1-8b-instant");
        assert!(msg.contains("groq"), "names the dead provider");
        assert!(msg.contains("forge auth"), "points at adding a key");
        assert!(
            msg.contains("forge models"),
            "points at the usable-models view"
        );
        assert!(msg.contains("/model"), "offers a pin escape hatch");
        // Mentions auto-discovery so a user who DOES have another key knows why it fell back.
        assert!(msg.to_lowercase().contains("auto-discovery"));
    }

    #[test]
    fn summarize_does_not_panic_on_multibyte_boundary() {
        // Byte 80 lands inside the multi-byte 'é' — `&first[..80]` would panic here.
        let line = format!(
            "{}éééééé, and a tail to push well past eighty bytes",
            "a".repeat(78)
        );
        let s = summarize(&line);
        assert!(s.ends_with('…'), "long line is truncated with an ellipsis");
        assert!(s.chars().count() <= 81);
    }

    #[test]
    fn summarize_passes_short_lines_through() {
        assert_eq!(summarize("ok: [workspace]"), "ok: [workspace]");
        assert_eq!(summarize("line one\nline two"), "line one");
    }

    #[tokio::test]
    async fn hard_stop_refuses_once_over_cap() {
        // AC-7: once the day total exceeds the cap, the next turn is refused before any
        // provider call and records no further spend.
        let mut config = priced_complex_config();
        config.mesh.daily_budget_usd = Some(0.05);
        let mut session = fresh_session(Arc::new(Store::open_in_memory().unwrap()), config);

        // Turn 1 sees $0 spent -> proceeds, spends ~$0.102 (over the $0.05 cap).
        session
            .run_turn("refactor the architecture for concurrency")
            .await
            .unwrap();
        let cost_after_1 = session.store.session_cost(session.id()).unwrap();
        assert!(
            cost_after_1 > 0.05,
            "turn 1 should exceed the cap: {cost_after_1}"
        );

        // Turn 2 is over budget -> hard stop.
        let answer = session
            .run_turn("refactor the concurrency design again")
            .await
            .unwrap();
        assert!(
            answer.contains("budget cap reached"),
            "turn 2 refused: {answer}"
        );
        let cost_after_2 = session.store.session_cost(session.id()).unwrap();
        assert!(
            (cost_after_2 - cost_after_1).abs() < 1e-9,
            "no spend after a hard stop"
        );
    }

    #[tokio::test]
    async fn daily_spend_aggregates_across_sessions() {
        // AC-1/AC-2: a second session sees the first session's spend in the day total.
        let path = std::env::temp_dir().join(format!("forge-budget-{}.db", forge_types::new_id()));
        let config = priced_complex_config(); // no cap -> both proceed; complex tier is priced

        let day_total_after_a = {
            let mut a = fresh_session(Arc::new(Store::open(&path).unwrap()), config.clone());
            a.run_turn("refactor the architecture for concurrency")
                .await
                .unwrap();
            a.store.spend_today_usd().unwrap()
        };
        assert!(day_total_after_a > 0.0, "session A recorded spend today");

        // A brand-new session on the same DB must see A's spend (the bug was a per-session reset).
        let b = fresh_session(Arc::new(Store::open(&path).unwrap()), config.clone());
        let seen_by_b = b.store.spend_today_usd().unwrap();
        assert!(
            (seen_by_b - day_total_after_a).abs() < 1e-9,
            "B sees the cross-session day total: {seen_by_b} vs {day_total_after_a}"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn resume_rehydrates_transcript_and_continues_same_session() {
        let path = std::env::temp_dir().join(format!("forge-resume-{}.db", forge_types::new_id()));
        // This test asserts message_count == transcript length; the per-turn recap side-call would
        // add a usage row (counted by message_count, not rehydrated), so disable it here.
        let mut config = Config::default();
        config.recap.enabled = false;
        config.suggest.enabled = false;

        // First run on a file-backed store, then drop it.
        let (id, cost1, msgs1) = {
            let mut s = fresh_session(Arc::new(Store::open(&path).unwrap()), config.clone());
            s.run_turn("refactor the architecture for concurrency")
                .await
                .unwrap();
            let id = s.id().to_string();
            (
                id.clone(),
                s.store.session_cost(&id).unwrap(),
                s.store.message_count(&id).unwrap(),
            )
        };

        // Resume on a fresh connection to the same file.
        let mut s2 = Session::resume(
            Arc::new(Store::open(&path).unwrap()),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            config,
            &id,
        )
        .unwrap();

        assert_eq!(s2.id(), id, "must continue the same session row");
        assert_eq!(
            s2.transcript.len() as i64,
            msgs1,
            "transcript should be rehydrated"
        );
        let cost_after_resume = s2.store.session_cost(&id).unwrap();
        assert!(
            (cost_after_resume - cost1).abs() < 1e-9,
            "prior cost preserved"
        );

        // Continuing appends to the same session.
        s2.run_turn("another complex refactor of the design")
            .await
            .unwrap();
        assert!(
            s2.store.message_count(&id).unwrap() > msgs1,
            "new turn appended"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn resume_missing_session_errors() {
        let err = Session::resume(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            Config::default(),
            "ghost-id",
        )
        .err()
        .unwrap();
        assert!(matches!(err, CoreError::SessionNotFound(_)));
    }

    // --- Subagent orchestration (RFC subagent-orchestration) ---

    /// A test provider that, for the TOP-LEVEL agent, calls `spawn_agents` with two inline
    /// subtasks then synthesizes; for a SUBAGENT (its transcript opens with the subagent system
    /// prompt) it behaves like the normal mock (read_file → done). Shared via `Arc` by parent
    /// and children, exactly as in production.
    #[derive(Default)]
    struct SpawnThenSynthProvider;

    #[async_trait::async_trait]
    impl Provider for SpawnThenSynthProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_provider::ModelResponse;
            use forge_types::{new_id, ToolCall, Usage};
            let is_subagent = messages
                .iter()
                .any(|m| m.role == Role::System && m.content.contains("subagent"));
            let used_tool = messages.iter().any(|m| m.role == Role::Tool);
            let usage = Usage {
                input_tokens: 30,
                output_tokens: 12,
                cached_input_tokens: 0,
                cost_usd: 0.0,
            };
            if is_subagent {
                // Child: read a file once, then answer.
                if used_tool {
                    let content = "child finding: ok";
                    on_event(StreamEvent::Text(content.into()));
                    return Ok(ModelResponse {
                        content: content.into(),
                        tool_calls: vec![],
                        usage,
                        quotas: Vec::new(),
                    });
                }
                return Ok(ModelResponse {
                    content: "reading".into(),
                    tool_calls: vec![ToolCall {
                        id: new_id(),
                        name: "read_file".into(),
                        args: serde_json::json!({"path": "Cargo.toml"}),
                    }],
                    usage,
                    quotas: Vec::new(),
                });
            }
            // Parent: fan out, then synthesize once results return.
            if used_tool {
                let content = "synthesized from subagents";
                on_event(StreamEvent::Text(content.into()));
                return Ok(ModelResponse {
                    content: content.into(),
                    tool_calls: vec![],
                    usage,
                    quotas: Vec::new(),
                });
            }
            Ok(ModelResponse {
                content: "delegating".into(),
                tool_calls: vec![ToolCall {
                    id: new_id(),
                    name: "spawn_agents".into(),
                    args: serde_json::json!({"agents": [
                        {"agent": "reviewer", "task": "review the change"},
                        {"task": "fix the typo in the readme"}
                    ]}),
                }],
                usage,
                quotas: Vec::new(),
            })
        }
    }

    /// A config with three distinct, keyless, priced tiers so routing is deterministic and a
    /// Trivial child routes to a cheaper model than a Complex parent.
    fn tiered_config() -> Config {
        use forge_config::{OneOrMany, PriceOverride};
        let mut config = Config::default();
        for (tier, model, price) in [
            ("trivial", "ollama::small", 0.001),
            ("standard", "ollama::mid", 0.05),
            ("complex", "ollama::big", 1.0),
        ] {
            config
                .mesh
                .models
                .insert(tier.into(), OneOrMany::One(model.into()));
            config.mesh.pricing.insert(
                model.into(),
                PriceOverride {
                    input_per_1k: price,
                    output_per_1k: price,
                },
            );
        }
        config
    }

    #[tokio::test]
    async fn spawn_agents_creates_linked_children_and_returns_results() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let config = tiered_config();
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(SpawnThenSynthProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        let parent_id = session.id().to_string();

        let answer = session
            .run_turn("design and architect a complex concurrency refactor across modules")
            .await
            .unwrap();

        assert!(
            answer.contains("synthesized"),
            "parent synthesizes: {answer}"
        );

        // Two child sessions, both linked to the parent.
        let children = store.child_sessions(&parent_id).unwrap();
        assert_eq!(children.len(), 2, "two children persisted with parent link");

        // Coarse lifecycle events surfaced for each child.
        let ev = events.lock().unwrap();
        let starts = ev
            .iter()
            .filter(|e| matches!(e, PresenterEvent::SubagentStart { .. }))
            .count();
        let results = ev
            .iter()
            .filter(|e| matches!(e, PresenterEvent::SubagentResult { .. }))
            .count();
        assert_eq!((starts, results), (2, 2), "start+result per child");

        // Children stream their activity → live progress events surface (Phase 3b).
        let progress = ev
            .iter()
            .filter(|e| matches!(e, PresenterEvent::SubagentProgress { .. }))
            .count();
        assert!(progress > 0, "at least one live progress delta surfaced");

        // Child usage rolled into the shared day budget (children did real model work).
        assert!(store.spend_today_usd().unwrap() > 0.0);
    }

    /// Parent: spawn once → follow up via send_to_agent → synthesize. Child: answers, then
    /// answers the follow-up WITH its prior context (persistent subagents, gap-analysis #12).
    struct SpawnThenFollowUpProvider;

    #[async_trait::async_trait]
    impl Provider for SpawnThenFollowUpProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_provider::ModelResponse;
            use forge_types::{new_id, ToolCall, Usage};
            let usage = Usage {
                input_tokens: 30,
                output_tokens: 12,
                cached_input_tokens: 0,
                cost_usd: 0.0,
            };
            let is_subagent = messages
                .iter()
                .any(|m| m.role == Role::System && m.content.contains("subagent"));
            if is_subagent {
                let user_turns = messages.iter().filter(|m| m.role == Role::User).count();
                // The follow-up turn must still SEE the first exchange — that's the whole point.
                let has_context = messages
                    .iter()
                    .any(|m| m.role == Role::Assistant && m.content.contains("first finding"));
                let content = if user_turns >= 2 {
                    assert!(has_context, "follow-up child lost its prior transcript");
                    "deeper: confirmed with prior context".to_string()
                } else {
                    "first finding: suspicious module".to_string()
                };
                on_event(StreamEvent::Text(content.clone()));
                return Ok(ModelResponse {
                    content,
                    tool_calls: vec![],
                    usage,
                    quotas: Vec::new(),
                });
            }
            let tool_rounds = messages.iter().filter(|m| m.role == Role::Tool).count();
            let (content, calls) = match tool_rounds {
                0 => (
                    "delegating".to_string(),
                    vec![ToolCall {
                        id: new_id(),
                        name: "spawn_agents".into(),
                        args: serde_json::json!({
                            "agents": [ { "agent": "scout", "task": "scan the auth module" } ]
                        }),
                    }],
                ),
                1 => (
                    "following up".to_string(),
                    vec![ToolCall {
                        id: new_id(),
                        name: "send_to_agent".into(),
                        args: serde_json::json!({
                            "agent": "scout",
                            "message": "dig deeper on that finding"
                        }),
                    }],
                ),
                _ => {
                    let c = "synthesized with follow-up".to_string();
                    on_event(StreamEvent::Text(c.clone()));
                    (c, vec![])
                }
            };
            Ok(ModelResponse {
                content,
                tool_calls: calls,
                usage,
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn send_to_agent_continues_a_persisted_child_with_its_context() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let config = tiered_config();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(SpawnThenFollowUpProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        let parent_id = session.id().to_string();

        let answer = session
            .run_turn("investigate the auth module and follow up on findings")
            .await
            .unwrap();
        assert!(answer.contains("synthesized"), "parent finished: {answer}");

        // ONE child total: the follow-up reused the persisted child, no second spawn.
        let children = store.named_child_sessions(&parent_id).unwrap();
        assert_eq!(children.len(), 1, "follow-up must not create a new child");
        let (child_id, title) = &children[0];
        // Named at spawn — the send_to_agent address book works by this title.
        assert_eq!(title.as_deref(), Some("scout"));

        // The child transcript holds BOTH exchanges: task + first answer, follow-up + deeper
        // answer (the provider itself asserts the follow-up turn saw the first finding).
        let msgs = store.load_messages(child_id).unwrap();
        let users: Vec<_> = msgs.iter().filter(|m| m.role == Role::User).collect();
        assert_eq!(users.len(), 2, "task + follow-up persisted");
        assert!(msgs
            .iter()
            .any(|m| m.role == Role::Assistant && m.content.contains("deeper: confirmed")));
    }

    #[test]
    fn child_addresses_resolve_by_name_then_prefix_most_recent_first() {
        use crate::subagent::resolve_child_address;
        let children = vec![
            ("aaa111".to_string(), Some("scout".to_string())),
            ("bbb222".to_string(), Some("critic".to_string())),
            ("ccc333".to_string(), Some("scout".to_string())),
        ];
        // Duplicate names: the most recent child answers.
        assert_eq!(
            resolve_child_address(&children, "scout"),
            Some(("ccc333".into(), "scout".into()))
        );
        assert_eq!(
            resolve_child_address(&children, "critic"),
            Some(("bbb222".into(), "critic".into()))
        );
        // Unique id prefix works; an ambiguous or unknown address does not.
        assert_eq!(
            resolve_child_address(&children, "bbb"),
            Some(("bbb222".into(), "critic".into()))
        );
        assert_eq!(resolve_child_address(&children, "zzz"), None);
        let ambiguous = vec![("abc1".to_string(), None), ("abc2".to_string(), None)];
        assert_eq!(resolve_child_address(&ambiguous, "abc"), None);
    }

    #[tokio::test]
    async fn subagents_route_independently_via_the_mesh() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let config = tiered_config();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(SpawnThenSynthProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        let parent_id = session.id().to_string();

        session
            .run_turn("design and architect a complex concurrency refactor across modules")
            .await
            .unwrap();

        // Parent routed Complex; the "fix the typo" child routed Trivial → different model.
        let parent_models = store.session_models(&parent_id).unwrap();
        assert_eq!(
            parent_models.first().map(String::as_str),
            Some("ollama::big")
        );

        let children = store.child_sessions(&parent_id).unwrap();
        let child_models: Vec<String> = children
            .iter()
            .flat_map(|c| store.session_models(c).unwrap())
            .collect();
        assert!(
            child_models.iter().any(|m| m == "ollama::small"),
            "a trivial child routed to the cheap tier independently: {child_models:?}"
        );
    }

    /// A provider where EVERY agent (top or subagent) tries to `spawn_agents` once, then answers.
    /// Used to prove recursion is bounded by `max_depth` (the registry refuses `spawn_agents`
    /// once depth is exhausted, so the chain terminates).
    #[derive(Default)]
    struct AlwaysRecurseProvider;

    #[async_trait::async_trait]
    impl Provider for AlwaysRecurseProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_provider::ModelResponse;
            use forge_types::{new_id, ToolCall, Usage};
            let used_tool = messages.iter().any(|m| m.role == Role::Tool);
            let usage = Usage {
                input_tokens: 5,
                output_tokens: 2,
                cached_input_tokens: 0,
                cost_usd: 0.0,
            };
            if used_tool {
                return Ok(ModelResponse {
                    content: "leaf answer".into(),
                    tool_calls: vec![],
                    usage,
                    quotas: Vec::new(),
                });
            }
            Ok(ModelResponse {
                content: "delegating deeper".into(),
                tool_calls: vec![ToolCall {
                    id: new_id(),
                    name: "spawn_agents".into(),
                    args: serde_json::json!({"agents": [{"task": "go deeper"}]}),
                }],
                usage,
                quotas: Vec::new(),
            })
        }
    }

    #[test]
    fn cycle_temper_advances_wraps_and_persists() {
        use forge_types::PermissionMode;
        let store = Arc::new(Store::open_in_memory().unwrap());
        let session = fresh_session(Arc::clone(&store), Config::default());
        let id = session.id().to_string();
        let mut session = session;

        // Default config now starts at AcceptEdits (Smith).
        assert_eq!(session.temper(), PermissionMode::AcceptEdits); // Smith
        assert_eq!(session.cycle_temper(), PermissionMode::Plan); // → Survey
        assert_eq!(store.session_mode(&id).unwrap(), "Plan", "switch persisted");
        assert_eq!(session.cycle_temper(), PermissionMode::Default); // → Guarded
        assert_eq!(session.cycle_temper(), PermissionMode::AcceptEdits); // wraps → Smith
                                                                         // Cycling never lands on the dangerous Unfettered temper.
        for _ in 0..6 {
            assert_ne!(session.cycle_temper(), PermissionMode::Bypass);
        }
    }

    #[tokio::test]
    async fn recursion_is_bounded_by_max_depth() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut config = tiered_config();
        config.mesh.subagents.max_depth = 2;
        config.mesh.subagents.max_concurrency = 2;
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(AlwaysRecurseProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        let parent_id = session.id().to_string();

        // Must terminate (not infinite-recurse / stack-overflow).
        session
            .run_turn("kick off a delegating turn")
            .await
            .unwrap();

        // Walk the parent→child tree; with max_depth=2 the chain is child→grandchild→
        // great-grandchild (depths 0,1,2) and stops — never a 4th generation.
        fn max_gen(store: &Store, id: &str) -> usize {
            let kids = store.child_sessions(id).unwrap();
            1 + kids.iter().map(|k| max_gen(store, k)).max().unwrap_or(0)
        }
        let generations = max_gen(&store, &parent_id);
        assert_eq!(
            generations, 4,
            "parent + 3 nested generations (depths 0,1,2), bounded by max_depth"
        );
    }

    #[tokio::test]
    async fn default_subagent_depth_stops_after_one_child_generation() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut config = tiered_config();
        assert_eq!(
            config.mesh.subagents.max_depth, 0,
            "recursive delegation must remain an explicit opt-in"
        );
        config.mesh.subagents.max_concurrency = 2;
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(AlwaysRecurseProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        let parent_id = session.id().to_string();

        session
            .run_turn("kick off a default delegating turn")
            .await
            .unwrap();

        fn max_gen(store: &Store, id: &str) -> usize {
            let kids = store.child_sessions(id).unwrap();
            1 + kids
                .iter()
                .map(|kid| max_gen(store, kid))
                .max()
                .unwrap_or(0)
        }
        assert_eq!(
            max_gen(&store, &parent_id),
            2,
            "the parent may delegate once, but default children must not recurse"
        );
    }

    #[tokio::test]
    async fn agent_type_file_pins_tier_alongside_mesh_routed_inline_child() {
        // A `.forge/agents/reviewer.md` pins tier=complex; the inline "fix the typo" child has
        // no pin and mesh-routes to trivial. Both must coexist in one spawn_agents call.
        let dir = std::env::temp_dir().join(format!("forge-agents-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("reviewer.md"),
            "---\nname: reviewer\ntier: complex\ntools: [read_file]\n---\nYou review code.",
        )
        .unwrap();

        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut config = tiered_config();
        config.mesh.subagents.agents_dir = dir.to_string_lossy().to_string();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(SpawnThenSynthProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        let parent_id = session.id().to_string();

        session
            .run_turn("design and architect a complex concurrency refactor across modules")
            .await
            .unwrap();

        let children = store.child_sessions(&parent_id).unwrap();
        let child_models: Vec<String> = children
            .iter()
            .flat_map(|c| store.session_models(c).unwrap())
            .collect();
        // reviewer pinned → complex tier model; the inline "fix typo" → trivial tier model.
        assert!(
            child_models.iter().any(|m| m == "ollama::big"),
            "pinned reviewer routed to its tier: {child_models:?}"
        );
        assert!(
            child_models.iter().any(|m| m == "ollama::small"),
            "inline child still mesh-routed cheaply: {child_models:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Model health / failover (docs/features/mesh-routing.md) ---

    /// A router that returns a fixed model + fallback chain, so the failover loop is testable
    /// without depending on discovery/availability.
    struct FixedRouter {
        model: String,
        fallbacks: Vec<String>,
    }
    #[async_trait::async_trait]
    impl Router for FixedRouter {
        async fn route(
            &self,
            _prompt: &str,
            _has_images: bool,
            _budget: BudgetState,
            _health: &forge_types::ModelHealth,
            _quota: &forge_types::SubscriptionQuota,
            _effort: Option<forge_types::EffortLevel>,
            _project: &forge_types::ProjectContext,
        ) -> forge_mesh::RoutingDecision {
            forge_mesh::RoutingDecision {
                tier: forge_types::TaskTier::Trivial,
                model: self.model.clone(),
                rationale: "test".into(),
                fallbacks: self.fallbacks.clone(),
                pinned: false,
            }
        }
        async fn route_with_pin_set(
            &self,
            pin: &[String],
            _p: &str,
            _has_images: bool,
            _b: BudgetState,
            _h: &forge_types::ModelHealth,
            _q: &forge_types::SubscriptionQuota,
            _effort: Option<forge_types::EffortLevel>,
            _project: &forge_types::ProjectContext,
        ) -> forge_mesh::RoutingDecision {
            forge_mesh::RoutingDecision {
                tier: forge_types::TaskTier::Trivial,
                model: pin.first().cloned().unwrap_or_else(|| self.model.clone()),
                rationale: "test".into(),
                fallbacks: self.fallbacks.clone(),
                pinned: true,
            }
        }
    }

    /// Like [`FixedRouter`], but the decision is an EXPLICIT pin (`--model`), so the strict-pin
    /// failover rules + the pinned rate-limit backoff apply. `fallbacks` are deliberately allowed,
    /// mirroring a legacy decision, so tests can prove they are NOT used for a pinned model.
    struct PinnedRouter {
        model: String,
        fallbacks: Vec<String>,
    }
    #[async_trait::async_trait]
    impl Router for PinnedRouter {
        async fn route(
            &self,
            _prompt: &str,
            _has_images: bool,
            _budget: BudgetState,
            _health: &forge_types::ModelHealth,
            _quota: &forge_types::SubscriptionQuota,
            _effort: Option<forge_types::EffortLevel>,
            _project: &forge_types::ProjectContext,
        ) -> forge_mesh::RoutingDecision {
            forge_mesh::RoutingDecision {
                tier: forge_types::TaskTier::Trivial,
                model: self.model.clone(),
                rationale: "pinned via --model".into(),
                fallbacks: self.fallbacks.clone(),
                pinned: true,
            }
        }
    }

    /// A provider that fails for `bad` models (with a chosen error) and answers for any other.
    struct FlakyProvider {
        bad: std::collections::HashSet<String>,
        err: fn(&str) -> forge_provider::ProviderError,
    }
    #[async_trait::async_trait]
    impl Provider for FlakyProvider {
        async fn complete(
            &self,
            model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            if self.bad.contains(model) {
                return Err((self.err)(model));
            }
            on_event(StreamEvent::Text("recovered".into()));
            Ok(forge_provider::ModelResponse {
                content: "recovered".into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    fn rate_limited(_m: &str) -> forge_provider::ProviderError {
        forge_provider::ProviderError::RateLimited {
            message: "429".into(),
            retry_after: Some(std::time::Duration::from_secs(42)),
        }
    }

    fn unavailable(_m: &str) -> forge_provider::ProviderError {
        forge_provider::ProviderError::Unavailable("502".into())
    }

    /// Overflow rides `Unavailable` in the wild (providers report it inconsistently) — the sniff in
    /// `ProviderError::is_context_overflow` is what tells it apart from a real outage.
    fn context_overflow(_m: &str) -> forge_provider::ProviderError {
        forge_provider::ProviderError::Unavailable(
            "maximum context length is 8192 tokens, however you requested 41000".into(),
        )
    }

    /// Fails `bad` models with a chosen error; every other model answers with its OWN id as the
    /// content, so a test can tell WHICH fallback actually served the turn.
    struct EchoProvider {
        bad: std::collections::HashSet<String>,
        err: fn(&str) -> forge_provider::ProviderError,
    }
    #[async_trait::async_trait]
    impl Provider for EchoProvider {
        async fn complete(
            &self,
            model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            if self.bad.contains(model) {
                return Err((self.err)(model));
            }
            on_event(StreamEvent::Text(model.into()));
            Ok(forge_provider::ModelResponse {
                content: model.into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn rate_limit_skips_the_failed_providers_remaining_chain_entries() {
        // Chain is in mesh-rank order [prova::2, provb::1]. prova::1 rate-limits — a 429 is
        // provider-wide, so the lazy-skip must pass over prova::2 (same provider) and cross to
        // provb::1, NOT churn through prova's siblings.
        let provider = Arc::new(EchoProvider {
            bad: ["prova::1".to_string()].into_iter().collect(),
            err: rate_limited,
        });
        let router = Arc::new(FixedRouter {
            model: "prova::1".into(),
            fallbacks: vec!["prova::2".into(), "provb::1".into()],
        });
        let (_store, mut session) = fixed_session(provider, router);
        let answer = session.run_turn("do it").await.unwrap();
        assert_eq!(
            answer, "provb::1",
            "429 on prova::1 must skip same-provider prova::2 and use provb::1"
        );
    }

    /// Narrates a tool call as TEXT for the first `narrate` completions, then answers cleanly.
    struct NarrateThenAnswerProvider {
        calls: std::sync::atomic::AtomicUsize,
        narrate: usize,
    }
    #[async_trait::async_trait]
    impl Provider for NarrateThenAnswerProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let text = if n < self.narrate {
                "<invoke name=\"shell\"><parameter name=\"command\">git push</parameter></invoke>"
            } else {
                "all done"
            };
            on_event(StreamEvent::Text(text.into()));
            Ok(forge_provider::ModelResponse {
                content: text.into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn narrated_tool_call_is_not_accepted_as_a_final_answer() {
        // A direct model writes a tool call as text (nothing executes). The honest-failure guard
        // must NOT accept it as the turn's answer — it nudges the model, which then answers for
        // real. Proven by the final text being the clean answer, not the narrated markup.
        let provider = Arc::new(NarrateThenAnswerProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            narrate: 1,
        });
        let router = Arc::new(FixedRouter {
            model: "direct::model".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        let answer = session.run_turn("ship it").await.unwrap();
        assert_eq!(
            answer, "all done",
            "narrated tool-call text must be nudged, not accepted as the final answer"
        );
    }

    #[tokio::test]
    async fn non_rate_limit_failure_keeps_strict_rank_order() {
        // A NON-429 failure (outage) must NOT skip the provider — strict mesh-rank order means the
        // very next-ranked model (prova::2) is tried, even though it shares prova::1's provider.
        let provider = Arc::new(EchoProvider {
            bad: ["prova::1".to_string()].into_iter().collect(),
            err: unavailable,
        });
        let router = Arc::new(FixedRouter {
            model: "prova::1".into(),
            fallbacks: vec!["prova::2".into(), "provb::1".into()],
        });
        let (_store, mut session) = fixed_session(provider, router);
        let answer = session.run_turn("do it").await.unwrap();
        assert_eq!(
            answer, "prova::2",
            "an outage keeps rank order — next-ranked prova::2 is tried, not skipped"
        );
    }

    /// Fails the first `fail_first` calls with a context-overflow error, then answers. Used to
    /// prove an overflow self-heals (compact + retry the SAME model) instead of benching it.
    struct OverflowThenOkProvider {
        calls: std::sync::atomic::AtomicUsize,
        fail_first: usize,
    }
    #[async_trait::async_trait]
    impl Provider for OverflowThenOkProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < self.fail_first {
                return Err(forge_provider::ProviderError::Unavailable(
                    "maximum context length is 128000 tokens".into(),
                ));
            }
            on_event(StreamEvent::Text("recovered".into()));
            Ok(forge_provider::ModelResponse {
                content: "recovered".into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn context_overflow_compacts_and_retries_the_same_model_without_benching() {
        // The first call overflows the window; the fix is to shrink the transcript and retry the
        // SAME (healthy) model — NOT to bench it and churn the failover chain (the stuck-turn bug).
        let provider = Arc::new(OverflowThenOkProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            fail_first: 1,
        });
        let router = Arc::new(FixedRouter {
            model: "good::model".into(),
            fallbacks: vec!["other::model".into()],
        });
        let (store, mut session) = fixed_session(provider, router);
        // Enough history that the compaction triggered by the overflow actually folds messages.
        for i in 0..12 {
            session
                .transcript
                .push(Message::user(format!("message {i}")));
        }
        let answer = session.run_turn("summarize the work").await.unwrap();
        assert_eq!(answer, "recovered", "the turn self-healed and completed");
        // The healthy model must NOT have been benched — overflow is an input problem, not a
        // model-health problem.
        let benched = store.current_benched_report().unwrap();
        assert!(
            benched.is_empty(),
            "overflow must not bench the model: {benched:?}"
        );
    }

    /// Rate-limits the first `fail_first` calls with a tiny `retry_after`, then answers — to prove
    /// the in-turn wait-for-reset retries an explicitly pinned model instead of degrading to a fallback.
    struct RateLimitThenOkProvider {
        calls: std::sync::atomic::AtomicUsize,
        fail_first: usize,
    }
    #[async_trait::async_trait]
    impl Provider for RateLimitThenOkProvider {
        async fn complete(
            &self,
            model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < self.fail_first {
                return Err(forge_provider::ProviderError::RateLimited {
                    message: "429 rate limited".into(),
                    retry_after: Some(std::time::Duration::from_millis(10)),
                });
            }
            on_event(StreamEvent::Text(model.into()));
            Ok(forge_provider::ModelResponse {
                content: model.into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn rate_limit_waits_for_reset_and_retries_the_same_model() {
        // An explicit pin keeps the requested model: a short 429 reset is waited out before
        // retrying it. Unpinned routes instead bench and immediately fail over.
        let provider = Arc::new(RateLimitThenOkProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            fail_first: 1,
        });
        let router = Arc::new(PinnedRouter {
            model: "best::model".into(),
            fallbacks: vec!["worse::model".into()],
        });
        let (store, mut session) = fixed_session(provider, router);
        session.config.mesh.rate_limit_wait_secs = 1; // re-enable waiting (10ms reset → instant)
        let answer = session.run_turn("hi").await.unwrap();
        assert_eq!(
            answer, "best::model",
            "waited for the reset and retried the best model; fallback unused"
        );
        assert!(
            store.current_benched().unwrap().is_empty(),
            "a model we waited out and recovered must not be benched"
        );
    }

    #[test]
    fn pinned_backoff_schedule_grows_caps_and_respects_retry_after() {
        use std::time::Duration;
        let secs = |a: u32, j: f64| pinned_backoff_delay(a, None, j).as_secs_f64();
        // jitter 0.5 → factor 1.0: the documented 5s/15s/45s schedule, capped at 60s from #4.
        for (attempt, want) in [
            (1, 5.0),
            (2, 15.0),
            (3, 45.0),
            (4, 60.0),
            (5, 60.0),
            (6, 60.0),
        ] {
            assert!(
                (secs(attempt, 0.5) - want).abs() < 1e-9,
                "attempt {attempt}: want {want}s"
            );
        }
        // Jitter bounds: ±20% of the base delay.
        assert!((secs(1, 0.0) - 4.0).abs() < 1e-9, "low jitter = 0.8×base");
        assert!((secs(1, 1.0) - 6.0).abs() < 1e-9, "high jitter = 1.2×base");
        // A server Retry-After is respected verbatim — it beats the blind schedule either way.
        assert_eq!(
            pinned_backoff_delay(1, Some(Duration::from_millis(10)), 0.5),
            Duration::from_millis(10)
        );
        assert_eq!(
            pinned_backoff_delay(1, Some(Duration::from_secs(90)), 0.5),
            Duration::from_secs(90)
        );
        // The full jittered schedule can exceed the wait budget, so the budget is the real cap.
        let worst: f64 = (1..=PINNED_RL_MAX_ATTEMPTS).map(|a| secs(a, 1.0)).sum();
        assert!(
            worst > PINNED_RL_TOTAL_WAIT_SECS as f64,
            "total budget must bind before the attempt cap at max jitter"
        );
    }

    /// Fails `Unavailable` (a transient outage, e.g. a stalled stream) `fail_first` times, then
    /// answers. Unlike [`RateLimitThenOkProvider`], the first [`MAX_TRANSIENT_RETRIES`] failures
    /// are absorbed by the hot same-model retry in the turn loop itself, before the pinned-outage
    /// backoff (pinned-outage-resilience §1) ever engages — so `fail_first` must exceed that to
    /// actually exercise the outage-backoff arm.
    struct UnavailableThenOkProvider {
        calls: std::sync::atomic::AtomicUsize,
        fail_first: usize,
    }
    #[async_trait::async_trait]
    impl Provider for UnavailableThenOkProvider {
        async fn complete(
            &self,
            model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < self.fail_first {
                return Err(forge_provider::ProviderError::Unavailable("502".into()));
            }
            on_event(StreamEvent::Text(model.into()));
            Ok(forge_provider::ModelResponse {
                content: model.into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    /// Fails with a transient outage (`Unavailable`) for the first `outage_calls`, then a
    /// rate-limit (`RateLimited`) for the next `rl_calls`, then answers — so a single turn drives
    /// BOTH pinned backoff paths in sequence, proving the outage attempt/budget counters
    /// (`pinned_outage_attempts`/`pinned_outage_waited`) and the rate-limit ones
    /// (`pinned_rl_attempts`/`pinned_rl_waited`) are independent: neither path's attempts are
    /// consumed by, or blocked by, the other having already run in the same turn.
    struct OutageThenRateLimitThenOkProvider {
        calls: std::sync::atomic::AtomicUsize,
        outage_calls: usize,
        rl_calls: usize,
    }
    #[async_trait::async_trait]
    impl Provider for OutageThenRateLimitThenOkProvider {
        async fn complete(
            &self,
            model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < self.outage_calls {
                return Err(forge_provider::ProviderError::Unavailable("502".into()));
            }
            if n < self.outage_calls + self.rl_calls {
                return Err(forge_provider::ProviderError::RateLimited {
                    message: "429".into(),
                    retry_after: Some(std::time::Duration::from_millis(10)),
                });
            }
            on_event(StreamEvent::Text(model.into()));
            Ok(forge_provider::ModelResponse {
                content: model.into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn rate_limited_pinned_model_backs_off_and_retries_the_same_model() {
        // Baseline defect (harness-robustness wave 2): pinned SWE-bench turns aborted
        // "skipped: rate limited" with ZERO retry. Two consecutive 429s (retry_after 10ms)
        // must be waited out on the SAME pinned model — the fallback stays unused and the
        // recovered model is never benched.
        let provider = Arc::new(RateLimitThenOkProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            fail_first: 2,
        });
        let router = Arc::new(PinnedRouter {
            model: "pin::model".into(),
            fallbacks: vec!["worse::model".into()],
        });
        let (store, mut session) = fixed_session(provider, router);
        let answer = session.run_turn("fix the bug").await.unwrap();
        assert_eq!(
            answer, "pin::model",
            "pinned model must be retried with backoff, not failed or switched"
        );
        assert!(
            store.current_benched().unwrap().is_empty(),
            "a pinned model that recovered after backoff must not be benched"
        );
    }

    #[tokio::test]
    async fn session_model_pin_engages_the_pinned_backoff_too() {
        // The `/model` (session) pin flows through `self.pinned_model`, not the routing
        // decision — it must get the same backoff treatment as a `--model` pin.
        let provider = Arc::new(RateLimitThenOkProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            fail_first: 1,
        });
        let router = Arc::new(FixedRouter {
            model: "routed::model".into(),
            fallbacks: vec!["worse::model".into()],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.pin_model(Some("pin::model".into()));
        let answer = session.run_turn("fix the bug").await.unwrap();
        assert_eq!(
            answer, "pin::model",
            "session pin retried on the same model"
        );
    }

    #[tokio::test]
    async fn pinned_model_recovers_from_a_transient_outage_via_backoff() {
        // Generic `Unavailable` bypasses the Codex-specific hot retry and takes one bounded
        // outage backoff before recovering on the same pinned model.
        let provider = Arc::new(UnavailableThenOkProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            fail_first: 1, // One outage backoff, then recovers.
        });
        let router = Arc::new(PinnedRouter {
            model: "pin::model".into(),
            fallbacks: vec!["worse::model".into()],
        });
        let (store, mut session) = fixed_session(provider, router);
        let answer = session.run_turn("fix the bug").await.unwrap();
        assert_eq!(
            answer, "pin::model",
            "an outage that recovers within the budget must retry the SAME pinned model"
        );
        assert!(
            store.current_benched().unwrap().is_empty(),
            "a pinned model that recovered after outage backoff must not be benched"
        );
    }

    #[tokio::test]
    async fn pinned_outage_backoff_warns_once_at_halfway_then_fails_on_exhaustion() {
        // A small budget (6s) so the halfway warning and exhaustion are both reachable without a
        // long real-time sleep: attempt 1's delay (jittered 4-6s off the 5s base) already exceeds
        // 50% of a 6s budget regardless of jitter, and attempt 2's delay (jittered 12-18s off the
        // 15s base) always exceeds whatever budget remains, so exhaustion follows without needing
        // a second real sleep.
        let provider = Arc::new(EchoProvider {
            bad: ["pin::model".to_string()].into_iter().collect(),
            err: unavailable,
        });
        let router = Arc::new(PinnedRouter {
            model: "pin::model".into(),
            fallbacks: vec!["worse::model".into()],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.mesh.pin_outage_wait_secs = 6;
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        session.presenter = Box::new(capture);
        let err = session.run_turn("fix the bug").await.unwrap_err();
        assert!(
            err.to_string().contains("502"),
            "the REAL provider error must surface, got: {err}"
        );
        let events = events.lock().unwrap();
        let halfway = events
            .iter()
            .filter(|e| {
                matches!(e, PresenterEvent::Warning(w) if w.contains("provider unreachable") && w.contains("retrying pinned model"))
            })
            .count();
        assert_eq!(halfway, 1, "the 50%-budget warning must fire exactly once");
        let exhausted = events
            .iter()
            .filter(|e| matches!(e, PresenterEvent::Warning(w) if w.contains("still unreachable")))
            .count();
        assert_eq!(
            exhausted, 1,
            "exhaustion fails with one warning mirroring the rate-limit exhaustion wording"
        );
        assert!(
            events.iter().any(|e| matches!(e, PresenterEvent::Warning(w) if w.contains("/model") && w.contains("pin_failover"))),
            "the exhaustion warning must carry the unpin / pin_failover hint"
        );
    }

    #[tokio::test]
    async fn pinned_outage_and_rate_limit_backoffs_use_independent_budgets() {
        // One turn drives BOTH pinned backoff paths in sequence (outage first, then rate-limit),
        // proving their attempt/budget counters don't share state: if they did, the rate-limit
        // attempts below (or their budget check) could be corrupted by the outage attempt that
        // already ran earlier in the same turn.
        let provider = Arc::new(OutageThenRateLimitThenOkProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            outage_calls: 3, // 2 hot-retry absorbed, 1 outage-backoff attempt (~4-6s real sleep).
            rl_calls: 2,     // 2 rate-limit backoff attempts (10ms retry_after each, fast).
        });
        let router = Arc::new(PinnedRouter {
            model: "pin::model".into(),
            fallbacks: vec!["worse::model".into()],
        });
        let (store, mut session) = fixed_session(provider, router);
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        session.presenter = Box::new(capture);
        let answer = session.run_turn("fix the bug").await.unwrap();
        assert_eq!(
            answer, "pin::model",
            "both backoff paths must recover the SAME pinned model within one turn"
        );
        assert!(
            store.current_benched().unwrap().is_empty(),
            "a pinned model that recovered must not be benched"
        );
        let events = events.lock().unwrap();
        assert!(
            events.iter().any(|e| matches!(e, PresenterEvent::Warning(w) if w.contains("rate limited") && w.contains("attempt 1/"))),
            "the rate-limit path must still run its own attempt 1, unaffected by the earlier outage attempt"
        );
        assert!(
            events.iter().any(|e| matches!(e, PresenterEvent::Warning(w) if w.contains("rate limited") && w.contains("attempt 2/"))),
            "the rate-limit path must reach attempt 2 — its budget wasn't pre-consumed by the outage attempt"
        );
        assert!(
            !events.iter().any(|e| matches!(e, PresenterEvent::Warning(w) if w.contains("still rate limited") || w.contains("still unreachable"))),
            "the turn recovered — neither budget was exhausted"
        );
    }

    #[tokio::test]
    async fn pin_outage_wait_secs_zero_disables_outage_backoff_and_fails_immediately() {
        // `mesh.pin_outage_wait_secs = 0` restores the pre-outage-resilience FailTurn behaviour:
        // the hot same-model transient retries still run (2 quick sleeps, same as any transient
        // failure), but the multi-attempt, multi-second outage BACKOFF is skipped entirely —
        // `failover_policy` sees `transient_outage=false` and fails the turn right away.
        let provider = Arc::new(EchoProvider {
            bad: ["pin::model".to_string()].into_iter().collect(),
            err: unavailable,
        });
        let router = Arc::new(PinnedRouter {
            model: "pin::model".into(),
            fallbacks: vec!["worse::model".into()],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.mesh.pin_outage_wait_secs = 0;
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        session.presenter = Box::new(capture);
        let err = session.run_turn("fix the bug").await.unwrap_err();
        assert!(
            err.to_string().contains("502"),
            "the REAL provider error must surface, got: {err}"
        );
        assert!(
            !events
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, PresenterEvent::Warning(w) if w.contains("unreachable"))),
            "wait_secs=0 must skip the outage backoff entirely — no outage warning at all"
        );
    }

    #[test]
    fn failover_chooser_forbids_cross_model_switching_for_pins() {
        // Table test on the single failover chooser (strict pin semantics, fix 2; extended by
        // pinned-outage-resilience §1 with `transient_outage`):
        // (pinned, pin_failover escape hatch, rate_limited, transient_outage) → what the loop
        // may do. The caller folds `mesh.pin_outage_wait_secs > 0` into `transient_outage`
        // itself, so `transient_outage=false` also covers the "outage backoff disabled" case —
        // no separate table row needed for that; it collapses to the same FailTurn as "permanent".
        use FailoverPolicy::*;
        let table = [
            // Unpinned turns: normal failover regardless of error kind or escape hatch.
            (false, false, true, false, SwitchModels),
            (false, false, false, false, SwitchModels),
            (false, false, false, true, SwitchModels),
            (false, true, true, false, SwitchModels),
            (false, true, false, false, SwitchModels),
            // Pinned + strict (default): rate limit OR transient outage → same-model backoff
            // (on their own separate budgets, enforced at the call site, not here); a permanent
            // error (neither flag set) → fail the turn with the real error. Never a silent switch.
            (true, false, true, false, BackoffSameModel),
            (true, false, false, true, BackoffSameModel),
            (true, false, false, false, FailTurn),
            // Pinned + permanent error (capability/auth): `is_permanent()` forces
            // `transient_outage=false` at the call site regardless of `pin_outage_wait_secs`, so
            // this is FailTurn even with outage backoff enabled.
            (true, false, false, false, FailTurn),
            // Pinned + escape hatch: the old switch-away behaviour, end to end.
            (true, true, true, false, SwitchModels),
            (true, true, false, true, SwitchModels),
            (true, true, false, false, SwitchModels),
        ];
        for (pinned, hatch, rl, outage, want) in table {
            assert_eq!(
                failover_policy(pinned, hatch, rl, outage),
                want,
                "pinned={pinned} pin_failover={hatch} rate_limited={rl} transient_outage={outage}"
            );
        }
    }

    #[test]
    fn pin_outage_wait_secs_zero_gate_restores_fail_turn() {
        // `mesh.pin_outage_wait_secs = 0` disables outage backoff (pinned-outage-resilience §3):
        // the call site computes `transient_outage = !permanent && !rate_limited && wait_secs >
        // 0`, so a `0` budget must fold straight into `FailTurn` — the exact wiring the turn loop
        // uses, exercised here without needing a full `run_model_loop` provider fixture.
        let permanent = false;
        let rate_limited = false;
        for wait_secs in [0u64, 600] {
            let transient_outage = !permanent && !rate_limited && wait_secs > 0;
            let want = if wait_secs == 0 {
                FailoverPolicy::FailTurn
            } else {
                FailoverPolicy::BackoffSameModel
            };
            assert_eq!(
                failover_policy(true, false, rate_limited, transient_outage),
                want,
                "pin_outage_wait_secs={wait_secs}"
            );
        }
    }

    fn capability(_m: &str) -> forge_provider::ProviderError {
        forge_provider::ProviderError::Capability("no tool support".into())
    }

    #[tokio::test]
    async fn pinned_model_with_a_permanent_error_fails_the_turn_with_the_real_cause() {
        // Strict pins: a pinned model that permanently can't serve the turn must FAIL the turn
        // with the real error — not silently run the fallback (benchmark contamination).
        let provider = Arc::new(EchoProvider {
            bad: ["pin::model".to_string()].into_iter().collect(),
            err: capability,
        });
        let router = Arc::new(PinnedRouter {
            model: "pin::model".into(),
            fallbacks: vec!["worse::model".into()],
        });
        let (_store, mut session) = fixed_session(provider, router);
        let err = session.run_turn("fix the bug").await.unwrap_err();
        assert!(
            err.to_string().contains("no tool support"),
            "the REAL provider error must surface, got: {err}"
        );
    }

    #[tokio::test]
    async fn pin_failover_escape_hatch_restores_cross_model_switching() {
        // `mesh.pin_failover = true` deliberately restores the old behaviour: a failing pinned
        // model may switch to the decision's fallbacks.
        let provider = Arc::new(EchoProvider {
            bad: ["pin::model".to_string()].into_iter().collect(),
            err: unavailable,
        });
        let router = Arc::new(PinnedRouter {
            model: "pin::model".into(),
            fallbacks: vec!["worse::model".into()],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.mesh.pin_failover = true;
        let answer = session.run_turn("fix the bug").await.unwrap();
        assert_eq!(answer, "worse::model", "escape hatch allows the old switch");
    }

    // --- Empty-diff completion nudge (harness-robustness wave 2, fix 4) ---

    /// Scripted "describe instead of implement" model: completion 1 explores (one read-only tool
    /// call), later completions only narrate — no tool calls, no edits. Counts completions so a
    /// test can prove the nudge fired exactly once.
    struct DescribeOnlyProvider {
        calls: std::sync::atomic::AtomicUsize,
    }
    #[async_trait::async_trait]
    impl Provider for DescribeOnlyProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                return Ok(forge_provider::ModelResponse {
                    content: String::new(),
                    tool_calls: vec![forge_types::ToolCall {
                        id: forge_types::new_id(),
                        name: "list_dir".into(),
                        args: serde_json::json!({ "path": "." }),
                    }],
                    usage: forge_types::Usage::default(),
                    quotas: Vec::new(),
                });
            }
            let text = if n == 1 {
                "here is how you would fix it"
            } else {
                "still only describing"
            };
            on_event(StreamEvent::Text(text.into()));
            Ok(forge_provider::ModelResponse {
                content: text.into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    /// A throwaway git repo with one committed file and a CLEAN tree, so
    /// the turn baseline is deterministic regardless of the checkout state of the repo the tests
    /// happen to run in.
    fn clean_git_repo() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("forge-nudge-{}", forge_types::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .expect("git");
            assert!(out.status.success(), "git {args:?} failed");
        };
        git(&["init", "-q"]);
        // Pin line-ending handling so the repo is byte-deterministic regardless of the host's
        // global git config. Windows CI images default to `core.autocrlf=true`, which rewrites
        // LF→CRLF on checkout — so a `git stash push` restore of a file this test wrote with LF
        // would come back with CRLF and fail the byte-equality asserts. Disabling autocrlf keeps
        // the on-disk bytes exactly what the test committed on every platform.
        git(&["config", "core.autocrlf", "false"]);
        git(&["config", "core.safecrlf", "false"]);
        git(&["config", "core.eol", "lf"]);
        std::fs::write(dir.join("f.txt"), "seed").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-qm", "seed"]);
        dir
    }

    #[test]
    fn turn_progress_ignores_preexisting_untracked_worktree_scaffolding() {
        let dir = clean_git_repo();
        std::fs::create_dir_all(dir.join(".cargo")).unwrap();
        std::fs::write(dir.join(".cargo/config.toml"), "[build]\n").unwrap();
        let baseline = working_tree_status(Some(&dir)).unwrap();

        assert!(String::from_utf8_lossy(&baseline).contains(".cargo/"));
        assert!(!working_tree_changed_since(Some(&dir), Some(&baseline)));

        std::fs::write(dir.join("f.txt"), "task edit").unwrap();
        assert!(working_tree_changed_since(Some(&dir), Some(&baseline)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn empty_diff_code_change_run_is_nudged_until_diminishing_returns() {
        // Baseline defect: 2 SWE-bench instances "completed" with an empty diff and no pushback.
        // A code-change run (bench sets `expect_code_change`) whose turn ran tools but edited
        // nothing gets pushed back — and, under the H8 continuation guard, keeps being re-driven
        // while there's budget headroom and no progress, then STOPS on diminishing returns. This
        // model only ever describes (tiny output, empty tree), so the guard nudges CONTINUATION_
        // DIMINISHING_MIN (3) times, sees each re-drive grow the transcript by < the token floor,
        // and halts on the 4th check — 2 primary completions + 3 continuation re-drives.
        let dir = clean_git_repo();
        let provider = Arc::new(DescribeOnlyProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let calls = Arc::clone(&provider);
        let router = Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        // The recap + auto-memory summarizers also call the provider at turn end — disable them
        // so the call count below measures ONLY the main loop's completions.
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.workspace = WorkspaceContext::new(&dir).unwrap();
        session.set_expect_code_change(true);
        let answer = session.run_turn("fix the bug").await.unwrap();
        assert_eq!(
            calls.calls.load(std::sync::atomic::Ordering::SeqCst),
            2 + CONTINUATION_DIMINISHING_MIN,
            "explore + describe (2 completions), then 3 continuation re-drives before the \
             diminishing-returns stop"
        );
        assert_eq!(answer, "still only describing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn explicit_interactive_change_contract_nudges_an_empty_diff() {
        // A direct `fix ...` request now carries the same artifact requirement that previously
        // existed only for SWE-bench. The scripted model explores but merely describes, so Forge
        // re-drives it rather than accepting a phantom implementation.
        let dir = clean_git_repo();
        let provider = Arc::new(DescribeOnlyProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let calls = Arc::clone(&provider);
        let router = Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.workspace = WorkspaceContext::new(&dir).unwrap();
        let answer = session.run_turn("fix the bug").await.unwrap();
        assert_eq!(
            calls.calls.load(std::sync::atomic::Ordering::SeqCst),
            2 + CONTINUATION_DIMINISHING_MIN,
            "the explicit change contract must re-drive an empty implementation"
        );
        assert_eq!(answer, "still only describing");
        assert!(session.last_turn_contract().requires_changed_artifact());
        assert!(session
            .last_context_pack()
            .entries()
            .iter()
            .any(|entry| entry.source() == context_pack::ContextSource::TurnContract));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn ambiguous_interactive_question_does_not_add_a_recovery_loop() {
        // Asking how one *would* fix something is advisory. The contract deliberately does not
        // guess that it is an implementation request, preserving existing one-pass behavior.
        let dir = clean_git_repo();
        let provider = Arc::new(DescribeOnlyProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let calls = Arc::clone(&provider);
        let router = Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.workspace = WorkspaceContext::new(&dir).unwrap();
        let answer = session
            .run_turn("How would you fix the bug?")
            .await
            .unwrap();
        assert_eq!(calls.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(answer, "here is how you would fix it");
        assert!(!session.last_turn_contract().requires_changed_artifact());
        assert!(session.last_context_pack().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn empty_diff_nudge_respects_the_config_gate() {
        // `mesh.nudge_empty_diff = false` disables the push-back even for bench runs.
        let dir = clean_git_repo();
        let provider = Arc::new(DescribeOnlyProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let calls = Arc::clone(&provider);
        let router = Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.workspace = WorkspaceContext::new(&dir).unwrap();
        session.set_expect_code_change(true);
        session.config.mesh.nudge_empty_diff = false;
        let answer = session.run_turn("fix the bug").await.unwrap();
        assert_eq!(calls.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(answer, "here is how you would fix it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Scripted CLI-BRIDGE model (wave 6). A claude-cli/codex-cli bridge runs its WHOLE tool loop
    /// inside one `complete()` in a subprocess and surfaces each tool as a `StreamEvent::ToolStarted`
    /// through the sink — never in `resp.tool_calls`. So the empty-diff nudge's `turn_tools_ran > 0`
    /// gate must count sink tool starts, not just direct `resp.tool_calls`, or it stays blind to the
    /// exact path every bridge benchmark uses. `edit_file`, when set, makes the first completion
    /// write a real file (a non-empty diff) so the "edited → must NOT nudge" case is exercised too.
    struct BridgeDescribeProvider {
        calls: std::sync::atomic::AtomicUsize,
        edit_file: Option<std::path::PathBuf>,
        /// Whether the first completion surfaces a tool via the sink. `false` models a bridge that
        /// yields with an empty diff having surfaced NO parseable tool event (refusal / prose-only /
        /// CLI output drift) — the case the wave-6 bridge-path relaxation covers.
        emit_tool: bool,
    }
    #[async_trait::async_trait]
    impl Provider for BridgeDescribeProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                // The bridge subprocess ran a tool: it surfaces via the sink, NOT resp.tool_calls.
                if self.emit_tool {
                    on_event(StreamEvent::ToolStarted {
                        name: "shell".into(),
                        args: "ls".into(),
                    });
                    on_event(StreamEvent::ToolFinished {
                        name: "shell".into(),
                        ok: true,
                        summary: String::new(),
                    });
                }
                if let Some(p) = &self.edit_file {
                    std::fs::write(p, "patched").unwrap();
                }
                let text = "explored the repo — here is how you would fix it";
                on_event(StreamEvent::Text(text.into()));
                return Ok(forge_provider::ModelResponse {
                    content: text.into(),
                    tool_calls: vec![],
                    usage: forge_types::Usage::default(),
                    quotas: Vec::new(),
                });
            }
            let text = "still only describing after the nudge";
            on_event(StreamEvent::Text(text.into()));
            Ok(forge_provider::ModelResponse {
                content: text.into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn bridge_empty_diff_run_is_nudged_until_diminishing_returns() {
        // Wave 6: the empty-diff nudge must fire on the CLI-BRIDGE path. Hard evidence: a 15-instance
        // SWE-bench Lite sweep on the codex-cli::gpt-5.5 bridge resolved 3/15 vs raw codex 9/15;
        // 8/15 bridge instances submitted an EMPTY patch and the nudge fired 0×. The bridge ran its
        // tools inside its subprocess (surfaced via the sink's ToolStarted), so the gate must see
        // that activity. Here the bridge explores (one sink tool) then only describes → the H8
        // continuation guard nudges 3× (each re-drive tiny + empty tree) before the diminishing-
        // returns stop: 1 primary bridge completion + 3 continuation re-drives.
        let dir = clean_git_repo();
        let provider = Arc::new(BridgeDescribeProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            edit_file: None,
            emit_tool: true,
        });
        let calls = Arc::clone(&provider);
        let router = Arc::new(FixedRouter {
            model: "codex-cli::gpt-5.5".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.workspace = WorkspaceContext::new(&dir).unwrap();
        session.set_expect_code_change(true);
        // This test isolates the empty-diff recovery loop; completeness has its own
        // bridge re-drive tests and would add one unrelated provider invocation.
        session.config.mesh.verify_completeness = false;
        let answer = session.run_turn("fix the bug").await.unwrap();
        assert_eq!(
            calls.calls.load(std::sync::atomic::Ordering::SeqCst),
            1 + CONTINUATION_DIMINISHING_MIN,
            "bridge yields its whole loop in ONE completion, then 3 continuation re-drives before \
             the diminishing-returns stop"
        );
        assert_eq!(answer, "still only describing after the nudge");
        // The synthetic nudge must actually have been injected (not just an extra completion).
        assert!(
            session
                .transcript
                .iter()
                .any(|m| m.content == EMPTY_DIFF_NUDGE),
            "the empty-diff nudge message was injected on the bridge path"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn bridge_that_edited_files_is_not_nudged() {
        // The counterpart guard: a bridge turn that DID change the tree (non-empty diff) must NOT
        // be nudged, so a real fix is never second-guessed.
        let dir = clean_git_repo();
        let provider = Arc::new(BridgeDescribeProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            edit_file: Some(dir.join("patch.txt")),
            emit_tool: true,
        });
        let calls = Arc::clone(&provider);
        let router = Arc::new(FixedRouter {
            model: "codex-cli::gpt-5.5".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.workspace = WorkspaceContext::new(&dir).unwrap();
        session.set_expect_code_change(true);
        // Isolate the empty-diff guard from the independent completeness review.
        session.config.mesh.verify_completeness = false;
        let answer = session.run_turn("fix the bug").await.unwrap();
        assert_eq!(
            calls.calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a bridge turn that edited the tree must not be nudged"
        );
        assert_eq!(answer, "explored the repo — here is how you would fix it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn bridge_empty_diff_with_no_surfaced_tool_still_nudges() {
        // Wave-6 bridge-path robustness: a bridge that yields an empty diff having surfaced NO
        // parseable tool event (refusal / prose-only / CLI output drift → `tools_ran == 0`) still
        // gets pushed back. The direct-path `tools_ran > 0` gate would have dropped it on the very
        // path every bench uses; the `is_cli_bridge` arm relaxes that requirement. Under the H8
        // guard it is re-driven 3× before the diminishing-returns stop.
        let dir = clean_git_repo();
        let provider = Arc::new(BridgeDescribeProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            edit_file: None,
            emit_tool: false,
        });
        let calls = Arc::clone(&provider);
        let router = Arc::new(FixedRouter {
            model: "codex-cli::gpt-5.5".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.workspace = WorkspaceContext::new(&dir).unwrap();
        session.set_expect_code_change(true);
        let answer = session.run_turn("fix the bug").await.unwrap();
        assert_eq!(
            calls.calls.load(std::sync::atomic::Ordering::SeqCst),
            1 + CONTINUATION_DIMINISHING_MIN,
            "bridge empty diff with no surfaced tool must still be nudged (3× before the stop)"
        );
        assert_eq!(answer, "still only describing after the nudge");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn direct_mutating_turn_that_only_plans_is_nudged() {
        // Live serve regression: a Full-mode worktree task returned only a three-step plan and no
        // tool calls. An explicit implementation contract must re-drive that zero-diff turn too;
        // requiring prior tool activity silently accepted the stall.
        let dir = clean_git_repo();
        let provider = Arc::new(BridgeDescribeProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            edit_file: None,
            emit_tool: false,
        });
        let calls = Arc::clone(&provider);
        let router = Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.workspace = WorkspaceContext::new(&dir).unwrap();
        session.set_expect_code_change(true);
        let answer = session.run_turn("fix the bug").await.unwrap();
        assert_eq!(
            calls.calls.load(std::sync::atomic::Ordering::SeqCst),
            1 + CONTINUATION_DIMINISHING_MIN,
            "a direct mutating turn that only planned must be pushed to execute"
        );
        assert_eq!(answer, "still only describing after the nudge");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Toolless-bridge classification (bridge MCP-tool health guard, wave 7) ---

    #[test]
    fn classify_tools_unavailable_requires_all_signals() {
        // The positive case: an expect_code_change bridge turn that saw the mcp-startup failure,
        // ran zero forge tools, and left the tree unchanged → TOOLS-UNAVAILABLE.
        assert!(classify_tools_unavailable(true, true, true, 0, true));
        // A NORMAL empty completion (no startup-failure signal) is NOT tools-unavailable — that's
        // the wave-2 empty-diff nudge's job, kept distinct.
        assert!(!classify_tools_unavailable(true, true, false, 0, true));
        // Tools actually ran → mcp-serve came up; not toolless.
        assert!(!classify_tools_unavailable(true, true, true, 3, true));
        // The tree changed → the model DID edit; not a toolless empty run.
        assert!(!classify_tools_unavailable(true, true, true, 0, false));
        // Not a bridge (direct model) → never classified.
        assert!(!classify_tools_unavailable(true, false, true, 0, true));
        // Not a code-change run (interactive) → never classified.
        assert!(!classify_tools_unavailable(false, true, true, 0, true));
    }

    /// Scripted CLI-bridge that emits `StreamEvent::ToolsUnavailable` on its FIRST completion —
    /// modelling a bridge whose `forge mcp-serve` tool server failed to start (wave 7): it ran no
    /// tools, edited nothing, and reported prose. Every completion yields prose with an empty tree.
    struct BridgeToollessProvider {
        calls: std::sync::atomic::AtomicUsize,
    }
    #[async_trait::async_trait]
    impl Provider for BridgeToollessProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                on_event(StreamEvent::ToolsUnavailable {
                    reason: "resources/list failed: MCP startup failed: No such file or directory \
                             (os error 2)"
                        .into(),
                });
            }
            let text = "I can't edit — no writable tool is exposed here.";
            on_event(StreamEvent::Text(text.into()));
            Ok(forge_provider::ModelResponse {
                content: text.into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn toolless_bridge_turn_is_classified_tools_unavailable() {
        // Wave 7: a bridge turn whose mcp-serve failed to start (ToolsUnavailable event), ran no
        // tools, and left an empty tree must be classified TOOLS-UNAVAILABLE so the harness retries
        // — NOT scored as a clean empty completion.
        let dir = clean_git_repo();
        let provider = Arc::new(BridgeToollessProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let router = Arc::new(FixedRouter {
            model: "codex-cli::gpt-5.5".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.workspace = WorkspaceContext::new(&dir).unwrap();
        session.set_expect_code_change(true);
        session.run_turn("fix the bug").await.unwrap();
        assert!(
            session.tools_unavailable(),
            "a toolless bridge turn (mcp-serve startup failure) must be classified TOOLS-UNAVAILABLE"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn normal_empty_completion_is_not_tools_unavailable() {
        // Distinctness from the wave-2 nudge: a bridge that yields an empty diff WITHOUT any
        // mcp-startup-failure signal (it simply described the fix) is nudged, but is NOT classified
        // TOOLS-UNAVAILABLE — the harness must not retry it as a broken-tools turn.
        let dir = clean_git_repo();
        let provider = Arc::new(BridgeDescribeProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            edit_file: None,
            emit_tool: false,
        });
        let router = Arc::new(FixedRouter {
            model: "codex-cli::gpt-5.5".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.workspace = WorkspaceContext::new(&dir).unwrap();
        session.set_expect_code_change(true);
        session.run_turn("fix the bug").await.unwrap();
        assert!(
            !session.tools_unavailable(),
            "a normal empty completion (no startup-failure signal) is NOT tools-unavailable"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tools_unavailable_respects_the_config_gate() {
        // `mesh.bridge_require_tools = false` disables the classification even for a bench run that
        // saw the mcp-startup failure.
        let dir = clean_git_repo();
        let provider = Arc::new(BridgeToollessProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let router = Arc::new(FixedRouter {
            model: "codex-cli::gpt-5.5".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.workspace = WorkspaceContext::new(&dir).unwrap();
        session.set_expect_code_change(true);
        session.config.mesh.bridge_require_tools = false;
        session.run_turn("fix the bug").await.unwrap();
        assert!(
            !session.tools_unavailable(),
            "the config gate must suppress the classification"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Existing-tests-are-spec guard (quality guards wave 4, fix 1) ---

    #[test]
    fn test_path_classifier_matches_the_pattern_list() {
        for p in [
            "tests/test_dataset.py",
            "xarray/tests/test_concat.py",
            "pkg/foo_test.py",
            "src/parser_test.rs",
            "src/parser_tests.rs",
            "test_units.rs",
            "web/app.test.ts",
            "web/app.spec.js",
            "tests/helpers.py", // under a tests/ dir counts even without a test_ name
        ] {
            assert!(is_test_path(p), "{p} must classify as a test path");
        }
        for p in [
            "src/lib.rs",
            "xarray/core/concat.py",
            "docs/testing.md",
            "attest.py",
        ] {
            assert!(!is_test_path(p), "{p} must NOT classify as a test path");
        }
    }

    #[test]
    fn modified_test_paths_flags_m_and_d_but_never_new_files() {
        // The red flag is a MODIFIED (or deleted) existing test; a NEW test file (`A` staged or
        // `??` untracked) is normal practice and must never trip the guard.
        let porcelain = " M xarray/tests/test_concat.py\n\
                         M  tests/test_merge.py\n\
                         D  tests/test_old.py\n\
                         A  tests/test_new.py\n\
                         ?? tests/test_scratch.py\n\
                         M  xarray/core/concat.py\n\
                         R  tests/test_a.py -> tests/test_b.py\n";
        assert_eq!(
            modified_test_paths(porcelain),
            vec![
                "xarray/tests/test_concat.py".to_string(),
                "tests/test_merge.py".to_string(),
                "tests/test_old.py".to_string(),
            ]
        );
    }

    #[test]
    fn explicit_no_weaken_contract_persists_across_session_transcript() {
        let messages = vec![
            Message::user(
                "Fix the implementation. Do not weaken, skip, or delete tests; add coverage.",
            ),
            Message::assistant("Implementation in progress."),
            Message::user("Continue the same work."),
        ];
        assert!(session_requires_pristine_existing_tests(&messages));
        assert!(!session_requires_pristine_existing_tests(&[
            Message::user("Add and update the tests for the new API."),
            Message::assistant("Working."),
        ]));
    }

    #[test]
    fn fault_seam_audit_requires_an_explicit_production_failure_task() {
        assert!(prompt_requires_fault_seam_audit(
            "Exercise an injected storage failure and rollback every partial write."
        ));
        assert!(prompt_requires_fault_seam_audit(
            "Use fault-injection to verify the persistence save path."
        ));
        assert!(!prompt_requires_fault_seam_audit(
            "Review error handling and add ordinary edge-case tests."
        ));
        assert!(!prompt_requires_fault_seam_audit(
            "Inject a clock so time-based tests are deterministic."
        ));
    }

    #[tokio::test]
    async fn pristine_test_guidance_is_proactive_and_injected_only_once_per_session() {
        let provider = Arc::new(CountingFinalProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let router = Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;

        session
            .run_turn("Review the suite and keep existing tests unchanged.")
            .await
            .unwrap();
        session.run_turn("Continue the same review.").await.unwrap();

        let guidance_positions = session
            .transcript
            .iter()
            .enumerate()
            .filter_map(|(index, message)| {
                (message.role == Role::System && message.content == PRISTINE_TEST_GUIDANCE)
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            guidance_positions.len(),
            1,
            "the stable contract must not accumulate on continuation turns"
        );
        let first_user = session
            .transcript
            .iter()
            .position(|message| message.role == Role::User)
            .unwrap();
        assert!(
            guidance_positions[0] < first_user,
            "the model must see the preventive contract before the task-defining prompt"
        );
    }

    #[tokio::test]
    async fn fault_seam_guidance_starts_on_triggering_turn_and_is_injected_once() {
        let provider = Arc::new(CountingFinalProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let router = Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;

        let first_prompt = "Fix the persistence implementation and run its tests.";
        let triggering_prompt =
            "Now exercise an injected storage failure, rollback partial writes, and verify it.";
        let continuation_prompt = "Continue the same rollback review.";
        session.run_turn(first_prompt).await.unwrap();
        session.run_turn(triggering_prompt).await.unwrap();
        session.run_turn(continuation_prompt).await.unwrap();

        let guidance_positions = session
            .transcript
            .iter()
            .enumerate()
            .filter_map(|(index, message)| {
                (message.role == Role::System && message.content == FAULT_SEAM_GUIDANCE)
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            guidance_positions.len(),
            1,
            "the fault-seam contract must not accumulate on continuation turns"
        );
        let prompt_position = |prompt: &str| {
            session
                .transcript
                .iter()
                .position(|message| message.role == Role::User && message.content == prompt)
                .unwrap()
        };
        let first_user = prompt_position(first_prompt);
        let triggering_user = prompt_position(triggering_prompt);
        let continuation_user = prompt_position(continuation_prompt);
        assert!(
            first_user < guidance_positions[0]
                && guidance_positions[0] < triggering_user
                && guidance_positions[0] < continuation_user,
            "guidance must begin immediately before the first triggering turn"
        );
    }

    /// A throwaway git repo with a committed test file (plus a source file) whose test is then
    /// MODIFIED in the working tree — the xarray-3364 shape the guard exists for.
    fn repo_with_modified_test() -> std::path::PathBuf {
        let dir = clean_git_repo();
        std::fs::create_dir_all(dir.join("tests")).unwrap();
        std::fs::write(dir.join("tests/test_foo.py"), "assert fix() == 1\n").unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .expect("git");
            assert!(out.status.success(), "git {args:?} failed");
        };
        git(&["add", "-A"]);
        git(&["commit", "-qm", "add test"]);
        // The turn "rewrites the test's expectations".
        std::fs::write(dir.join("tests/test_foo.py"), "assert fix() == 2\n").unwrap();
        dir
    }

    #[tokio::test]
    async fn modified_existing_tests_are_stashed_and_the_model_pushed_back_once() {
        let dir = repo_with_modified_test();
        let provider = Arc::new(DescribeOnlyProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let calls = Arc::clone(&provider);
        let router = Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.workspace = WorkspaceContext::new(&dir).unwrap();
        session.set_expect_code_change(true);
        let answer = session.run_turn("fix the bug").await.unwrap();
        assert_eq!(
            calls.calls.load(std::sync::atomic::Ordering::SeqCst),
            3 + CONTINUATION_DIMINISHING_MIN,
            "explore + describe, three bounded empty-diff continuations, then exactly ONE \
             pristine-test guard re-drive"
        );
        assert_eq!(answer, "still only describing");
        // The pristine test was restored (the stash took the rewritten expectations with it)…
        assert_eq!(
            std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap(),
            "assert fix() == 1\n",
            "the test file must be back at its committed content"
        );
        // …and the edits are recoverable, not destroyed.
        let stashes = std::process::Command::new("git")
            .args(["stash", "list"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            !stashes.stdout.is_empty(),
            "the test edits must be stashed, not discarded"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn explicit_no_weaken_contract_arms_test_guard_in_regular_chat() {
        let dir = repo_with_modified_test();
        let provider = Arc::new(DescribeOnlyProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let router = Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.workspace = WorkspaceContext::new(&dir).unwrap();

        session
            .run_turn("Fix the bug. Do not weaken, skip, or delete tests.")
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap(),
            "assert fix() == 1\n",
            "explicit session contract must restore committed tests without bench mode"
        );
        let stashes = std::process::Command::new("git")
            .args(["stash", "list"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(!stashes.stdout.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn bridge_modified_tests_are_stashed_and_pushed_back_once() {
        // Wave 6: the pristine-test guard is git-tree-based (it inspects the working tree, not
        // `resp.tool_calls`), so it already covers a CLI-BRIDGE turn — whose file edits happen in
        // the `forge mcp-serve` subprocess and only ever show up as a tree change. Proven here: a
        // bridge turn (tools surfaced via the sink) that left a modified existing test gets exactly
        // ONE guard re-drive, and the pristine test is restored.
        let dir = repo_with_modified_test();
        let provider = Arc::new(BridgeDescribeProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            edit_file: None,
            emit_tool: true,
        });
        let calls = Arc::clone(&provider);
        let router = Arc::new(FixedRouter {
            model: "codex-cli::gpt-5.5".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.workspace = WorkspaceContext::new(&dir).unwrap();
        session.set_expect_code_change(true);
        // Isolate the pristine-test guard from the independent completeness review.
        session.config.mesh.verify_completeness = false;
        let answer = session.run_turn("fix the bug").await.unwrap();
        assert_eq!(
            calls.calls.load(std::sync::atomic::Ordering::SeqCst),
            2 + CONTINUATION_DIMINISHING_MIN,
            "bridge completion, three bounded empty-diff continuations, then exactly ONE \
             pristine-test guard re-drive"
        );
        assert_eq!(answer, "still only describing after the nudge");
        assert_eq!(
            std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap(),
            "assert fix() == 1\n",
            "the test file must be back at its committed content"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn new_test_files_never_trip_the_guard() {
        // Adding a fresh reproduction test is normal practice — only MODIFIED existing tests are
        // the red flag.
        let dir = clean_git_repo();
        std::fs::create_dir_all(dir.join("tests")).unwrap();
        std::fs::write(dir.join("tests/test_new.py"), "assert repro()\n").unwrap();
        let provider = Arc::new(DescribeOnlyProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let calls = Arc::clone(&provider);
        let router = Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.workspace = WorkspaceContext::new(&dir).unwrap();
        session.set_expect_code_change(true);
        let answer = session.run_turn("fix the bug").await.unwrap();
        assert_eq!(
            calls.calls.load(std::sync::atomic::Ordering::SeqCst),
            2 + CONTINUATION_DIMINISHING_MIN,
            "an untracked new test file must not fire the pristine-test guard; only the bounded \
             empty-diff continuations run"
        );
        assert_eq!(answer, "still only describing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_edit_guard_respects_the_config_gate() {
        // Disabling the test-edit guard leaves the model's rewritten test in place. The separate
        // empty-diff completion guard still performs its bounded recovery continuations because
        // no production artifact changed; this gate must not silently disable that safety net.
        let dir = repo_with_modified_test();
        let provider = Arc::new(DescribeOnlyProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let calls = Arc::clone(&provider);
        let router = Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.workspace = WorkspaceContext::new(&dir).unwrap();
        session.set_expect_code_change(true);
        session.config.mesh.guard_test_edits = false;
        let answer = session.run_turn("fix the bug").await.unwrap();
        assert_eq!(
            calls.calls.load(std::sync::atomic::Ordering::SeqCst),
            2 + CONTINUATION_DIMINISHING_MIN
        );
        assert_eq!(answer, "still only describing");
        // The rewritten test is left exactly as the model wrote it.
        assert_eq!(
            std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap(),
            "assert fix() == 2\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Timeout reconciliation window (quality guards wave 4, fix 2) ---

    #[test]
    fn reconcile_deadline_budget_math() {
        // bench swe's shape: a 900s hard timeout leaves a 780s soft budget (120s reserve).
        assert_eq!(reconcile_deadline_budget_secs(900, 120), Some(780));
        assert_eq!(reconcile_deadline_budget_secs(300, 120), Some(180));
        // A timeout at or under the reserve leaves no usable budget → no deadline is set (the
        // hard kill is then the only bound, exactly the pre-wave-4 behaviour).
        assert_eq!(reconcile_deadline_budget_secs(120, 120), None);
        assert_eq!(reconcile_deadline_budget_secs(60, 120), None);
        assert_eq!(reconcile_deadline_budget_secs(0, 120), None);
    }

    /// Scripted runaway model: ALWAYS returns a read-only tool call, never finishing on its own —
    /// only an external bound (step cap or the deadline) can end the loop. Counts completions.
    struct CountingToolLoopProvider {
        calls: std::sync::atomic::AtomicUsize,
    }
    #[async_trait::async_trait]
    impl Provider for CountingToolLoopProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(forge_provider::ModelResponse {
                content: String::new(),
                tool_calls: vec![forge_types::ToolCall {
                    id: forge_types::new_id(),
                    name: "list_dir".into(),
                    args: serde_json::json!({ "path": "." }),
                }],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn past_deadline_allows_exactly_one_reconcile_completion() {
        // With the deadline already past, the loop must inject the revert instruction, allow ONE
        // model completion to act on it, then end — not run to the 100-step cap.
        let provider = Arc::new(CountingToolLoopProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let calls = Arc::clone(&provider);
        let router = Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        });
        let (store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.set_turn_deadline(std::time::Instant::now() - std::time::Duration::from_secs(1));
        session
            .run_turn("How would you fix the bug?")
            .await
            .unwrap();
        assert_eq!(
            calls.calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "one reconciliation completion, then the loop must end"
        );
        // The revert instruction was actually delivered to the transcript.
        let msgs = store.load_messages(session.id()).unwrap();
        assert!(
            msgs.iter().any(|m| m.content == DEADLINE_RECONCILE_NUDGE),
            "the reconcile instruction must be in the transcript"
        );
    }

    #[tokio::test]
    async fn deadline_reconcile_respects_the_config_gate() {
        // `mesh.deadline_reconcile = false` restores the old behaviour: the deadline is ignored
        // and only the step cap bounds the loop.
        let provider = Arc::new(CountingToolLoopProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let calls = Arc::clone(&provider);
        let router = Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.config.mesh.deadline_reconcile = false;
        session.config.mesh.max_steps = 3;
        session.set_turn_deadline(std::time::Instant::now() - std::time::Duration::from_secs(1));
        session
            .run_turn("How would you fix the bug?")
            .await
            .unwrap();
        assert_eq!(
            calls.calls.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "gate off → the step cap, not the deadline, bounds the loop"
        );
    }

    #[tokio::test]
    async fn no_deadline_means_no_reconcile_behaviour() {
        // An interactive session (no deadline set) is byte-for-byte unaffected.
        let provider = Arc::new(CountingToolLoopProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let calls = Arc::clone(&provider);
        let router = Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.config.mesh.max_steps = 3;
        session
            .run_turn("How would you fix the bug?")
            .await
            .unwrap();
        assert_eq!(calls.calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    // --- Minimal-diff bias (quality guards wave 4, fix 3) ---

    #[test]
    fn base_prompt_requires_evidence_without_redundant_verification() {
        assert!(FORGE_SYSTEM.contains("one final complete"));
        assert!(FORGE_SYSTEM.contains("Reuse still-current successful evidence"));
        assert!(FORGE_SYSTEM.contains("do not rerun an unchanged check"));
        assert!(FORGE_SYSTEM.contains("Fix failures before"));
        assert!(FORGE_SYSTEM.contains("Plan bookkeeping is non-blocking"));
        assert!(FORGE_SYSTEM.contains("NEVER call update_tasks by itself"));
        assert!(update_tasks_spec()
            .description
            .contains("NEVER call this tool by itself"));
    }

    #[test]
    fn minimal_diff_bias_stays_small() {
        // One short paragraph, not another token-tripling preamble: the always-on completeness
        // clause tripled tokens; this bias must stay a few sentences. Wave 5 added one out-of-tree
        // verification clause, moving the ceiling 400 → 520 bytes; it must not grow past that.
        assert!(
            MINIMAL_DIFF_BIAS.len() <= 520,
            "MINIMAL_DIFF_BIAS must stay ≤520 bytes, is {}",
            MINIMAL_DIFF_BIAS.len()
        );
    }

    #[test]
    fn minimal_diff_bias_permits_out_of_tree_verification() {
        // The wave 5 clause must keep the minimal-final-diff discipline while explicitly allowing
        // throwaway scaffolding, so the astropy build-archaeology regression isn't re-locked in.
        assert!(
            MINIMAL_DIFF_BIAS.contains("keep the diff minimal"),
            "must retain the minimal-diff discipline"
        );
        assert!(
            MINIMAL_DIFF_BIAS.contains("/tmp")
                && MINIMAL_DIFF_BIAS.contains("FINAL committed diff"),
            "must permit /tmp scaffolding gated on a minimal FINAL diff"
        );
    }

    #[test]
    fn minimal_diff_bias_rides_only_code_change_turns() {
        let router = Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(Arc::new(PanicProvider), router);
        let plain = session.system_preamble();
        assert!(
            plain.iter().all(|m| m.content != MINIMAL_DIFF_BIAS),
            "interactive turns must NOT carry the bias"
        );
        session.set_expect_code_change(true);
        let bench = session.system_preamble();
        assert!(
            bench.iter().any(|m| m.content == MINIMAL_DIFF_BIAS),
            "code-change turns must carry the bias as system context"
        );
    }

    // --- Env-fight spend cap (quality guards wave 4, fix 4) ---

    #[test]
    fn env_setup_command_heuristic() {
        for c in [
            "pip install numpy==1.16",
            "pip3 install -e .",
            "python -m pip install -r requirements.txt",
            "python3 -m ensurepip --upgrade",
            "python -m venv .venv",
            "cd /repo && virtualenv env27",
            "uv venv --python 3.7",
            "uv pip install pytest",
            "sudo apt-get install -y python3-dev",
            "apt install python2",
            "conda create -n old python=2.7",
            // Build archaeology (wave 5, fix 2) — the astropy-12907 C-extension churn.
            "python setup.py build_ext --inplace",
            "cd astropy && python setup.py build_ext -i",
            "make -j4",
            "cd build && make",
            "gcc -c _np_utils.c -o _np_utils.o",
            "g++ -shared foo.o -o foo.so",
            "cc -fPIC -c wcslib.c",
            "cmake -DCMAKE_BUILD_TYPE=Release ..",
            "pyenv install 3.7.9",
            "./configure --prefix=/usr/local",
            "ninja -C build",
        ] {
            assert!(is_env_setup_command(c), "{c} must count as env setup");
        }
        for c in [
            "pytest tests/test_concat.py",
            "python -m pytest -x",
            "git status",
            "cargo build",
            "cat requirements.txt",
            // Whole-token matching must NOT let these false-positive off `make`/`cc`.
            "python manage.py makemigrations",
            "cat accumulator.py",
            "grep -rn cc_email .",
        ] {
            assert!(!is_env_setup_command(c), "{c} must NOT count as env setup");
        }
    }

    #[test]
    fn bridge_tool_command_extracts_from_json_or_raw() {
        // claude Bash / Forge shell over MCP: JSON with a command field.
        assert_eq!(
            bridge_tool_command(r#"{"command":"python setup.py build_ext","timeout":60}"#),
            "python setup.py build_ext"
        );
        // codex command_execution: the raw command string.
        assert_eq!(bridge_tool_command("make -j4"), "make -j4");
        // The extracted command feeds the same build-fight heuristic.
        assert!(is_env_setup_command(&bridge_tool_command(
            r#"{"command":"cd astropy && python setup.py build_ext -i"}"#
        )));
    }

    // --- Bridge token ceiling (wave 5, fix 1) ---

    #[test]
    fn bridge_turn_ceiling_trips_at_or_past_the_cap() {
        let cap = 2_500_000u64;
        assert!(!bridge_turn_over_budget(0, cap));
        assert!(!bridge_turn_over_budget(cap - 1, cap));
        assert!(
            bridge_turn_over_budget(cap, cap),
            "exactly at the cap trips"
        );
        assert!(bridge_turn_over_budget(cap + 1, cap));
        // The astropy tail (6.46M input) trips comfortably; n=1 and stochastic, a backstop only.
        assert!(bridge_turn_over_budget(6_460_000, cap));
    }

    #[test]
    fn bridge_turn_ceiling_disabled_by_zero_cap() {
        assert!(
            !bridge_turn_over_budget(u64::MAX, 0),
            "0 disables the ceiling"
        );
    }

    #[test]
    fn env_fight_tracker_fires_once_after_failure_and_recovery_attempt() {
        let mut t = EnvFightTracker::default();
        assert!(!t.observe(true));
        assert!(
            t.observe(false),
            "one failed setup plus one recovery attempt fires the spend cap"
        );
        assert!(t.should_block());
        assert!(!t.observe(true), "latched — never re-fires this turn");
        assert!(!t.observe(true));
    }

    #[test]
    fn env_fight_tracker_does_not_arm_on_successes_only() {
        let mut t = EnvFightTracker::default();
        for _ in 0..4 {
            assert!(!t.observe(false));
        }
        assert!(!t.should_block(), "successful setup is not an env fight");
    }

    /// Scripted env-fighter: four DISTINCT failing env-setup shell commands (distinct args so the
    /// identical-call doom-loop guard stays out of the way), then a final text sign-off. The first
    /// two execute; the cap blocks later setup commands.
    struct EnvFighterProvider {
        calls: std::sync::atomic::AtomicUsize,
    }
    #[async_trait::async_trait]
    impl Provider for EnvFighterProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < 4 {
                // A `-m venv` command (recognized by `is_env_setup_command`) invoking a binary that
                // does not exist, so it fails to start with a non-zero exit on EVERY platform
                // (`cmd /C` → "not recognized", `sh -c` → 127). The earlier form
                // `python3 -m venv /dev/null/venvN` only failed on Unix, where `/dev/null` is a
                // device file; on Windows `/dev/null/venvN` is an ordinary path that `venv` happily
                // creates → exit 0 → the failure streak never reached the threshold. Distinct target
                // per `n` keeps the identical-call doom-loop guard out of the way.
                return Ok(forge_provider::ModelResponse {
                    content: String::new(),
                    tool_calls: vec![forge_types::ToolCall {
                        id: forge_types::new_id(),
                        name: "shell".into(),
                        args: serde_json::json!({
                            "command": format!("forge-no-such-python -m venv target-venv{n}")
                        }),
                    }],
                    usage: forge_types::Usage::default(),
                    quotas: Vec::new(),
                });
            }
            on_event(StreamEvent::Text("stopping the provisioning fight".into()));
            Ok(forge_provider::ModelResponse {
                content: "stopping the provisioning fight".into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn env_setup_spend_cap_nudges_once_and_blocks_later_commands() {
        let provider = Arc::new(EnvFighterProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let router = Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        });
        let (store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        // Keep the count deterministic: no side-call diagnosis completions on shell failures.
        session.config.shell.explain_errors = false;
        session.mode = PermissionMode::Bypass;
        session.run_turn("fix the bug").await.unwrap();
        let msgs = store.load_messages(session.id()).unwrap();
        assert_eq!(
            msgs.iter().filter(|m| m.content == ENV_FIGHT_NUDGE).count(),
            1,
            "exactly one env-fight nudge after the recovery attempt"
        );
        assert!(
            msgs.iter()
                .any(|message| message.content == ENV_FIGHT_BLOCKED_RESULT),
            "later provisioning commands must be blocked without execution"
        );
    }

    #[tokio::test]
    async fn env_fight_nudge_respects_the_config_gate() {
        let provider = Arc::new(EnvFighterProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let router = Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        });
        let (store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        session.config.shell.explain_errors = false;
        session.config.mesh.env_fight_nudge = false;
        session.mode = PermissionMode::Bypass;
        session.run_turn("fix the bug").await.unwrap();
        let msgs = store.load_messages(session.id()).unwrap();
        assert!(
            msgs.iter()
                .all(|m| { m.content != ENV_FIGHT_NUDGE && m.content != ENV_FIGHT_BLOCKED_RESULT }),
            "gate off → no nudge or block"
        );
    }

    #[tokio::test]
    async fn busy_pinned_model_dispatches_without_a_reservation() {
        // Pins are governed by their normal outage/failover policy, never rejected solely because
        // an auto-routed turn holds the model reservation.
        let provider = Arc::new(EchoProvider {
            bad: std::collections::HashSet::new(),
            err: unavailable,
        });
        let router = Arc::new(FixedRouter {
            model: "pin::model".into(),
            fallbacks: vec!["fallback::model".into()],
        });
        let (store, mut session) = fixed_session(provider, router);
        session.pin_model(Some("pin::model".into()));
        let _reservation = store.try_reserve_model("pin::model").unwrap();

        assert_eq!(session.run_turn("hi").await.unwrap(), "pin::model");
    }

    #[tokio::test]
    async fn occupied_model_fails_over_without_benching_it() {
        // A concurrent completion owns the primary reservation. This turn must use an eligible
        // fallback without treating the busy primary as a provider-health failure.
        let provider = Arc::new(EchoProvider {
            bad: std::collections::HashSet::new(),
            err: unavailable,
        });
        let router = Arc::new(FixedRouter {
            model: "busy::model".into(),
            fallbacks: vec!["good::model".into()],
        });
        let (store, mut session) = fixed_session(provider, router);
        let _reservation = store.try_reserve_model("busy::model").unwrap();

        assert_eq!(session.run_turn("hi").await.unwrap(), "good::model");
        assert!(
            !store.current_benched().unwrap().is_benched("busy::model"),
            "a live completion is not a provider failure"
        );
    }

    #[tokio::test]
    async fn failover_skips_disabled_candidates_in_a_stale_fallback_chain() {
        let provider = Arc::new(EchoProvider {
            bad: ["bad::model".to_string()].into_iter().collect(),
            err: unavailable,
        });
        let router = Arc::new(FixedRouter {
            model: "bad::model".into(),
            fallbacks: vec!["disabled::model".into(), "good::model".into()],
        });
        let (store, mut session) = fixed_session(provider, router);
        session.config.mesh.disabled = vec!["disabled".into()];

        assert_eq!(session.run_turn("hi").await.unwrap(), "good::model");
        assert!(
            !store
                .current_benched()
                .unwrap()
                .is_benched("disabled::model"),
            "a disabled stale fallback must not be treated as a provider failure"
        );
    }

    #[tokio::test]
    async fn unavailable_model_is_benched_and_fails_over_without_retrying() {
        // A provider outage is shared health information, not a reason for every session to wait
        // through local retries. The next eligible model must serve this turn immediately.
        let provider = Arc::new(EchoProvider {
            bad: ["bad::model".to_string()].into_iter().collect(),
            err: unavailable,
        });
        let router = Arc::new(FixedRouter {
            model: "bad::model".into(),
            fallbacks: vec!["good::model".into()],
        });
        let (store, mut session) = fixed_session(provider, router);
        let answer = session.run_turn("hi").await.unwrap();
        assert_eq!(answer, "good::model");
        assert_eq!(
            session
                .route_affinity
                .as_ref()
                .map(|affinity| affinity.model.as_str()),
            Some("good::model"),
            "affinity must follow the model that actually completed after failover"
        );
        assert!(store.current_benched().unwrap().is_benched("bad::model"));
    }

    struct CodexRequestFailedOnce {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Provider for CodexRequestFailedOnce {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                return Err(forge_provider::ProviderError::Unavailable(
                    "provider request failed".into(),
                ));
            }
            Ok(forge_provider::ModelResponse {
                content: "recovered automatically".into(),
                tool_calls: Vec::new(),
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn codex_oauth_provider_request_failed_retries_the_same_continuation() {
        let provider = Arc::new(CodexRequestFailedOnce {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let router = Arc::new(FixedRouter {
            model: "codex-oauth::gpt-5.5".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider.clone(), router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;

        let answer = session
            .run_turn("continue the implementation")
            .await
            .unwrap();

        assert_eq!(answer, "recovered automatically");
        assert_eq!(
            provider.calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "one recoverable failure must retry once without user input"
        );
    }

    /// Mimics a CLI bridge: returns text with NO structured tool calls (a bridge's tools run in
    /// its own process; only its narration comes back here). Emits a `shell` ToolStarted on the
    /// first `inspect_calls` invocations — that's both the "made progress" signal the re-drive gate
    /// keys on AND the real-inspection signal the verification gate requires. 0 = never inspects
    /// (pure reasoning / a model that won't check); usize::MAX = inspects every turn; 1 = does real
    /// work once then stops inspecting (verification can't confirm).
    struct BridgeProvider {
        calls: std::sync::atomic::AtomicUsize,
        inspect_calls: usize,
    }
    #[async_trait::async_trait]
    impl Provider for BridgeProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < self.inspect_calls {
                on_event(StreamEvent::ToolStarted {
                    name: "shell".into(),
                    args: "git status".into(),
                });
                on_event(StreamEvent::ToolFinished {
                    name: "shell".into(),
                    ok: true,
                    summary: "working tree clean".into(),
                });
            }
            on_event(StreamEvent::Text("working".into()));
            Ok(forge_provider::ModelResponse {
                content: "working".into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    fn seed_tasks(store: &Store, id: &str, titles_done: &[(&str, bool)]) {
        let tasks: Vec<forge_types::TodoItem> = titles_done
            .iter()
            .map(|(t, done)| forge_types::TodoItem {
                title: (*t).to_string(),
                status: if *done {
                    forge_types::TodoStatus::Done
                } else {
                    forge_types::TodoStatus::Pending
                },
                assignee: None,
            })
            .collect();
        store.set_tasks(id, &tasks).unwrap();
    }

    /// Models a bridge that FALSELY reports done, then — when forced to verify — discovers the gap
    /// and reopens the task before genuinely finishing. Uses structured `update_tasks` calls so the
    /// real dispatch path drives task state (mirroring a bridge's MCP `update_tasks`).
    struct ReopenOnVerifyProvider {
        calls: std::sync::atomic::AtomicUsize,
    }
    #[async_trait::async_trait]
    impl Provider for ReopenOnVerifyProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_types::{new_id, ToolCall};
            let set = |status: &str| {
                vec![ToolCall {
                    id: new_id(),
                    name: "update_tasks".into(),
                    args: serde_json::json!({"tasks":[{"title":"ship","status":status}]}),
                }]
            };
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (content, tool_calls) = match n {
                0 => ("marking done", set("done")), // falsely claims done
                1 => ("all set", vec![]),           // narrates done -> triggers verification
                2 => ("oh, not actually done", set("in_progress")), // verify reopens the gap
                3 => ("finishing for real", set("done")), // genuinely completes
                _ => ("verified, done", vec![]),    // verification re-confirms -> terminal
            };
            on_event(StreamEvent::Text(content.into()));
            Ok(forge_provider::ModelResponse {
                content: content.into(),
                tool_calls,
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn verification_reopens_a_falsely_reported_done_task() {
        // The whole point of the gate: a model can CLAIM done while the work isn't. The forced
        // verification turn catches it, reopens the task, the re-drive finishes it, and a second
        // verification confirms. The turn must end with the task genuinely Done — and only after
        // more than the 2 invocations a truthful "done" would have taken.
        let provider = Arc::new(ReopenOnVerifyProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let (store, mut session) = bridge_session(provider.clone());
        let _ = session.run_turn("ship it").await.unwrap();
        let tasks = store.tasks(&session.id).unwrap();
        assert_eq!(tasks[0].status, forge_types::TodoStatus::Done);
        assert!(
            provider.calls.load(std::sync::atomic::Ordering::SeqCst) > 2,
            "verification must have reopened the false 'done' and re-driven to a real finish"
        );
    }

    fn bridge_session(provider: Arc<dyn Provider>) -> (Arc<Store>, Session) {
        let router = Arc::new(FixedRouter {
            model: "claude-cli::opus".into(),
            fallbacks: vec![],
        });
        let (store, mut session) = fixed_session(provider, router);
        // Isolate the model loop: the end-of-turn recap + auto-memory capture are separate provider
        // calls that would otherwise inflate the invocation count these tests assert on.
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        (store, session)
    }

    struct ToolOnlyBridgeThenFinal {
        calls: std::sync::atomic::AtomicUsize,
        session: std::sync::Mutex<Option<(Arc<Store>, String)>>,
    }

    #[async_trait::async_trait]
    impl Provider for ToolOnlyBridgeThenFinal {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            on_event(StreamEvent::ToolStarted {
                name: "shell".into(),
                args: "git status".into(),
            });
            on_event(StreamEvent::ToolFinished {
                name: "shell".into(),
                ok: true,
                summary: "clean".into(),
            });
            if n >= 3 {
                if let Some((store, id)) = self.session.lock().unwrap().as_ref() {
                    seed_tasks(store, id, &[("finish the implementation", true)]);
                }
            }
            Ok(forge_provider::ModelResponse {
                content: if n >= 3 { "finished and verified" } else { "" }.into(),
                tool_calls: Vec::new(),
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn bridge_tool_only_steps_continue_without_empty_reply_recovery() {
        let provider = Arc::new(ToolOnlyBridgeThenFinal {
            calls: std::sync::atomic::AtomicUsize::new(0),
            session: std::sync::Mutex::new(None),
        });
        let (store, mut session) = bridge_session(provider.clone());
        *provider.session.lock().unwrap() = Some((Arc::clone(&store), session.id.clone()));
        seed_tasks(&store, &session.id, &[("finish the implementation", false)]);
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        session.presenter = Box::new(capture);

        let answer = session.run_turn("finish it").await.unwrap();

        assert_eq!(answer, "finished and verified");
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 4);
        let events = events.lock().unwrap();
        assert!(events.iter().all(|event| {
            !matches!(event, PresenterEvent::Warning(message) | PresenterEvent::Error(message)
                if message.contains("empty response") || message.contains("last response was empty"))
        }));
        assert!(session.replay_items().iter().all(|item| {
            !matches!(item, forge_types::ReplayItem::Assistant(text) if text.trim().is_empty())
        }));
    }

    #[tokio::test]
    async fn bridge_with_unfinished_tasks_but_no_progress_halts_without_spiraling() {
        // The anti-spiral guarantee: a bridge that yields with a task still open but did NOTHING
        // this run (no tool, no task closed) must STOP, not be re-driven into a narration loop
        // (the old bridge-nudge bug). Exactly one invocation.
        let provider = Arc::new(BridgeProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            inspect_calls: 0,
        });
        let (store, mut session) = bridge_session(provider.clone());
        seed_tasks(&store, &session.id, &[("ship the release", false)]);
        let answer = session.run_turn("release it").await.unwrap();
        assert_eq!(answer, "working");
        assert_eq!(
            provider.calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "no-progress bridge must not be re-driven — it would spiral"
        );
    }

    #[tokio::test]
    async fn bridge_re_drives_while_making_progress_then_stops_at_the_cap() {
        // A bridge that keeps making progress (a tool runs each turn) but never closes the task is
        // re-driven — proving forge won't accept a half-done plan — but BOUNDED so it can't run
        // forever. 1 initial turn + MAX_BRIDGE_CONTINUE_NUDGES (8) re-drives = 9 invocations.
        let provider = Arc::new(BridgeProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            inspect_calls: usize::MAX, // a tool runs every turn = progress every turn
        });
        let (store, mut session) = bridge_session(provider.clone());
        seed_tasks(&store, &session.id, &[("ship the release", false)]);
        // This test counts task-continuation re-drives, not completeness review calls.
        session.config.mesh.verify_completeness = false;
        let _ = session.run_turn("release it").await.unwrap();
        assert_eq!(
            provider.calls.load(std::sync::atomic::Ordering::SeqCst),
            9,
            "must re-drive on progress but stop at the cap (1 + 8)"
        );
    }

    #[tokio::test]
    async fn bridge_completion_accepts_fresh_inspection_without_a_redundant_turn() {
        // "All tasks Done" must have fresh tool-grounded evidence. Here the bridge runs an
        // inspection tool (shell) in the same turn as its completion claim, so the claim is already
        // genuinely verified and no redundant second invocation is needed.
        let provider = Arc::new(BridgeProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            inspect_calls: usize::MAX, // emits a `shell` ToolStarted each turn = a real inspection
        });
        let (store, mut session) = bridge_session(provider.clone());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        session.presenter = Box::new(capture);
        seed_tasks(&store, &session.id, &[("ship the release", true)]);
        // Isolate completion verification from the separate completeness re-drive.
        session.config.mesh.verify_completeness = false;
        let answer = session.run_turn("release it").await.unwrap();
        assert_eq!(answer, "working");
        assert_eq!(
            provider.calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "fresh evidence in the completion turn must be accepted immediately"
        );
        assert_eq!(
            events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| matches!(event, PresenterEvent::AssistantDone))
                .count(),
            1,
            "only the accepted answer may finalize on the UI"
        );
        assert_eq!(
            store
                .load_history_page(&session.id, None, 100)
                .unwrap()
                .iter()
                .filter(|row| row.role == Role::Assistant)
                .count(),
            1,
            "provisional completion text must stay out of user history"
        );
    }

    #[tokio::test]
    async fn bridge_reasoning_only_completion_accepted_without_overfiring() {
        // The over-fire fix: a pure reasoning/analysis plan does NO inspectable work (the answer is
        // the deliverable). Demanding a tool inspection would wrongly flag it. Forge runs ONE
        // verification pass, sees there's nothing external to check, and ACCEPTS with a calm note —
        // it does NOT loop to the cap or shout UNVERIFIED. `inspect_calls: 0` = never inspects.
        let provider = Arc::new(BridgeProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            inspect_calls: 0,
        });
        let (store, mut session) = bridge_session(provider.clone());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        session.presenter = Box::new(capture);
        seed_tasks(&store, &session.id, &[("analyze the tradeoffs", true)]);
        let answer = session.run_turn("think it through").await.unwrap();
        assert_eq!(answer, "working");
        assert_eq!(
            provider.calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "reasoning-only completion must accept after ONE verification pass, not over-fire to the cap"
        );
        let ev = events.lock().unwrap();
        let calm = ev.iter().any(
            |e| matches!(e, PresenterEvent::Warning(w) if w.contains("no external artifacts")),
        );
        let shouted = ev
            .iter()
            .any(|e| matches!(e, PresenterEvent::Warning(w) if w.contains("UNVERIFIED")));
        assert!(
            calm,
            "must note it couldn't tool-verify (no artifacts), calmly"
        );
        assert!(
            !shouted,
            "must NOT shout UNVERIFIED on a legitimate reasoning task"
        );
    }

    #[tokio::test]
    async fn bridge_completion_flagged_unverified_when_work_done_but_never_re_checked() {
        // The C8 hole, properly scoped: the turn DID mutate an artifact, then claimed done but
        // never inspected it. Forge forces the verification cap and ends LOUDLY flagging
        // UNVERIFIED — never a silent success. A plain `git status` inspection is deliberately not
        // used here: fresh inspection evidence should now be accepted without a redundant pass.
        struct MutatingBridgeWithoutVerification {
            calls: std::sync::atomic::AtomicUsize,
        }
        #[async_trait::async_trait]
        impl Provider for MutatingBridgeWithoutVerification {
            async fn complete(
                &self,
                _model: &str,
                _messages: &[Message],
                _tools: &[ToolSpec],
                on_event: &mut forge_provider::EventSink<'_>,
            ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
                let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n == 0 {
                    on_event(StreamEvent::ToolStarted {
                        name: "write_file".into(),
                        args: r#"{"path":"artifact.txt","content":"changed"}"#.into(),
                    });
                    on_event(StreamEvent::ToolFinished {
                        name: "write_file".into(),
                        ok: true,
                        summary: "wrote artifact.txt".into(),
                    });
                }
                on_event(StreamEvent::Text("working".into()));
                Ok(forge_provider::ModelResponse {
                    content: "working".into(),
                    tool_calls: vec![],
                    usage: forge_types::Usage::default(),
                    quotas: Vec::new(),
                })
            }
        }

        let provider = Arc::new(MutatingBridgeWithoutVerification {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let (store, mut session) = bridge_session(provider.clone());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        session.presenter = Box::new(capture);
        seed_tasks(&store, &session.id, &[("ship the release", true)]);
        // Isolate completion verification from the separate completeness re-drive.
        session.config.mesh.verify_completeness = false;
        let _ = session.run_turn("release it").await.unwrap();
        // 1 work/claim turn + MAX_VERIFY_ATTEMPTS (2) verification turns = 3 invocations.
        assert_eq!(
            provider.calls.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "must force the verification cap, not loop forever"
        );
        let unverified = events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, PresenterEvent::Warning(w) if w.contains("UNVERIFIED")));
        assert!(
            unverified,
            "work-producing completion never re-checked must end flagged UNVERIFIED, not as success"
        );
    }

    fn fixed_session(
        provider: Arc<dyn Provider>,
        router: Arc<dyn Router>,
    ) -> (Arc<Store>, Session) {
        let store = Arc::new(Store::open_in_memory().unwrap());
        // Disable the in-turn rate-limit WAIT by default so failover tests don't real-sleep on a
        // server `retry_after`; the wait path has its own test that re-enables it with a tiny reset.
        let mut config = Config::default();
        config.mesh.rate_limit_wait_secs = 0;
        let session = Session::start(
            Arc::clone(&store),
            provider,
            router,
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        (store, session)
    }

    /// Panics if asked to complete — proves a code path makes NO provider call.
    struct PanicProvider;
    #[async_trait::async_trait]
    impl Provider for PanicProvider {
        async fn complete(
            &self,
            model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            panic!("provider must NOT be called when no usable model exists (routed: {model})");
        }
    }

    #[test]
    fn last_resort_skips_a_keyless_provider_even_when_it_recovers_soonest() {
        // The "groq for everything" churn: groq (no key) gets benched and, recovering soonest,
        // becomes the last-resort pick — dispatched, no-auth "Resolver error", re-benched, forever.
        // last_resort must skip any provider with no key (ollama/bridges keep qualifying — keyless).
        // `minimax` has no key on any test box (mirrors the sibling no-usable-model test); the dev
        // machine may well have a real GROQ_API_KEY, so use minimax as the keyless stand-in.
        assert!(
            !forge_config::has_api_key("minimax"),
            "test precondition: no minimax key in the environment"
        );
        assert!(
            forge_config::has_api_key("ollama"),
            "ollama is keyless → always usable"
        );
        let (store, session) = fixed_session(
            Arc::new(PanicProvider),
            Arc::new(FixedRouter {
                model: "m".into(),
                fallbacks: vec![],
            }),
        );
        // minimax recovers SOONER (10s) than ollama (60s) → soonest_unbenched would return minimax.
        store
            .bench_for(
                "minimax::abab-test",
                std::time::Duration::from_secs(10),
                "rate-limited",
            )
            .unwrap();
        store
            .bench_for(
                "ollama::llama3.2",
                std::time::Duration::from_secs(60),
                "rate-limited",
            )
            .unwrap();
        assert_eq!(
            session.last_resort_model("other::x", false).as_deref(),
            Some("ollama::llama3.2"),
            "last-resort must skip keyless groq and pick the usable ollama model"
        );
    }

    #[tokio::test]
    async fn no_usable_model_stops_the_turn_instead_of_spinning_on_a_keyless_provider() {
        // The "keeps trying groq for everything" bug: when nothing is usable the router falls back
        // to a key-needing model anyway. The core must STOP with an actionable diagnostic, not call
        // it (and auth-fail) every turn. `minimax` has no key here, so routing to it must short
        // out before the provider is ever touched — PanicProvider would fire if it were called.
        assert!(
            !forge_config::has_api_key("minimax"),
            "test precondition: no minimax key in the environment"
        );
        let (_store, mut session) = fixed_session(
            Arc::new(PanicProvider),
            Arc::new(FixedRouter {
                model: "minimax::abab-test".into(),
                fallbacks: vec![],
            }),
        );
        let answer = session.run_turn("write hello world").await.unwrap();
        assert!(
            answer.contains("No usable model") && answer.contains("minimax"),
            "actionable no-usable-model stop expected, got: {answer}"
        );
    }

    #[test]
    fn replay_items_reconstructs_text_and_tool_activity() {
        use forge_types::ReplayItem;
        let (_store, mut session) = fixed_session(
            Arc::new(FlakyProvider {
                bad: std::collections::HashSet::new(),
                err: rate_limited,
            }),
            Arc::new(FixedRouter {
                model: "m".into(),
                fallbacks: vec![],
            }),
        );
        // A compaction marker, a user turn, a tool-only assistant turn + its result, a final answer.
        session.transcript = vec![
            Message::system("[Earlier conversation summarized to save context]\ndid X then Y"),
            Message::user("do the thing"),
            Message::assistant_tool_calls(
                "",
                vec![forge_types::ToolCall {
                    id: "c1".into(),
                    name: "read_file".into(),
                    args: serde_json::json!({"path": "a.rs"}),
                }],
            ),
            Message::tool_result("c1", "fn main() {}"),
            Message::assistant("done"),
        ];
        let items = session.replay_items();
        // The old history() dropped the summary, the tool-only turn, and the result; replay_items
        // keeps all of them so the resumed conversation is faithful.
        assert!(matches!(&items[0], ReplayItem::Note(s) if s.contains("summarized")));
        assert!(matches!(&items[1], ReplayItem::User(s) if s == "do the thing"));
        assert!(matches!(&items[2], ReplayItem::Tool { name, .. } if name == "read_file"));
        assert!(
            matches!(&items[3], ReplayItem::ToolResult { name, ok, .. } if name == "read_file" && *ok)
        );
        assert!(matches!(&items[4], ReplayItem::Assistant(s) if s == "done"));
        assert_eq!(items.len(), 5);
    }

    #[tokio::test]
    async fn run_turn_with_prepends_persisted_guidance_before_the_prompt() {
        // A skill/command's methodology is injected as a System message ahead of the user prompt
        // and persisted (so resume rehydrates it). The turn otherwise runs exactly as normal.
        let provider = Arc::new(FlakyProvider {
            bad: std::collections::HashSet::new(),
            err: rate_limited,
        });
        let router = Arc::new(FixedRouter {
            model: "good::model".into(),
            fallbacks: vec![],
        });
        let (store, mut session) = fixed_session(provider, router);
        let answer = session
            .run_turn_with(
                "do the thing",
                &["METHODOLOGY: be rigorous".to_string()],
                Some(TaskTier::Complex),
            )
            .await
            .unwrap();
        assert_eq!(answer, "recovered");

        let msgs = store.load_messages(session.id()).unwrap();
        assert_eq!(msgs[0].role, Role::System);
        assert!(msgs[0].content.contains("METHODOLOGY"));
        assert_eq!(msgs[1].role, Role::User);
        assert_eq!(msgs[1].content, "do the thing");
    }

    #[tokio::test]
    async fn dependent_turn_proactively_prunes_bulky_completed_tool_output() {
        let provider = Arc::new(FlakyProvider {
            bad: std::collections::HashSet::new(),
            err: rate_limited,
        });
        let router = Arc::new(FixedRouter {
            model: "good::model".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        session.config.recap.enabled = false;
        session.config.suggest.enabled = false;
        session.config.mesh.auto_memory = false;
        let full_result = "diagnostic output ".repeat(1_000);
        session.transcript = vec![
            Message::user("first turn"),
            Message::assistant_tool_calls(
                "",
                vec![forge_types::ToolCall {
                    id: "old-shell".into(),
                    name: "shell".into(),
                    args: serde_json::json!({"command": "tests"}),
                }],
            ),
            Message::tool_result("old-shell", full_result.clone()),
            Message::assistant("first turn complete"),
        ];
        for note in ["recap", "suggest", "memory", "cost", "status", "done"] {
            session.transcript.push(Message::system(note).ui_only());
        }

        session
            .run_turn("continue the implementation")
            .await
            .unwrap();

        let retained = session
            .transcript
            .iter()
            .find(|message| message.tool_call_id.as_deref() == Some("old-shell"))
            .expect("old tool result remains represented");
        assert!(retained.content.ends_with(PRUNE_MARKER));
        assert!(retained.content.len() < full_result.len() / 5);
    }

    #[tokio::test]
    async fn run_turn_recovers_when_its_empty_parent_session_was_pruned() {
        // A long-lived TUI driver can retain a freshly-created session id while another process
        // performs retention. The next turn must restore that parent before it persists command
        // guidance or the user message; otherwise SQLite reports a raw foreign-key failure.
        let provider = Arc::new(FlakyProvider {
            bad: std::collections::HashSet::new(),
            err: rate_limited,
        });
        let router = Arc::new(FixedRouter {
            model: "good::model".into(),
            fallbacks: vec![],
        });
        let (store, mut session) = fixed_session(provider, router);
        let session_id = session.id().to_string();

        assert_eq!(store.prune_empty(-1, 1).unwrap(), 1);
        assert!(!store.session_exists(&session_id).unwrap());

        assert_eq!(
            session
                .run_turn_with(
                    "do the thing",
                    &["METHODOLOGY: be rigorous".to_string()],
                    Some(TaskTier::Complex),
                )
                .await
                .unwrap()
                .text,
            "recovered"
        );
        assert!(store.session_exists(&session_id).unwrap());
    }

    #[tokio::test]
    async fn retryable_error_benches_the_model_and_fails_over() {
        // AC-1 + AC-2: the primary 429s → benched (with the server's 42s cooldown) → the turn
        // retries on the fallback and succeeds.
        let provider = Arc::new(FlakyProvider {
            bad: ["bad::model".to_string()].into_iter().collect(),
            err: rate_limited,
        });
        let router = Arc::new(FixedRouter {
            model: "bad::model".into(),
            fallbacks: vec!["good::model".into()],
        });
        let (store, mut session) = fixed_session(provider, router);
        let answer = session.run_turn("hi").await.unwrap();
        assert_eq!(answer, "recovered");
        // The bad model is benched; the cooldown reflects the server's 42s (not the default).
        let report = store.current_benched_report().unwrap();
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].0, "bad::model");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!(
            (report[0].1 - now - 42).abs() <= 2,
            "cooldown ~42s: {report:?}"
        );
    }

    #[tokio::test]
    async fn auth_error_benches_the_entire_provider_before_failover() {
        let provider = Arc::new(FlakyProvider {
            bad: ["agy-cli::gemini-3.1-pro".to_string()]
                .into_iter()
                .collect(),
            err: |_| forge_provider::ProviderError::Auth("login required".into()),
        });
        let router = Arc::new(FixedRouter {
            model: "agy-cli::gemini-3.1-pro".into(),
            fallbacks: vec!["good::model".into()],
        });
        let (store, mut session) = fixed_session(provider, router);
        assert_eq!(session.run_turn("hi").await.unwrap(), "recovered");
        let health = store.current_benched().unwrap();
        assert!(health.is_benched("agy-cli::gemini-3.1-pro"));
        assert!(health.is_benched("agy-cli::gemini-3.5-flash"));
        assert!(!health.is_benched("good::model"));
    }

    #[tokio::test]
    async fn pinned_auth_error_still_benches_the_entire_provider() {
        // A pin forbids changing models for this turn, but it must not prevent the durable
        // provider-wide auth exclusion that protects every subsequent mesh route.
        let provider = Arc::new(FlakyProvider {
            bad: ["agy-cli::gemini-3.1-pro".to_string()]
                .into_iter()
                .collect(),
            err: |_| forge_provider::ProviderError::Auth("login required".into()),
        });
        let router = Arc::new(PinnedRouter {
            model: "agy-cli::gemini-3.1-pro".into(),
            fallbacks: vec!["good::model".into()],
        });
        let (store, mut session) = fixed_session(provider, router);
        assert!(
            session.run_turn("hi").await.is_err(),
            "a strict pin must fail this turn"
        );
        let health = store.current_benched().unwrap();
        assert!(health.is_benched("agy-cli::gemini-3.1-pro"));
        assert!(health.is_benched("agy-cli::gemini-3.5-flash"));
        assert!(!health.is_benched("good::model"));
    }

    #[tokio::test]
    async fn non_retryable_error_does_not_fail_over_or_bench() {
        // AC-5: a 400-style error fails the turn as before; the model is NOT benched.
        let provider = Arc::new(FlakyProvider {
            bad: ["bad::model".to_string()].into_iter().collect(),
            err: |_| forge_provider::ProviderError::Request("bad request".into()),
        });
        let router = Arc::new(FixedRouter {
            model: "bad::model".into(),
            fallbacks: vec!["good::model".into()],
        });
        let (store, mut session) = fixed_session(provider, router);
        assert!(session.run_turn("hi").await.is_err());
        assert!(store.current_benched().unwrap().is_empty());
    }

    #[tokio::test]
    async fn exhausting_the_chain_returns_no_healthy_model() {
        // AC-6: primary 429s, no fallbacks → a clear error, not a hang.
        let provider = Arc::new(FlakyProvider {
            bad: ["bad::model".to_string()].into_iter().collect(),
            err: rate_limited,
        });
        let router = Arc::new(FixedRouter {
            model: "bad::model".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        assert!(matches!(
            session.run_turn("hi").await,
            Err(CoreError::NoHealthyModel { .. })
        ));
    }

    #[test]
    fn a_redrive_without_text_keeps_the_primary_answer() {
        // Regression: the empty-diff nudge, test-edit guard, autofix and stop-hook re-drives each
        // assigned their `final_text` unconditionally. A re-drive that ends on a loop-guard halt,
        // the empty-response dead-end, or the step cap returns "" — which erased the answer the
        // primary loop had already produced, so the turn reported an empty final answer for work
        // that was actually done.
        let mut answer = "here is what I changed".to_string();
        adopt_redrive_text(&mut answer, String::new());
        assert_eq!(
            answer, "here is what I changed",
            "empty re-drive must not erase"
        );
        adopt_redrive_text(&mut answer, "  \n\t ".to_string());
        assert_eq!(
            answer, "here is what I changed",
            "whitespace-only re-drive must not erase"
        );
        adopt_redrive_text(&mut answer, "and here is the fix".to_string());
        assert_eq!(
            answer, "and here is the fix",
            "a real re-drive answer still wins"
        );
    }

    #[tokio::test]
    async fn chain_exhaustion_reports_the_real_provider_error() {
        // The generic "every model is rate-limited or down" story is WRONG (and sends the user off
        // to wait out a nonexistent rate limit) when the real cause was an expired credential. The
        // provider's actionable message must survive into the terminal error.
        let provider = Arc::new(FlakyProvider {
            bad: ["bad::model".to_string()].into_iter().collect(),
            err: |_| {
                forge_provider::ProviderError::Auth(
                    "ChatGPT OAuth token rejected (401) — run `forge auth codex-oauth`".into(),
                )
            },
        });
        let router = Arc::new(FixedRouter {
            model: "bad::model".into(),
            fallbacks: vec![],
        });
        let (_store, mut session) = fixed_session(provider, router);
        let err = session.run_turn("hi").await.expect_err("chain exhausted");
        let shown = err.to_string();
        match err {
            CoreError::NoHealthyModel {
                model,
                reason,
                last_error,
            } => {
                assert_eq!(model, "bad::model");
                assert_eq!(reason, "auth failed");
                assert!(
                    last_error.contains("forge auth codex-oauth"),
                    "actionable provider message must survive: {last_error}"
                );
            }
            other => panic!("expected NoHealthyModel, got {other}"),
        }
        assert!(
            shown.contains("forge auth codex-oauth"),
            "…and be visible in the Display string: {shown}"
        );
    }

    // --- Conversation checkpoints + /undo (RFC session-management-and-commands, PR2) ---

    #[tokio::test]
    async fn undo_rewinds_the_last_user_turn() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = fresh_session(Arc::clone(&store), Config::default());
        let id = session.id().to_string();

        session
            .run_turn("check the project manifest")
            .await
            .unwrap();
        assert!(
            store.load_messages(&id).unwrap().len() >= 2,
            "the turn persisted messages"
        );

        // Undo drops the whole turn (the user prompt + its replies/tools).
        assert!(session.undo().unwrap().is_some(), "a turn was undone");
        assert!(
            store.load_messages(&id).unwrap().is_empty(),
            "rewound turn is excluded from the active transcript"
        );
        assert!(session.undo().unwrap().is_none(), "nothing left to undo");
    }

    #[tokio::test]
    async fn undo_after_compacted_resume_does_not_wipe_survivors() {
        // P0 data-loss regression: after compaction the active tail starts at a HIGH db seq, but a
        // resumed transcript is short. If `self.seq` were the loaded count (not MAX(seq)+1) and
        // `rewind_to` used the transcript index directly as the db seq, an `/undo` of the next turn
        // would `deactivate_messages_from(low_index)` and sweep the pre-compaction survivors.
        let store = Arc::new(Store::open_in_memory().unwrap());
        let sid = store.create_session("/tmp", "default").unwrap();
        for i in 0..16i64 {
            store
                .add_message(&sid, i, Role::User, &format!("msg {i}"), None)
                .unwrap();
        }
        // Keep the last 6 (seq 10-15) active; summarize seq 0-9.
        store
            .compact_session_store(&sid, "summary of the first ten", 6)
            .unwrap();
        // Sanity: summary + 6 survivors.
        assert_eq!(store.load_messages(&sid).unwrap().len(), 7);

        let mut session = Session::resume(
            Arc::clone(&store),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            Config::default(),
            &sid,
        )
        .unwrap();

        // A fresh turn after the compacted resume, then undo it.
        session.run_turn("a brand new prompt").await.unwrap();
        assert!(session.undo().unwrap().is_some(), "the new turn was undone");

        // The six pre-compaction survivors (seq 10-15) MUST still be active — undo only removed the
        // new turn. Pre-fix, load_messages would return just the summary (survivors wiped).
        let after = store.load_messages(&sid).unwrap();
        assert_eq!(
            after.len(),
            7,
            "summary + 6 survivors must remain after undo; got {} msgs",
            after.len()
        );
        assert!(
            after.iter().any(|m| m.content == "msg 15"),
            "survivor 'msg 15' must still be active"
        );
    }

    #[tokio::test]
    async fn checkpoint_rewind_by_db_seq_after_compaction_targets_the_right_turn() {
        // Regression: the /checkpoints picker passes a DB SEQ to rewind_to. After compaction the
        // transcript index and DB seq diverge; rewind_to must interpret its argument as a DB seq
        // (both undo and the picker pass seqs) — not a transcript index, which double-offset and
        // rewound to the wrong turn (or no-op).
        let store = Arc::new(Store::open_in_memory().unwrap());
        let sid = store.create_session("/tmp", "default").unwrap();
        for i in 0..16i64 {
            store
                .add_message(&sid, i, Role::User, &format!("msg {i}"), None)
                .unwrap();
        }
        store
            .compact_session_store(&sid, "summary of the first ten", 6)
            .unwrap();
        let mut session = Session::resume(
            Arc::clone(&store),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            Config::default(),
            &sid,
        )
        .unwrap();

        session.checkpoint(Some("before the turn")).unwrap();
        let cp_seq = session.checkpoints().unwrap()[0].seq; // a DB seq, as the picker passes
        session.run_turn("a brand new prompt").await.unwrap();

        // Picker-style rewind by DB seq must roll back exactly the new turn and keep the survivors.
        session.rewind_to(cp_seq).unwrap();
        let after = store.load_messages(&sid).unwrap();
        assert_eq!(
            after.len(),
            7,
            "summary + 6 survivors after rewinding the new turn by DB seq; got {}",
            after.len()
        );
    }

    #[tokio::test]
    async fn every_turn_auto_checkpoints_with_a_prompt_preview() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = fresh_session(Arc::clone(&store), Config::default());

        session
            .run_turn("check the project manifest")
            .await
            .unwrap();
        session.run_turn("now check it again please").await.unwrap();

        let cps = session.checkpoints().unwrap();
        assert_eq!(cps.len(), 2, "one auto checkpoint per turn");
        // Newest first, labeled with the prompt preview (so /undo can show the message).
        assert_eq!(cps[0].label.as_deref(), Some("now check it again please"));
        assert_eq!(cps[1].label.as_deref(), Some("check the project manifest"));
        // Each checkpoint's boundary is its turn's start, so rewinding there undoes that turn.
        assert!(cps[0].seq > cps[1].seq);
    }

    #[tokio::test]
    async fn checkpoint_then_turn_then_rewind_to_it() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = fresh_session(Arc::clone(&store), Config::default());
        let id = session.id().to_string();

        session
            .run_turn("check the project manifest")
            .await
            .unwrap();
        session.checkpoint(Some("after first turn")).unwrap();
        let boundary = session.checkpoints().unwrap()[0].seq;
        session.run_turn("check the manifest again").await.unwrap();
        let after_two = store.load_messages(&id).unwrap().len();

        session.rewind_to(boundary).unwrap();
        let after_rewind = store.load_messages(&id).unwrap().len();
        assert!(
            after_rewind < after_two && after_rewind == boundary as usize,
            "rewind drops the second turn back to the checkpoint boundary"
        );
    }

    /// A provider that writes a file once (via `write_file`), then answers.
    struct WritingProvider {
        path: String,
        content: String,
    }
    #[async_trait::async_trait]
    impl Provider for WritingProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_provider::ModelResponse;
            use forge_types::{new_id, ToolCall, Usage};
            let usage = Usage::default();
            if messages.iter().any(|m| m.role == Role::Tool) {
                return Ok(ModelResponse {
                    content: "done".into(),
                    tool_calls: vec![],
                    usage,
                    quotas: Vec::new(),
                });
            }
            Ok(ModelResponse {
                content: "writing".into(),
                tool_calls: vec![ToolCall {
                    id: new_id(),
                    name: "write_file".into(),
                    args: serde_json::json!({ "path": self.path, "content": self.content }),
                }],
                usage,
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn checkpoint_context_is_explicit_and_does_not_pollute_process_env() {
        // The bridge handoff was a process-global `set_var` (a `getenv` race / cross-session clobber
        // risk). It is now an EXPLICIT `CheckpointContext` threaded via `CompletionOptions` to the
        // spawned child's own env. Prove the parent builds it from session state and, crucially, that
        // running a turn no longer writes this session's context into the process-global env.
        let dir = std::env::temp_dir().join(format!("forge-cpctx-{}", forge_types::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.txt");
        std::fs::write(&file, "ORIGINAL").unwrap();

        let config = Config {
            permission_mode: PermissionMode::Default,
            ..Config::default()
        };
        let mut session = Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(WritingProvider {
                path: file.to_string_lossy().to_string(),
                content: "X".into(),
            }),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(&dir),
            Box::new(HeadlessPresenter::new(false)),
            config,
            dir.to_str().unwrap(),
        )
        .unwrap();
        assert_eq!(session.temper(), PermissionMode::Default);
        assert_eq!(session.mode(), PermissionMode::Default);
        session.set_temper(PermissionMode::Bypass);
        assert_eq!(session.temper(), PermissionMode::Bypass);
        assert_eq!(session.mode(), PermissionMode::Bypass);
        session.set_checkpoint_root(dir.join("snaps"));

        session.run_turn("edit it").await.unwrap();

        let ctx = session.checkpoint_context();
        assert_eq!(ctx.session, session.id);
        assert_eq!(ctx.seq, session.current_turn_seq);
        assert_eq!(ctx.mode, session.temper().key());
        assert!(
            std::path::Path::new(&ctx.root).is_absolute(),
            "checkpoint root is absolutized for the child"
        );

        // The race fix: this session's id must NOT have leaked into the process-global env.
        assert_ne!(
            std::env::var(snapshot::ENV_SESSION).ok().as_deref(),
            Some(session.id.as_str()),
            "the parent no longer mutates process-global checkpoint env"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn picker_rewind_to_an_earlier_turn_reverts_files() {
        // Mirrors the /undo picker path: two turns edit a file, then rewind to the FIRST turn's
        // checkpoint seq (as the picker does) — the file must return to its pre-turn-1 bytes.
        let dir = std::env::temp_dir().join(format!("forge-rew-{}", forge_types::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.txt");
        std::fs::write(&file, "ORIGINAL").unwrap();

        let config = Config {
            permission_mode: PermissionMode::Bypass,
            ..Config::default()
        };
        let mut session = Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(WritingProvider {
                path: file.to_string_lossy().to_string(),
                content: "MODEL-EDIT".into(),
            }),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(&dir),
            Box::new(HeadlessPresenter::new(false)),
            config,
            dir.to_str().unwrap(),
        )
        .unwrap();
        session.set_checkpoint_root(dir.join("snaps"));

        session.run_turn("turn one edits the file").await.unwrap();
        session.run_turn("turn two edits it again").await.unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "MODEL-EDIT");

        // Picker uses the checkpoint's seq; pick the OLDEST (first turn).
        let cps = session.checkpoints().unwrap();
        let first_turn_seq = cps.last().unwrap().seq;
        let report = session.rewind_to(first_turn_seq).unwrap().restore;

        assert!(
            !report.restored.is_empty(),
            "files were restored: {report:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "ORIGINAL",
            "rewinding to turn 1 reverts the file to its pre-turn-1 bytes"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn undo_restores_files_written_during_the_turn() {
        let dir = std::env::temp_dir().join(format!("forge-undo-{}", forge_types::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("edited.txt");
        std::fs::write(&file, "original bytes").unwrap();

        let config = Config {
            permission_mode: PermissionMode::Bypass, // allow the write without a prompt
            ..Config::default()
        };
        let mut session = Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(WritingProvider {
                path: file.to_string_lossy().to_string(),
                content: "the model overwrote this".into(),
            }),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(&dir),
            Box::new(HeadlessPresenter::new(false)),
            config,
            dir.to_str().unwrap(),
        )
        .unwrap();
        session.set_checkpoint_root(dir.join("snaps"));

        session.run_turn("rewrite the file").await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "the model overwrote this",
            "the turn wrote the file"
        );

        let report = session.undo().unwrap().unwrap().restore;
        assert!(
            report.restored.iter().any(|p| p.contains("edited.txt")),
            "the written file was restored: {report:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "original bytes",
            "undo restored the pre-turn bytes"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn snapshot_failure_warns_that_undo_wont_cover_the_write() {
        // When the pre-write snapshot can't be written, the write still proceeds — but the user must
        // be warned that /undo can't restore it, instead of silently losing the safety net.
        let dir = std::env::temp_dir().join(format!("forge-snapfail-{}", forge_types::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("edited.txt");
        std::fs::write(&file, "original").unwrap();
        // A regular file standing where the checkpoint root's parent dir would be, so the snapshot's
        // `create_dir_all` fails (you can't create a directory underneath a file).
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, "i am a file, not a dir").unwrap();

        let events = Arc::new(Mutex::new(Vec::new()));
        let config = Config {
            permission_mode: PermissionMode::Bypass,
            ..Config::default()
        };
        let mut session = Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(WritingProvider {
                path: file.to_string_lossy().to_string(),
                content: "model wrote this".into(),
            }),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(&dir),
            Box::new(CapturePresenter {
                events: events.clone(),
            }),
            config,
            dir.to_str().unwrap(),
        )
        .unwrap();
        session.set_checkpoint_root(blocker.join("snaps"));

        session.run_turn("rewrite the file").await.unwrap();

        // The write still landed…
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "model wrote this");
        // …and a Warning told the user /undo won't cover it.
        let warned = events.lock().unwrap().iter().any(|e| {
            matches!(e, PresenterEvent::Warning(w) if w.contains("undo") && w.contains("snapshot"))
        });
        assert!(warned, "expected an /undo snapshot-failure warning");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A provider that blocks for a long time, so a turn can be interrupted mid-flight.
    struct SlowProvider;
    #[async_trait::async_trait]
    impl Provider for SlowProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            Ok(forge_provider::ModelResponse {
                content: "too late".into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn aborting_a_running_turn_releases_the_session_lock() {
        // The interrupt feature aborts the turn task; this proves the invariant it relies on —
        // cancelling a task that holds the session Mutex across an await frees the lock, so the
        // session stays usable (no deadlock / frozen UI).
        use std::time::Duration;
        let store = Arc::new(Store::open_in_memory().unwrap());
        // Disable auto-memory: its start-of-turn recall can invoke the embedder (a network call on
        // CI) before the user message is persisted, which would race the 100ms abort window below.
        // This test is about lock release, not memory.
        let mut config = Config::default();
        config.mesh.auto_memory = false;
        let session = Arc::new(tokio::sync::Mutex::new(
            Session::start(
                store,
                Arc::new(SlowProvider),
                Arc::new(HeuristicRouter::new(Config::default())),
                ToolRegistry::with_core_tools_in(test_workspace()),
                Box::new(HeadlessPresenter::new(false)),
                config,
                test_workspace().to_str().expect("workspace path is UTF-8"),
            )
            .unwrap(),
        ));

        let s = session.clone();
        let handle = tokio::spawn(async move {
            let mut g = s.lock().await;
            let _ = g.run_turn("a slow request").await;
        });
        // Let the task acquire the lock and enter the 30s provider sleep, then interrupt it.
        tokio::time::sleep(Duration::from_millis(100)).await;
        handle.abort();
        let _ = handle.await;

        // The lock must be free immediately (the aborted task dropped its guard).
        let guard = tokio::time::timeout(Duration::from_secs(2), session.lock())
            .await
            .expect("abort released the session lock");
        assert!(
            guard
                .history()
                .iter()
                .any(|(r, c)| matches!(r, Role::User) && c == "a slow request"),
            "the interrupted turn's prompt was recorded before the abort"
        );
    }

    // --- Assay mode (docs/features/analysis-mode.md) ---

    /// A provider that plays the critic + verifier roles for an in-session assay run.
    struct AssayProvider;
    #[async_trait::async_trait]
    impl Provider for AssayProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_provider::ModelResponse;
            let sys = messages
                .iter()
                .find(|m| m.role == Role::System)
                .map(|m| m.content.as_str())
                .unwrap_or("");
            let content = if sys.contains("ASSAY-VERIFIER") {
                r#"{"verdict":"uphold","confidence":"high"}"#.to_string()
            } else if sys.contains("ASSAY-CRITIC") && sys.contains("'correctness'") {
                r#"[{"severity":"high","file":"a.rs","line":1,"title":"bug","why":"w","fix":"f","effort":"small"}]"#.to_string()
            } else {
                "[]".to_string()
            };
            Ok(ModelResponse {
                content,
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn assay_analysis_emits_a_report_and_persists_the_run() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(AssayProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(capture),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        session
            .assay(
                Arc::from("fn main() {}"),
                assay::TierModels {
                    trivial: vec!["m".into()],
                    complex: vec!["m".into()],
                },
                vec![], // default: full crew
                forge_types::AssayScope::Repo,
                false, // analysis-only
            )
            .await
            .unwrap();

        let ev = events.lock().unwrap();
        let report = ev.iter().find_map(|e| match e {
            PresenterEvent::AssayReport(r) => Some(r.clone()),
            _ => None,
        });
        let report = report.expect("an AssayReport was emitted");
        assert_eq!(report.findings.len(), 1, "the upheld finding is reported");
        assert!(!report.run_id.is_empty(), "the run was persisted");
        assert_eq!(store.list_assay_runs().unwrap().len(), 1);
        assert_eq!(store.load_findings(&report.run_id).unwrap().len(), 1);
    }

    // --- In-TUI session swap (RFC session-management-and-commands, PR1) ---

    #[tokio::test]
    async fn reset_resumed_and_fresh_swap_the_live_session() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        // Seed a past session A with a user+assistant exchange.
        let a = store.create_session(".", "default").unwrap();
        store.add_message(&a, 0, Role::User, "hello", None).unwrap();
        store
            .add_message(&a, 1, Role::Assistant, "hi there", Some("m"))
            .unwrap();
        // A live session B (what the TUI is holding).
        let mut b = Session::start(
            Arc::clone(&store),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(HeadlessPresenter::new(false)),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        let b_id = b.id().to_string();

        // /resume A: B becomes A, rehydrating A's transcript.
        b.reset_resumed(&a).unwrap();
        assert_eq!(b.id(), a);
        assert_ne!(b.id(), b_id);
        assert_eq!(
            b.history(),
            vec![
                (Role::User, "hello".to_string()),
                (Role::Assistant, "hi there".to_string()),
            ]
        );

        // /new: a fresh empty session, new id.
        b.reset_fresh(".").unwrap();
        assert!(b.history().is_empty());
        assert_ne!(b.id(), a);
    }

    // ── Autofix tests ──────────────────────────────────────────────────────────────────────

    #[test]
    fn autofix_detection_does_not_enable_default_off_commands() {
        let mut config = forge_config::AutofixConfig::default();
        let detected = Session::fill_detected_autofix_commands(
            &mut config,
            "false".to_string(),
            Some("false".to_string()),
        );
        assert!(detected.is_empty());
        assert!(!config.auto_lint);
        assert!(!config.auto_test);
        assert!(config.lint_cmd.is_empty());
        assert!(config.test_cmd.is_empty());
    }

    #[test]
    fn autofix_detection_fills_only_explicitly_enabled_commands() {
        let mut config = forge_config::AutofixConfig {
            auto_lint: false,
            auto_test: true,
            ..forge_config::AutofixConfig::default()
        };
        let detected = Session::fill_detected_autofix_commands(
            &mut config,
            "npm run lint 2>&1".to_string(),
            Some("npm test 2>&1".to_string()),
        );
        assert_eq!(detected, vec!["npm test 2>&1"]);
        assert!(config.lint_cmd.is_empty());
        assert_eq!(config.test_cmd, "npm test 2>&1");
    }

    #[test]
    fn npm_autofix_detection_uses_only_scripts_that_exist() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"build":"tsc -p tsconfig.json","test":"node --test"}}"#,
        )
        .unwrap();

        let (lint, test) = Session::detect_project_commands(dir.path())
            .unwrap()
            .unwrap();
        assert!(
            lint.is_empty(),
            "missing lint/typecheck/check must stay empty"
        );
        assert_eq!(test.as_deref(), Some("npm test 2>&1"));
    }

    #[test]
    fn npm_autofix_detection_reports_malformed_package_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "{\"scripts\":").unwrap();

        let error = Session::detect_project_commands(dir.path())
            .expect_err("malformed package metadata must not look like no project");
        assert!(error.contains("cannot parse"));
        assert!(error.contains("package.json"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn autofix_stage_passes_when_commands_exit_zero() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(CapturePresenter::default()),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        let af = forge_config::AutofixConfig {
            auto_lint: true,
            auto_test: true,
            lint_cmd: "true".to_string(), // always exits 0
            test_cmd: "true".to_string(), // always exits 0
            max_iterations: 3,
            auto_detect: false, // explicit cmds set; no detection needed
        };
        // run_autofix_stage returns Ok(true) when all enabled commands pass.
        let passed = session.run_autofix_stage(&af).await.unwrap();
        assert!(passed, "both 'true' commands exit 0 → stage should pass");
        // No synthetic failure message pushed to transcript.
        assert!(
            session
                .transcript
                .iter()
                .all(|m| !m.content.contains("Auto-fix:")),
            "no failure message injected on pass"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn autofix_stage_fails_when_lint_exits_nonzero() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(CapturePresenter::default()),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        let af = forge_config::AutofixConfig {
            auto_lint: true,
            auto_test: false,              // test disabled
            lint_cmd: "false".to_string(), // always exits 1
            test_cmd: String::new(),
            max_iterations: 3,
            auto_detect: false,
        };
        let passed = session.run_autofix_stage(&af).await.unwrap();
        assert!(!passed, "'false' exits 1 → stage should fail");
        // A synthetic user message with the failure should be in the transcript.
        assert!(
            session
                .transcript
                .iter()
                .any(|m| m.content.contains("Auto-fix:") && m.content.contains("lint:")),
            "failure message injected into transcript: {:?}",
            session
                .transcript
                .iter()
                .map(|m| &m.content)
                .collect::<Vec<_>>()
        );
    }

    /// Call 0 writes a file (an edit → `edits_this_turn > 0`, arming autofix); every later call just
    /// says "done" (no tools), so the only thing that can stop the self-heal loop is its iteration cap.
    /// `cfg(unix)` because the only test using it relies on the `false` shell command.
    #[cfg(unix)]
    struct EditOnceThenDoneProvider {
        calls: std::sync::atomic::AtomicUsize,
        path: String,
    }
    #[cfg(unix)]
    #[async_trait::async_trait]
    impl Provider for EditOnceThenDoneProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_types::{new_id, ToolCall, Usage};
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let tool_calls = if n == 0 {
                vec![ToolCall {
                    id: new_id(),
                    name: "write_file".into(),
                    args: serde_json::json!({"path": self.path, "content": "x = 1\n"}),
                }]
            } else {
                Vec::new()
            };
            Ok(forge_provider::ModelResponse {
                content: "done".into(),
                tool_calls,
                usage: Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    /// A direct turn writes once, then its completeness pass performs a valid search that returns
    /// no matches. The outer policy must detect that tool evidence and issue exactly one fallback
    /// search prompt; all other provider calls finish without tools.
    #[cfg(unix)]
    struct EditThenEmptySearchProvider {
        calls: std::sync::atomic::AtomicUsize,
        path: String,
        audit_path: String,
        sibling_path: String,
        search_root: String,
    }
    #[cfg(unix)]
    #[async_trait::async_trait]
    impl Provider for EditThenEmptySearchProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use forge_types::{new_id, ToolCall, Usage};
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let tool_calls = match n {
                0 => vec![ToolCall {
                    id: new_id(),
                    name: "write_file".into(),
                    args: serde_json::json!({"path": self.path, "content": "x = 1\n"}),
                }],
                2 => vec![
                    ToolCall {
                        id: new_id(),
                        name: "write_file".into(),
                        args: serde_json::json!({
                            "path": self.audit_path,
                            "content": "audit changed something\n",
                        }),
                    },
                    ToolCall {
                        id: new_id(),
                        name: "search".into(),
                        args: serde_json::json!({
                            "path": self.search_root,
                            "query": "__forge_missing_old_identifier__",
                            "regex": false,
                        }),
                    },
                ],
                4 => vec![ToolCall {
                    id: new_id(),
                    name: "read_file".into(),
                    args: serde_json::json!({
                        "path": self.sibling_path,
                        "start_line": 1,
                        "end_line": 20,
                    }),
                }],
                _ => Vec::new(),
            };
            Ok(forge_provider::ModelResponse {
                content: "done".into(),
                tool_calls,
                usage: Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn autofix_iteration_cap_halts_the_self_heal_loop() {
        // The autofix self-heal loop re-runs the model when lint/test fail. If they NEVER pass, only
        // the `max_iterations` cap can stop it. Pin that: a turn makes one edit (arming autofix), the
        // lint command always fails (`false`), and the loop must stop at the cap, not spin forever.
        let dir = std::env::temp_dir().join(format!("forge-autofix-cap-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("f.py");
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let config = Config {
            permission_mode: forge_types::PermissionMode::AcceptEdits, // auto-allow the write
            autofix: forge_config::AutofixConfig {
                auto_lint: true,
                auto_test: false,
                lint_cmd: "false".to_string(), // always exits 1 → never "fixed"
                test_cmd: String::new(),
                max_iterations: 2,
                auto_detect: false,
            },
            ..Config::default()
        };
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(EditOnceThenDoneProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
                path: path.to_string_lossy().into_owned(),
            }),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(&dir),
            Box::new(capture),
            config,
            dir.to_str().unwrap(),
        )
        .unwrap();

        // Must RETURN (the cap stops it), not loop forever.
        session.run_turn("write the file").await.unwrap();

        let warnings: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                PresenterEvent::Warning(w) => Some(w.clone()),
                _ => None,
            })
            .collect();
        assert!(
            warnings.iter().any(|w| w.contains("reached iteration cap")),
            "the autofix loop must stop at its iteration cap; warnings: {warnings:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `mesh.self_review` is off by default (it regressed when on-by-default), but must stay WIRED:
    /// when enabled, a turn that edited runs a review pass that re-checks the diff before finishing.
    /// Pin that it actually fires + announces itself, so the gated feature can't silently rot.
    #[cfg(unix)]
    #[tokio::test]
    async fn self_review_runs_after_an_edit_turn_when_enabled() {
        let dir = std::env::temp_dir().join(format!("forge-selfreview-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("f.py");
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        // MeshConfig has no `Default` (Config builds it explicitly), so take the default mesh and
        // flip just `self_review`.
        let base_mesh = Config::default().mesh;
        let config = Config {
            permission_mode: forge_types::PermissionMode::AcceptEdits, // auto-allow the write
            mesh: forge_config::MeshConfig {
                self_review: true,
                ..base_mesh
            },
            ..Config::default()
        };
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(EditOnceThenDoneProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
                path: path.to_string_lossy().into_owned(),
            }),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(&dir),
            Box::new(capture),
            config,
            dir.to_str().unwrap(),
        )
        .unwrap();

        session.run_turn("write the file").await.unwrap();

        let warned = events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, PresenterEvent::Warning(w) if w.contains("self-review")));
        assert!(
            warned,
            "the self-review pass must run + announce itself when mesh.self_review is enabled"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn direct_completeness_classifier_accepts_identifier_migrations() {
        for prompt in [
            "The db keyword is deprecated; use database instead.",
            "Rename old_widget to widget throughout the parser.",
            "Keep foo as a compatibility alias for bar.",
        ] {
            assert!(
                direct_completeness_is_identifier_migration(prompt),
                "expected identifier-migration classification for: {prompt}"
            );
        }
    }

    #[test]
    fn direct_completeness_classifier_rejects_unrelated_bug_reports() {
        for prompt in [
            "[Bug]: wspace and hspace in subfigures not working",
            "Regression in 5.2.3: pytest tries to collect random __init__.py files",
            "Subclassed SkyCoord gives misleading attribute access message",
        ] {
            assert!(
                !direct_completeness_is_identifier_migration(prompt),
                "unrelated bug report must not trigger direct completeness: {prompt}"
            );
        }
    }

    #[test]
    fn production_identifier_search_requires_production_scope_and_prompt_term() {
        let search = |path: &str, query: &str, result: &str| {
            let call_id = forge_types::new_id();
            vec![
                Message::assistant_tool_calls(
                    "",
                    vec![forge_types::ToolCall {
                        id: call_id.clone(),
                        name: "search".into(),
                        args: serde_json::json!({
                            "path": path,
                            "query": query,
                            "regex": false,
                        }),
                    }],
                ),
                Message::tool_result(call_id, result),
            ]
        };
        let root = std::path::Path::new("/repo");
        let prompt = "Replace deprecated db and passwd options with database and password.";

        assert!(
            completeness_production_identifier_search_matches(
                &search("tests/backends/mysql", "passwd", "tests.py:10: old passwd",),
                root,
                prompt,
            )
            .is_empty(),
            "a test-only search is not production completeness evidence"
        );
        assert!(
            completeness_production_identifier_search_matches(
                &search(
                    "django/db/backends/base",
                    "def copy(",
                    "base.py:10:def copy(",
                ),
                root,
                prompt,
            )
            .is_empty(),
            "an unrelated production search is not identifier-migration evidence"
        );
        assert_eq!(
            completeness_production_identifier_search_matches(
                &search(
                    "django/db/backends/mysql",
                    "passwd",
                    "base.py:203: kwargs['passwd']\nclient.py:15: get('passwd')",
                ),
                root,
                prompt,
            ),
            [
                "django/db/backends/mysql/base.py",
                "django/db/backends/mysql/client.py"
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            "a production search for a prompt identifier is sufficient search evidence"
        );
    }

    #[test]
    fn unresolved_completeness_carries_primary_matches_into_final_gate() {
        let primary_matches = ["src/parser.rs", "src/adapter.rs"]
            .into_iter()
            .map(str::to_string)
            .collect::<std::collections::BTreeSet<_>>();
        let primary_messages = vec![Message::assistant_tool_calls(
            "",
            ["src/parser.rs", "src/adapter.rs", "src/unrelated.rs"]
                .into_iter()
                .map(|path| forge_types::ToolCall {
                    id: forge_types::new_id(),
                    name: "read_file".into(),
                    args: serde_json::json!({"path": path}),
                })
                .collect(),
        )];
        let changed_paths = ["src/parser.rs"]
            .into_iter()
            .map(str::to_string)
            .collect::<std::collections::HashSet<_>>();
        let primary_opened_identifier_paths = opened_unchanged_production_paths(
            &primary_messages,
            std::path::Path::new("/repo"),
            &changed_paths,
        )
        .into_iter()
        .filter(|path| primary_matches.contains(path))
        .collect();

        assert_eq!(
            unresolved_completeness_production_paths(
                &primary_opened_identifier_paths,
                &[],
                std::path::Path::new("/repo"),
                &changed_paths,
            ),
            vec!["src/adapter.rs".to_string()],
            "an opened sibling found before the audit remains unresolved until the diff changes it"
        );
    }

    #[test]
    fn direct_scope_guidance_extracts_explicit_class_methods_only() {
        assert_eq!(
            direct_scope_guidance_named_apis(
                "Fix `Figure.subfigures()` consistently with Function._eval_evalf."
            ),
            vec!["Figure.subfigures", "Function._eval_evalf"]
        );
        assert!(
            direct_scope_guidance_named_apis(
                "Preserve X_out.dtypes on 5.2.3; do not collect __init__.py or ExitCode.OK"
            )
            .is_empty(),
            "variables, versions, filenames, and enum variants are not named production APIs"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn direct_completeness_runs_once_after_a_code_edit_when_enabled() {
        let dir =
            std::env::temp_dir().join(format!("forge-direct-completeness-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("f.py");
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let base_mesh = Config::default().mesh;
        let config = Config {
            permission_mode: forge_types::PermissionMode::AcceptEdits,
            mesh: forge_config::MeshConfig {
                verify_completeness: true,
                ..base_mesh
            },
            ..Config::default()
        };
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(EditOnceThenDoneProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
                path: path.to_string_lossy().into_owned(),
            }),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(&dir),
            Box::new(capture),
            config,
            dir.to_str().unwrap(),
        )
        .unwrap();
        session
            .run_turn("The parser currently returns the wrong value for deprecated aliases.")
            .await
            .unwrap();

        let fired = events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    PresenterEvent::Warning(message)
                        if message.contains("omitted related production paths")
                )
            })
            .count();
        assert_eq!(fired, 1, "direct completeness must run exactly once");
        let missing_search_retries = events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    PresenterEvent::Warning(message)
                        if message.contains("skipped its repository search")
                )
            })
            .count();
        assert_eq!(
            missing_search_retries, 1,
            "a prose-only first audit must trigger exactly one mandatory-search retry"
        );
        assert!(
            !session.last_turn_contract().requires_changed_artifact(),
            "the descriptive prompt must not make the contract trigger the audit"
        );
        assert!(
            session
                .transcript
                .iter()
                .any(|message| message.content.contains("targeted repository search")),
            "the direct provider must receive the evidence-grounded completeness prompt"
        );
        assert!(
            session
                .transcript
                .iter()
                .any(|message| message.content.contains("plain literal")
                    && message.content.contains("sibling files")
                    && message.content.contains("maximum THREE")
                    && message.content.contains("snippets are NOT inspection")
                    && message.content.contains("CONCRETE OMISSION")),
            "the audit must require a bounded, high-recall search rather than one narrow regex"
        );
        assert!(
            session.transcript.iter().any(|message| message
                .content
                .contains("skipped the REQUIRED repository search")),
            "the direct provider must receive the missing-search retry prompt"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn direct_completeness_skips_unrelated_bug_fixes() {
        let dir = std::env::temp_dir().join(format!(
            "forge-direct-completeness-unrelated-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("f.py");
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(EditOnceThenDoneProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
                path: path.to_string_lossy().into_owned(),
            }),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(&dir),
            Box::new(capture),
            Config {
                permission_mode: forge_types::PermissionMode::AcceptEdits,
                ..Config::default()
            },
            dir.to_str().unwrap(),
        )
        .unwrap();

        session
            .run_turn("[Bug]: wspace and hspace in subfigures not working")
            .await
            .unwrap();

        assert!(
            !events.lock().unwrap().iter().any(|event| {
                matches!(
                    event,
                    PresenterEvent::Warning(message)
                        if message.contains("omitted related production paths")
                            || message.contains("skipped its repository search")
                )
            }),
            "generic bug fixes must not run the direct completeness pass"
        );
        assert!(
            !session
                .transcript
                .iter()
                .any(|message| message.content == DIRECT_COMPLETENESS_PROMPT),
            "generic bug fixes must not receive the direct completeness prompt"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn direct_named_api_scope_guidance_is_injected_before_solving() {
        let dir =
            std::env::temp_dir().join(format!("forge-direct-named-api-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("f.py");
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(EditOnceThenDoneProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
                path: path.to_string_lossy().into_owned(),
            }),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(&dir),
            Box::new(CapturePresenter::default()),
            Config {
                permission_mode: forge_types::PermissionMode::AcceptEdits,
                ..Config::default()
            },
            dir.to_str().unwrap(),
        )
        .unwrap();

        session
            .run_turn("[Bug]: Figure.subfigures ignores explicit spacing")
            .await
            .unwrap();

        let scope_messages = session
            .transcript
            .iter()
            .filter(|message| {
                message.content.contains(DIRECT_NAMED_API_SCOPE_GUIDANCE)
                    && message.content.contains("Figure.subfigures")
            })
            .count();
        assert_eq!(
            scope_messages, 1,
            "named-API scope guidance must be injected exactly once before the solve"
        );
        assert!(
            session.transcript.iter().any(|message| {
                message.content.contains("control-flow guards")
                    && message.content.contains("squeeze/shape behavior")
                    && message.content.contains("moved or deleted guard")
            }),
            "named-API guidance must preserve existing control flow and return semantics"
        );
        assert!(
            !session
                .transcript
                .iter()
                .any(|message| message.content == DIRECT_COMPLETENESS_PROMPT),
            "scope guidance must not reactivate the outer completeness review"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn direct_identifier_migration_scope_guidance_is_injected_before_solving() {
        let dir = std::env::temp_dir().join(format!(
            "forge-direct-identifier-migration-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("f.py");
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(EditOnceThenDoneProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
                path: path.to_string_lossy().into_owned(),
            }),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(&dir),
            Box::new(CapturePresenter::default()),
            Config {
                permission_mode: forge_types::PermissionMode::AcceptEdits,
                ..Config::default()
            },
            dir.to_str().unwrap(),
        )
        .unwrap();

        session
            .run_turn("Replace deprecated db and passwd options with database and password.")
            .await
            .unwrap();

        let scope_messages = session
            .transcript
            .iter()
            .filter(|message| {
                message
                    .content
                    .contains(DIRECT_IDENTIFIER_MIGRATION_SCOPE_GUIDANCE)
            })
            .count();
        assert_eq!(
            scope_messages, 1,
            "identifier-migration scope guidance must be injected exactly once before the solve"
        );
        assert!(
            session.transcript.iter().any(|message| {
                message.content.contains("maximum TWO search commands")
                    && message.content.contains("Do not search tests alone")
                    && message.content.contains("production sibling match")
                    && message.content.contains("CONCRETE OMISSION")
                    && message
                        .content
                        .contains("retaining the old alias as a fallback")
            }),
            "migration guidance must require bounded production discovery and compatibility"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn direct_completeness_retries_an_empty_search_once() {
        let dir = std::env::temp_dir().join(format!(
            "forge-direct-completeness-empty-search-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("f.py");
        let audit_path = dir.join("audit.txt");
        let sibling_path = dir.join("sibling.py");
        std::fs::write(&sibling_path, "old_name = True\n").unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        let capture = CapturePresenter::default();
        let events = capture.events.clone();
        let base_mesh = Config::default().mesh;
        let config = Config {
            permission_mode: forge_types::PermissionMode::AcceptEdits,
            mesh: forge_config::MeshConfig {
                verify_completeness: true,
                ..base_mesh
            },
            ..Config::default()
        };
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(EditThenEmptySearchProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
                path: path.to_string_lossy().into_owned(),
                audit_path: audit_path.to_string_lossy().into_owned(),
                sibling_path: sibling_path.to_string_lossy().into_owned(),
                search_root: dir.to_string_lossy().into_owned(),
            }),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(&dir),
            Box::new(capture),
            config,
            dir.to_str().unwrap(),
        )
        .unwrap();

        session
            .run_turn("The parser currently returns the wrong value for deprecated aliases.")
            .await
            .unwrap();

        let retry_warnings = events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    PresenterEvent::Warning(message)
                        if message.contains("retrying once from the repository root")
                )
            })
            .count();
        assert_eq!(
            retry_warnings, 1,
            "an empty completeness search must trigger exactly one bounded fallback"
        );
        assert!(
            session.transcript.iter().any(|message| message
                .content
                .contains("likely search-scope or glob failure")),
            "the direct provider must receive the mechanically different fallback prompt"
        );
        assert!(
            events.lock().unwrap().iter().any(|event| {
                matches!(
                    event,
                    PresenterEvent::Warning(message)
                        if message.contains("left 1 evidence-backed production path")
                )
            }),
            "an evidence-backed unchanged production sibling must trigger the path-aware gate"
        );
        assert!(
            session.transcript.iter().any(|message| {
                message
                    .content
                    .contains("Evidence-backed but unchanged production paths")
                    && message.content.contains("sibling.py")
            }),
            "the final re-drive must name the exact unresolved production path"
        );
        assert!(
            events.lock().unwrap().iter().any(|event| {
                matches!(
                    event,
                    PresenterEvent::Warning(message)
                        if message.contains("retrying once with repository-state enforcement")
                )
            }),
            "an unchanged path after reconciliation must trigger one state-verified retry"
        );
        assert!(
            session.transcript.iter().any(|message| {
                message.content.contains("Still-unchanged production paths")
                    && message.content.contains("sibling.py")
            }),
            "the retry must name the exact path whose repository state did not change"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn autofix_stage_skipped_when_no_edits() {
        // edits_this_turn == 0 means the autofix outer condition evaluates to false;
        // test that run_autofix_stage is not reached (verify the guard independently).
        let store = Arc::new(Store::open_in_memory().unwrap());
        let session = Session::start(
            Arc::clone(&store),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(CapturePresenter::default()),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();
        // Fresh session: edits_this_turn must be 0 before any turn.
        assert_eq!(
            session.edits_this_turn, 0,
            "edits_this_turn starts at 0; autofix gate would not fire"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn autofix_stage_empty_cmd_is_skipped() {
        // When lint_cmd / test_cmd is empty the command must not run even if auto_lint/auto_test
        // is true (empty string = disabled per spec).
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = Session::start(
            Arc::clone(&store),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(CapturePresenter::default()),
            Config::default(),
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap();

        let af = forge_config::AutofixConfig {
            auto_lint: true,
            auto_test: true,
            lint_cmd: String::new(), // empty = disabled
            test_cmd: String::new(), // empty = disabled
            max_iterations: 3,
            auto_detect: false,
        };
        // No commands run → stage trivially passes.
        let passed = session.run_autofix_stage(&af).await.unwrap();
        assert!(passed, "empty commands → nothing runs → stage passes");
    }

    // ── Auto-review gate tests ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn serve_style_sessions_keep_distinct_workspace_metadata() {
        let base =
            std::env::temp_dir().join(format!("forge-workspace-session-{}", std::process::id()));
        let first = base.join("first");
        let second = base.join("second");
        let sentinel = base.join("sentinel");
        let _ = std::fs::remove_dir_all(&base);
        for root in [&first, &second, &sentinel] {
            std::fs::create_dir_all(root).unwrap();
            std::fs::write(
                root.join("AGENTS.md"),
                root.file_name().unwrap().to_string_lossy().as_bytes(),
            )
            .unwrap();
        }
        let store = Arc::new(Store::open_in_memory().unwrap());
        let make = |root: &std::path::Path| {
            Session::start(
                Arc::clone(&store),
                Arc::new(MockProvider),
                Arc::new(HeuristicRouter::new(Config::default())),
                ToolRegistry::with_core_tools_in(root),
                Box::new(CapturePresenter::default()),
                Config::default(),
                root.to_str().unwrap(),
            )
            .unwrap()
        };
        let (first_session, second_session) =
            tokio::join!(async { make(&first) }, async { make(&second) },);
        assert!(first_session.system_preamble()[1]
            .content
            .contains(first.to_string_lossy().as_ref()));
        assert!(second_session.system_preamble()[1]
            .content
            .contains(second.to_string_lossy().as_ref()));
        assert_eq!(
            store
                .session_cwd(first_session.session_id())
                .unwrap()
                .unwrap(),
            first.canonicalize().unwrap().display().to_string()
        );
        assert_eq!(
            store
                .session_cwd(second_session.session_id())
                .unwrap()
                .unwrap(),
            second.canonicalize().unwrap().display().to_string()
        );
        assert!(!sentinel.join("marker").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cc_pre_and_post_tool_hooks_receive_explicit_workspace_cwd() {
        let base = std::env::temp_dir().join(format!("forge-hook-cwd-{}", forge_types::new_id()));
        let workspace = base.join("workspace");
        let sentinel = base.join("sentinel");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&sentinel).unwrap();
        let capture = base.join("hook-cwds.txt");
        let _cwd_guard = test_cwd_guard(&sentinel);
        let command = format!(
            "read line; printf '%s\\n' \"$line\" >> {}",
            capture.display()
        );
        let config = Config {
            permission_mode: forge_types::PermissionMode::Bypass,
            hooks: vec![
                forge_config::HookConfig {
                    event: forge_config::HookEvent::PreToolUse,
                    matcher: Some("list_dir".into()),
                    command: command.clone(),
                    timeout_secs: 10,
                    cc_compat: false,
                },
                forge_config::HookConfig {
                    event: forge_config::HookEvent::PostToolUse,
                    matcher: Some("list_dir".into()),
                    command,
                    timeout_secs: 10,
                    cc_compat: false,
                },
            ],
            ..Config::default()
        };
        let mut session = Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(&workspace),
            Box::new(CapturePresenter::default()),
            config,
            workspace.to_str().unwrap(),
        )
        .unwrap();
        let call = forge_types::ToolCall {
            id: "list".into(),
            name: "list_dir".into(),
            args: serde_json::json!({}),
        };
        let session_id = session.session_id().to_string();
        let msg_id = session
            .store
            .add_message(&session_id, 0, Role::User, "hook", None)
            .unwrap();
        session.invoke_tool(&msg_id, &call).await.unwrap();
        let lines = std::fs::read_to_string(&capture).unwrap();
        let expected = workspace.canonicalize().unwrap().display().to_string();
        let payloads: Vec<serde_json::Value> = lines
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(payloads.len(), 2);
        for payload in payloads {
            assert_eq!(payload["cwd"], expected);
        }
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_validation_rejects_peer_repository_paths() {
        let root = std::env::temp_dir().join(format!("forge-workspace-a-{}", std::process::id()));
        let peer = std::env::temp_dir().join(format!("forge-workspace-b-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&peer);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&peer).unwrap();
        let rooted = subagent::rewrite_args_for_root(
            &serde_json::json!({ "path": "only-a.txt", "paths": ["also-a.txt"] }),
            &root,
        );
        let workspace = WorkspaceContext::new(&root).unwrap();
        validate_workspace_args(&rooted, &workspace).unwrap();
        assert!(validate_workspace_args(
            &serde_json::json!({ "path": root.join("../forge-workspace-b-").join(std::process::id().to_string()).join("peer.txt") }),
            &workspace,
        )
        .is_err());
        assert!(validate_workspace_args(
            &serde_json::json!({ "path": "/opt/forge-peer/peer.txt" }),
            &workspace,
        )
        .is_err());
        assert!(
            validate_workspace_args(
                &serde_json::json!({ "path": std::env::temp_dir().join("forge-peer/peer.txt") }),
                &workspace,
            )
            .is_err(),
            "a temporary workspace must not authorize every sibling temporary path"
        );
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(peer);
    }

    #[test]
    fn tool_batch_signature_distinguishes_calls() {
        use forge_types::ToolCall;
        let mk = |name: &str, args: serde_json::Value| ToolCall {
            id: "x".into(),
            name: name.into(),
            args,
        };
        let a = vec![mk("read_file", serde_json::json!({"path": "a.rs"}))];
        let a2 = vec![mk("read_file", serde_json::json!({"path": "a.rs"}))];
        let b = vec![mk("read_file", serde_json::json!({"path": "b.rs"}))];
        let c = vec![mk("edit_file", serde_json::json!({"path": "a.rs"}))];
        // Identical batches hash equal (drives doom-loop detection); different args or tool differ.
        assert_eq!(tool_batch_signature(&a), tool_batch_signature(&a2));
        assert_ne!(tool_batch_signature(&a), tool_batch_signature(&b));
        assert_ne!(tool_batch_signature(&a), tool_batch_signature(&c));
    }

    #[test]
    fn classify_tool_failure_detects_kinds_and_ignores_success() {
        assert_eq!(
            classify_tool_failure("error: No such file or directory (os error 2)"),
            Some(ErrorCategory::NotFound)
        );
        assert_eq!(
            classify_tool_failure("permission denied by policy"),
            Some(ErrorCategory::Permission)
        );
        assert_eq!(
            classify_tool_failure("error: no match for the given old_string"),
            Some(ErrorCategory::Schema)
        );
        assert_eq!(
            classify_tool_failure("error: the request timed out after 30s"),
            Some(ErrorCategory::Timeout)
        );
        assert_eq!(
            classify_tool_failure("error: the connection was reset by peer"),
            Some(ErrorCategory::Other)
        );
        // "not found" wins over the validation hint when both appear — fine; the guard only needs a
        // STABLE bucket so repeats of the same failure accumulate together.
        assert_eq!(
            classify_tool_failure("error: old_string not found in file"),
            Some(ErrorCategory::NotFound)
        );
        // Successful output that merely mentions a scary word must NOT be read as a failure.
        assert_eq!(
            classify_tool_failure("fn validate() { /* reject invalid states */ }"),
            None
        );
        assert_eq!(classify_tool_failure("file written"), None);
    }

    #[test]
    fn completion_gate_accepts_evidence_and_challenges_at_most_once() {
        const MAX: usize = 1;
        // Reasoning-only: one forced pass, then accepted calmly (never accepted at attempt 0).
        assert_eq!(
            completion_gate(0, MAX, false, false, false),
            CompletionGate::Reverify
        );
        assert_eq!(
            completion_gate(1, MAX, false, false, false),
            CompletionGate::AcceptNoArtifacts
        );
        // Fresh tool-grounded evidence newer than the last artifact mutation is already the proof
        // the gate would request; do not force a redundant model pass after `update_tasks`.
        assert_eq!(
            completion_gate(0, MAX, true, false, true),
            CompletionGate::AcceptClean
        );
        assert_eq!(
            completion_gate(1, MAX, true, false, true),
            CompletionGate::AcceptClean
        );
        assert_eq!(
            completion_gate(0, MAX, true, false, false),
            CompletionGate::Reverify
        );
        assert_eq!(
            completion_gate(1, MAX, true, false, false),
            CompletionGate::AcceptUnverified
        );
    }

    #[test]
    fn context_fill_uses_estimate_only_for_subscription_bridges() {
        // Direct API model: trust the provider's real input-token count.
        assert_eq!(
            context_fill_tokens("anthropic::claude-sonnet-4-5", 1_000, 50_000),
            50_000
        );
        assert_eq!(context_fill_tokens("openai::gpt-4o", 1_000, 50_000), 50_000);
        // Subscription CLI bridge: its reported usage is cumulative (here a bogus 900k), so the
        // gauge must use the transcript estimate instead — this is the 337%-gauge fix.
        assert_eq!(
            context_fill_tokens("claude-cli::opus", 90_000, 900_000),
            90_000
        );
        assert_eq!(
            context_fill_tokens("codex-cli::gpt-5.5", 90_000, 900_000),
            90_000
        );
        // xai-oauth:: is subscription-billed but NOT a cli bridge — it's a normal single-request
        // API call, so its reported input is accurate and must be trusted like a direct API model.
        assert_eq!(
            context_fill_tokens("xai-oauth::grok-4", 1_000, 50_000),
            50_000
        );
    }

    #[test]
    fn severity_meets_high_threshold() {
        use forge_types::Severity;
        // "high" gate: critical and high pass; medium and low do not.
        assert!(severity_meets(Severity::Critical, "high"));
        assert!(severity_meets(Severity::High, "high"));
        assert!(!severity_meets(Severity::Medium, "high"));
        assert!(!severity_meets(Severity::Low, "high"));
    }

    #[test]
    fn severity_meets_medium_threshold() {
        use forge_types::Severity;
        // "medium" gate: critical, high, medium pass; low does not.
        assert!(severity_meets(Severity::Critical, "medium"));
        assert!(severity_meets(Severity::High, "medium"));
        assert!(severity_meets(Severity::Medium, "medium"));
        assert!(!severity_meets(Severity::Low, "medium"));
    }

    #[test]
    fn severity_meets_low_threshold() {
        use forge_types::Severity;
        // "low" gate: everything passes.
        assert!(severity_meets(Severity::Critical, "low"));
        assert!(severity_meets(Severity::High, "low"));
        assert!(severity_meets(Severity::Medium, "low"));
        assert!(severity_meets(Severity::Low, "low"));
    }

    #[test]
    fn severity_meets_critical_threshold() {
        use forge_types::Severity;
        // "critical" gate: only critical passes.
        assert!(severity_meets(Severity::Critical, "critical"));
        assert!(!severity_meets(Severity::High, "critical"));
        assert!(!severity_meets(Severity::Medium, "critical"));
        assert!(!severity_meets(Severity::Low, "critical"));
    }

    #[test]
    fn severity_meets_unknown_threshold_is_permissive() {
        use forge_types::Severity;
        // Unknown threshold → fail-open (surface the finding).
        assert!(severity_meets(Severity::Low, "unknown-typo"));
        assert!(severity_meets(Severity::Medium, ""));
    }

    #[test]
    fn auto_review_gate_skipped_when_disabled() {
        // When auto_review = false, the gate condition is never entered regardless of edits.
        let cfg = forge_config::AssayConfig {
            auto_review: false,
            gate_severity: "high".to_string(),
            gate_mode: "block".to_string(),
            min_diff_bytes: 0,
            max_cost_usd: 0.0,
        };
        // The predicate `auto_review && edits_this_turn > 0` must be false with auto_review=off.
        let edits: u32 = 5;
        assert!(
            !(cfg.auto_review && edits > 0),
            "gate must be skipped when auto_review is off"
        );
    }

    #[test]
    fn auto_review_gate_skipped_when_no_edits() {
        // Even with auto_review=true, gate is skipped when edits_this_turn==0.
        let cfg = forge_config::AssayConfig {
            auto_review: true,
            gate_severity: "high".to_string(),
            gate_mode: "warn".to_string(),
            min_diff_bytes: 200,
            max_cost_usd: 0.0,
        };
        let edits: u32 = 0;
        assert!(
            !(cfg.auto_review && edits > 0),
            "gate must be skipped when no edits happened"
        );
    }

    #[test]
    fn auto_review_gate_skipped_when_diff_too_small() {
        // The diff-size check: if the concatenated diff is < min_diff_bytes the gate returns
        // early without running the crew. We test the predicate directly.
        let cfg = forge_config::AssayConfig {
            auto_review: true,
            gate_severity: "high".to_string(),
            gate_mode: "warn".to_string(),
            min_diff_bytes: 200,
            max_cost_usd: 0.0,
        };
        let diff = "small".to_string();
        assert!(
            diff.len() < cfg.min_diff_bytes,
            "a 5-byte diff is below the 200-byte threshold"
        );
    }

    // ── Assay gate cost-cap predicate tests ───────────────────────────────────────────────────

    #[test]
    fn gate_cap_zero_means_unlimited() {
        // max_cost_usd == 0.0 → cap is disabled, the gate always runs.
        let cfg = forge_config::AssayConfig {
            auto_review: true,
            gate_severity: "high".to_string(),
            gate_mode: "warn".to_string(),
            min_diff_bytes: 0,
            max_cost_usd: 0.0,
        };
        // When cap == 0.0 the gate skips the estimate check (never skips on cost).
        assert_eq!(
            cfg.max_cost_usd, 0.0,
            "zero cap means unlimited — cost check is skipped"
        );
    }

    #[test]
    fn gate_cap_exceeded_means_skip() {
        let cfg = forge_config::AssayConfig {
            auto_review: true,
            gate_severity: "high".to_string(),
            gate_mode: "warn".to_string(),
            min_diff_bytes: 0,
            max_cost_usd: 0.10,
        };
        let est_usd = 0.75_f64; // over cap
        assert!(
            cfg.max_cost_usd > 0.0 && est_usd > cfg.max_cost_usd,
            "gate should be skipped when estimate exceeds cap"
        );
    }

    #[test]
    fn gate_cap_not_exceeded_means_run() {
        let cfg = forge_config::AssayConfig {
            auto_review: true,
            gate_severity: "high".to_string(),
            gate_mode: "warn".to_string(),
            min_diff_bytes: 0,
            max_cost_usd: 0.50,
        };
        let est_usd = 0.10_f64; // under cap
        assert!(
            !(cfg.max_cost_usd > 0.0 && est_usd > cfg.max_cost_usd),
            "gate should run when estimate is within cap"
        );
    }

    #[test]
    fn cli_max_cost_abort_predicate() {
        // Mirror the CLI's guard: abort when !yes && max_cost.is_some() && est > cap.
        let yes = false;
        let max_cost: Option<f64> = Some(0.20);
        let est_usd = 0.85_f64;
        let should_abort = !yes && max_cost.is_some_and(|cap| est_usd > cap);
        assert!(
            should_abort,
            "should abort when estimate exceeds --max-cost"
        );

        // --yes overrides the cap
        let yes = true;
        let should_abort = !yes && max_cost.is_some_and(|cap| est_usd > cap);
        assert!(!should_abort, "--yes must bypass the cap check");

        // Under cap: no abort
        let yes = false;
        let est_usd = 0.05_f64;
        let should_abort = !yes && max_cost.is_some_and(|cap| est_usd > cap);
        assert!(!should_abort, "estimate under cap must not abort");

        // No --max-cost flag: never abort
        let max_cost: Option<f64> = None;
        let est_usd = 9999.0_f64;
        let should_abort = !yes && max_cost.is_some_and(|cap| est_usd > cap);
        assert!(!should_abort, "no --max-cost flag → never abort");
    }

    // ── Architect mode: model resolution tests ────────────────────────────────────────────────

    fn make_session(config: Config) -> Session {
        Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(forge_provider::MockProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(test_workspace()),
            Box::new(CapturePresenter::default()),
            config,
            test_workspace().to_str().expect("workspace path is UTF-8"),
        )
        .unwrap()
    }

    #[test]
    fn bump_tier_shifts_and_clamps_the_session_pin() {
        let mut session = make_session(Config::default());
        assert_eq!(session.pinned_tier(), None);
        // First press from a Standard baseline → Complex pin.
        assert_eq!(
            session.bump_tier(true, TaskTier::Standard),
            TaskTier::Complex
        );
        assert_eq!(session.pinned_tier(), Some(TaskTier::Complex));
        // Up again clamps at Complex.
        assert_eq!(
            session.bump_tier(true, TaskTier::Standard),
            TaskTier::Complex
        );
        // Down walks back through Standard → Trivial, then clamps.
        assert_eq!(
            session.bump_tier(false, TaskTier::Standard),
            TaskTier::Standard
        );
        assert_eq!(
            session.bump_tier(false, TaskTier::Standard),
            TaskTier::Trivial
        );
        assert_eq!(
            session.bump_tier(false, TaskTier::Standard),
            TaskTier::Trivial
        );
        // Clearing returns to normal classification.
        session.pin_tier(None);
        assert_eq!(session.pinned_tier(), None);
    }

    #[test]
    fn resolve_planner_falls_back_to_complex_tier_model() {
        // No architect_model set, no pin → first USABLE Complex-tier candidate. Deterministic
        // config (a single keyless candidate) so the result doesn't depend on which provider keys
        // happen to be set in the test environment.
        let mut config = Config::default();
        config.mesh.models.insert(
            forge_types::TaskTier::Complex.as_str().into(),
            forge_config::OneOrMany::Many(vec!["ollama::big".into()]),
        );
        let session = make_session(config);
        assert_eq!(session.resolve_planner_model(), "ollama::big");
    }

    #[test]
    fn resolve_editor_falls_back_to_standard_tier_model() {
        // No editor_model set, no pin → first USABLE Standard-tier candidate (deterministic config).
        let mut config = Config::default();
        config.mesh.models.insert(
            forge_types::TaskTier::Standard.as_str().into(),
            forge_config::OneOrMany::Many(vec!["ollama::mid".into()]),
        );
        let session = make_session(config);
        assert_eq!(session.resolve_editor_model(), "ollama::mid");
    }

    #[test]
    fn architect_planner_and_editor_skip_a_keyless_provider() {
        // The friend's bug: architect_mode on + the built-in tier defaults lead with `groq::…`, so
        // the planner/editor dispatched groq and auth-failed every turn (no groq key). The resolved
        // model must skip a no-key provider and pick the first USABLE candidate instead.
        assert!(
            !forge_config::has_api_key("minimax"),
            "test precondition: no minimax key"
        );
        assert!(forge_config::has_api_key("ollama"), "ollama is keyless");
        let mut config = Config::default();
        // First candidate keyless-unusable (no key), second keyless-usable.
        config.mesh.models.insert(
            forge_types::TaskTier::Complex.as_str().into(),
            forge_config::OneOrMany::Many(vec!["minimax::abab".into(), "ollama::y".into()]),
        );
        config.mesh.models.insert(
            forge_types::TaskTier::Standard.as_str().into(),
            forge_config::OneOrMany::Many(vec!["minimax::abab".into(), "ollama::z".into()]),
        );
        let session = make_session(config);
        assert_eq!(session.resolve_planner_model(), "ollama::y");
        assert_eq!(session.resolve_editor_model(), "ollama::z");
    }

    #[test]
    fn resolve_planner_uses_architect_model_when_set() {
        let mut config = Config::default();
        config.mesh.architect_model = Some("anthropic::claude-opus-4-8".to_string());
        let session = make_session(config);
        assert_eq!(
            session.resolve_planner_model(),
            "anthropic::claude-opus-4-8"
        );
    }

    #[test]
    fn resolve_editor_uses_editor_model_when_set() {
        let mut config = Config::default();
        config.mesh.editor_model = Some("groq::llama-3.1-8b-instant".to_string());
        let session = make_session(config);
        assert_eq!(session.resolve_editor_model(), "groq::llama-3.1-8b-instant");
    }

    #[test]
    fn pin_overrides_both_planner_and_editor() {
        // /model pin takes priority over both config fields and tier fallback.
        let mut config = Config::default();
        config.mesh.architect_model = Some("anthropic::claude-opus-4-8".to_string());
        config.mesh.editor_model = Some("groq::llama-3.1-8b-instant".to_string());
        let mut session = make_session(config);
        session.pin_model(Some("openai::gpt-4o".to_string()));
        assert_eq!(session.resolve_planner_model(), "openai::gpt-4o");
        assert_eq!(session.resolve_editor_model(), "openai::gpt-4o");
    }

    #[test]
    fn architect_mode_off_by_default() {
        // Default config must have architect_mode = false so run_turn is unchanged.
        let config = Config::default();
        assert!(!config.mesh.architect_mode);
    }
}
