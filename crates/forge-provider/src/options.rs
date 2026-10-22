//! The per-call types a [`Provider`](crate::Provider) is handed beside the transcript: the
//! checkpoint context of a CLI-bridge turn, structured-output requests and the completion options.

use forge_types::EffortLevel;

/// The per-turn snapshot context a CLI-bridge turn needs to hand its `forge mcp-serve` child so the
/// child snapshots the model's file edits into THIS turn's checkpoint dir and gates on the parent's
/// live permission mode. Passed EXPLICITLY (parent → provider → child `Command` env) instead of
/// through the parent's process-global env: a process-wide `set_var` here races a concurrent
/// `getenv` (UB) and would clobber another session's context in a future multi-session host. The
/// child still reads the same `FORGE_CHECKPOINT_*` / `FORGE_PERMISSION_MODE` var names from ITS OWN
/// env — only the parent stops mutating its global env. Field names map 1:1 to
/// `forge_core::snapshot::ENV_*`.
#[derive(Debug, Clone)]
pub struct CheckpointContext {
    /// Parent session id (`FORGE_CHECKPOINT_SESSION`).
    pub session: String,
    /// Current user-turn seq (`FORGE_CHECKPOINT_SEQ`).
    pub seq: i64,
    /// Absolute checkpoint root the child snapshots into (`FORGE_CHECKPOINT_ROOT`).
    pub root: String,
    /// Absolute workspace cwd for this session. CLI bridges and their `forge mcp-serve` children
    /// must run here rather than inheriting a multi-session daemon's process cwd.
    pub workspace: String,
    /// Parent's live permission temper key (`FORGE_PERMISSION_MODE`).
    pub mode: String,
    /// The conversation's history epoch: bumped whenever the transcript is rewritten from the
    /// outside (`/rewind`, `/uncompact`, a full-history reload). A CLI bridge that resumes its own
    /// server-side session must key that resume on this too — after a rewind the bridge's session
    /// still holds the turns Forge just removed, and a resumed delta would re-attach them.
    pub epoch: u64,
}

/// Env var naming the session workspace for a bridge's `forge mcp-serve` child.
pub const BRIDGE_WORKSPACE_ENV: &str = "FORGE_BRIDGE_WORKSPACE";

/// A provider-neutral structured-output request (OpenAI `response_format`). Backends that support
/// it (currently the genai/API path) map it to their JSON-mode / JSON-schema knob; backends that
/// don't (the CLI bridges) ignore it.
#[derive(Debug, Clone)]
pub enum ResponseFormat {
    /// Free-form JSON object (`{"type":"json_object"}`).
    JsonObject,
    /// Schema-constrained JSON (`{"type":"json_schema", ...}`).
    JsonSchema {
        name: String,
        schema: serde_json::Value,
    },
}

/// Per-completion options that extend the base [`Provider::complete`] signature without breaking
/// existing call sites. Passed via [`Provider::complete_with`]; the base `complete` ignores it.
#[derive(Debug, Clone, Default)]
pub struct CompletionOptions {
    /// Reasoning / thinking intensity hint forwarded to the model. `None` = provider default.
    pub effort: Option<EffortLevel>,
    /// Sampling temperature. `None` = provider default; coding turns set a low value so edits and
    /// patches are deterministic rather than creatively varied.
    pub temperature: Option<f32>,
    /// Penalties applied ONLY while a turn is under repetition pressure (the model has already
    /// restated itself). A coding turn runs at a low temperature on purpose, which also makes a
    /// repeated context produce a repeated continuation; these give the next request a different
    /// shape instead of asking the model again with the same one. `None` = send nothing, which is
    /// what every ordinary step does. Providers that do not accept them must omit them.
    pub frequency_penalty: Option<f32>,
    pub presence_penalty: Option<f32>,
    /// Checkpoint context handed explicitly to a CLI-bridge child. `None` for non-bridge calls and
    /// the base `complete` path, which fall back to inherited process env for legacy compatibility.
    pub checkpoint: Option<CheckpointContext>,
    /// Stable conversation identity for provider-side prompt caching. Main turns populate this via
    /// [`CheckpointContext::session`]; Forge's auxiliary calls use a purpose-scoped derivative so
    /// compaction/recap/diagnosis prefixes cache automatically without colliding with the main loop.
    pub prompt_cache_key: Option<String>,
    /// Ask the provider not to retain or train on this request, where it has a request-level
    /// control for that (OpenRouter `provider.data_collection = "deny"`). Set under
    /// `[privacy] zero_retention`; providers without such a control ignore it.
    pub deny_data_collection: bool,
    /// Per-call output ceiling. This is intentionally narrower than the provider-wide
    /// `mesh.max_output_tokens` setting: tiny auxiliary calls such as recap and suggestion can
    /// bound a misbehaving model without constraining the main coding turn. Providers that reject
    /// output caps may omit it.
    pub max_output_tokens: Option<u32>,
    /// A provider may retain a stateful response chain across this user-turn boundary. Main-loop
    /// callers set this only for a mesh-classified dependent continuation on the same warm model;
    /// the provider must still scope reuse to the exact session/model/account and verify that the
    /// new logical input extends the acknowledged chain. `false` starts a fresh turn boundary.
    pub reuse_response_chain: bool,
    /// Visible reusable request prefix at this completion boundary. Stateful providers compare
    /// this with observed cache and hidden-chain carry costs before retaining a response id across
    /// user turns. Zero means the caller has no defensible estimate.
    pub response_chain_prefix_tokens: u64,
    /// Structured-output request (OpenAI `response_format`). `None` = provider default (free text).
    pub response_format: Option<ResponseFormat>,
    /// Ask the provider to answer with a tool call (`tool_choice: "required"`). Set for the one
    /// request after the model narrated its next step and stopped with tasks still open. Honoured
    /// only where the provider is known to accept it; elsewhere it is a no-op.
    pub require_tool_call: bool,
}
