//! The Forge `Provider` trait: a minimal, provider-neutral model interface that every
//! other crate depends on instead of any concrete SDK (ADR-0003). v0.1 ships one real
//! implementation (`GenAiProvider`, backed by the `genai` crate, covering Anthropic /
//! OpenAI / Ollama) plus a deterministic `MockProvider` for offline tests and the
//! walking skeleton.

use async_trait::async_trait;
use forge_types::{EffortLevel, Message, QuotaHint, ToolCall, Usage};
use std::sync::Mutex;
use std::time::{Duration, Instant};

mod claude_bridge_home;
mod cli_provider;
mod codex_oauth;
mod codex_websocket;
mod embedder;
mod genai_provider;
mod mock;
mod oauth_responses;
mod tool_recovery;
mod xai_oauth;

pub use cli_provider::{
    codex_cli_detected_plan, codex_rollout_is_account_wide, CliKind, CliProvider, SUBAGENT_SINK_ENV,
};
pub use codex_oauth::{
    detected_plan as codex_oauth_detected_plan, exchange_code as exchange_codex_oauth_code,
    has_session as has_codex_oauth_session, is_pinned_codex_url,
    list_models as list_codex_oauth_models, probe_entitlement as probe_codex_entitlement,
    probe_quota as probe_codex_quota, CodexOauthProvider, CODEX_OAUTH_NAMESPACE,
};
pub use embedder::{select_embedder, GenaiEmbedder};
pub use genai_provider::{
    bundled_http_client, is_discoverable, list_custom_models, list_models, GenAiProvider,
};
pub use mock::MockProvider;
pub use tool_recovery::{
    looks_like_unexecuted_tool_call, recover_text_tool_calls, repair_malformed_args,
};
pub use xai_oauth::{
    has_session as has_xai_oauth_session, is_pinned_xai_url, list_models as list_xai_oauth_models,
    poll_for_tokens, probe_entitlement, start_device_login, EntitlementStatus, XaiOauthProvider,
    XAI_OAUTH_NAMESPACE,
};

/// Normalize legacy underscore-prefixed bridge/provider ids to the canonical hyphen form so
/// `codex_cli::gpt-5.4-mini` / `claude_cli::opus` / `xai_oauth::grok-4` work identically to their
/// hyphen forms.
pub fn normalize_model_id(model: &str) -> std::borrow::Cow<'_, str> {
    if let Some(rest) = model.strip_prefix("claude_cli::") {
        return std::borrow::Cow::Owned(format!("claude-cli::{rest}"));
    }
    if let Some(rest) = model.strip_prefix("codex_cli::") {
        return std::borrow::Cow::Owned(format!("codex-cli::{rest}"));
    }
    if let Some(rest) = model.strip_prefix("agy_cli::") {
        return std::borrow::Cow::Owned(format!("agy-cli::{rest}"));
    }
    if let Some(rest) = model.strip_prefix("xai_oauth::") {
        return std::borrow::Cow::Owned(format!("xai-oauth::{rest}"));
    }
    if let Some(rest) = model.strip_prefix("codex_oauth::") {
        return std::borrow::Cow::Owned(format!("codex-oauth::{rest}"));
    }
    std::borrow::Cow::Borrowed(model)
}

/// True when `model` routes to a subscription CLI bridge (`claude-cli::…` / `codex-cli::…` /
/// `agy-cli::…`). A bridge runs its OWN internal tool loop and returns the finished turn as a
/// single text response (no tool calls surface to the parent), so the parent must treat a bridge
/// response as terminal — it must NOT nudge it to "keep calling tools," which only re-runs the
/// whole bridge in confusion. `xai-oauth::…` is deliberately EXCLUDED: it's subscription-billed
/// like a bridge, but it's a normal single-turn API call whose tool calls DO surface to the
/// parent's own loop.
pub fn is_cli_bridge(model: &str) -> bool {
    let m = normalize_model_id(model);
    m.starts_with("claude-cli::") || m.starts_with("codex-cli::") || m.starts_with("agy-cli::")
}

/// TTL for [`detect_subscription_plans`]'s memoization. Short enough that a plan change (a fresh
/// `forge auth codex-oauth` login, a plan upgrade) is visible within a bounded window even if a
/// call site somehow misses [`invalidate_plan_cache`]; long enough to take the keyring/file-read
/// cost off the hot per-turn routing path. Named so it's greppable rather than a magic `60`.
/// Documented in docs/features/mesh-routing.md.
const PLAN_CACHE_TTL: Duration = Duration::from_secs(60);

/// A quota response is the backend's current account state, including a plan header. Keep that
/// observation only as long as the accompanying Codex quota is authoritative; an access-token
/// claim is merely a fallback when no fresh response has been seen.
const LIVE_CODEX_PLAN_TTL: Duration =
    Duration::from_secs(forge_types::CODEX_QUOTA_FRESHNESS_SECS as u64);

type PlanMap = std::collections::HashMap<String, String>;

/// Process-wide memoization for [`detect_subscription_plans`]. Deliberately NOT a process-lifetime
/// (`OnceLock`) cache: `forge mcp agent` / `forge mcp-serve` are long-lived daemons, so a
/// permanent cache would keep serving a stale plan forever after the user switches ChatGPT plans
/// or accounts. The `Instant` timestamp makes it self-healing on a short TTL instead.
static PLAN_CACHE: Mutex<Option<(Instant, PlanMap)>> = Mutex::new(None);

/// Fresh `x-codex-plan-type` observed from the account-wide Responses headers. Unlike the JWT
/// claim this reflects a just-accepted request, which is why it wins after an account upgrades.
static LIVE_CODEX_PLAN: Mutex<Option<(Instant, String)>> = Mutex::new(None);

/// Per-account ChatGPT plan, detected live from OAuth state and memoized for [`PLAN_CACHE_TTL`].
/// The uncached lookup (see [`detect_subscription_plans_uncached`]) does an OS keyring read (a
/// D-Bus roundtrip on Linux) plus an `auth.json` file read; `Session::live_quota` calls this from
/// 7 sites in forge-core, several per turn, on the hot routing path, so doing both on every call
/// is wasteful. A plain process-lifetime cache would be wrong here (see [`PLAN_CACHE`]'s doc) — a
/// 60s TTL self-heals without a restart, and [`invalidate_plan_cache`] forces an immediate refresh
/// right after a fresh OAuth login. The empty map (no codex session at all — the common case for
/// users who never used codex) is cached exactly like a non-empty result: it is a legitimate
/// answer, not "no cache yet".
/// Documented in docs/features/mesh-routing.md.
pub fn detect_subscription_plans() -> std::collections::HashMap<String, String> {
    cached_or_fetch(
        &PLAN_CACHE,
        PLAN_CACHE_TTL,
        detect_subscription_plans_uncached,
    )
}

/// Clear the memoized plan cache so the next [`detect_subscription_plans`] call re-fetches
/// immediately instead of waiting up to [`PLAN_CACHE_TTL`]. Call this right after a successful
/// subscription OAuth login — see `forge-cli`'s `invalidate_catalog_cache`, called from the same
/// `forge auth codex-oauth` / `forge auth xai-oauth` sites in `commands/local.rs`.
/// Documented in docs/features/mesh-routing.md.
pub fn invalidate_plan_cache() {
    clear_cache(&PLAN_CACHE);
    if let Ok(mut plan) = LIVE_CODEX_PLAN.lock() {
        *plan = None;
    }
}

/// Record a current plan returned by the authoritative Codex backend. Called only after a
/// successful OAuth response, and deliberately never persisted: it expires with the quota
/// observation and a future account switch cannot inherit it.
pub(crate) fn record_live_codex_plan(plan: &str) {
    let plan = plan.trim().to_string();
    if plan.is_empty() {
        return;
    }
    if let Ok(mut latest) = LIVE_CODEX_PLAN.lock() {
        *latest = Some((Instant::now(), plan));
    }
    // A route may ask for plans within the normal 60s JWT-cache TTL immediately after this
    // response; make the freshly observed header visible without waiting for that TTL.
    clear_cache(&PLAN_CACHE);
}

/// The fresh backend plan observed in this process, if any. Consumers persist it alongside the
/// quota snapshot when a completed OAuth turn crosses a process boundary.
pub fn fresh_live_codex_plan() -> Option<String> {
    let guard = LIVE_CODEX_PLAN.lock().ok()?;
    let (observed, plan) = guard.as_ref()?;
    (observed.elapsed() <= LIVE_CODEX_PLAN_TTL).then(|| plan.clone())
}

fn clear_cache(cache: &Mutex<Option<(Instant, PlanMap)>>) {
    // A poisoned lock means some earlier fetch panicked mid-update; either way an absent cache
    // is safe (the next call just re-fetches), so there's nothing to propagate here.
    if let Ok(mut guard) = cache.lock() {
        *guard = None;
    }
}

/// Generic TTL-cache-or-fetch over a `Mutex<Option<(Instant, PlanMap)>>`. The lock is held across
/// the `fetch()` call itself (not just the read/write of the cached value): a cold or
/// just-expired cache under concurrent callers must serialize onto a SINGLE fetch rather than each
/// caller independently hitting the keyring/disk. That's a deliberate trade of a slightly longer
/// lock hold (one keyring/file read, on a call gated by a 60s cooldown) for "never double-fetch".
fn cached_or_fetch(
    cache: &Mutex<Option<(Instant, PlanMap)>>,
    ttl: Duration,
    fetch: impl FnOnce() -> PlanMap,
) -> PlanMap {
    let mut guard = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((fetched_at, plans)) = guard.as_ref() {
        if fetched_at.elapsed() < ttl {
            return plans.clone();
        }
    }
    let plans = fetch();
    *guard = Some((Instant::now(), plans.clone()));
    plans
}

/// Uncached body of [`detect_subscription_plans`] (Fix 4, docs/design/subscription-efficiency-routing.md):
/// `codex-oauth` from the active provider-oauth account's access-token JWT, `codex-cli` from the
/// official CLI's own `~/.codex/auth.json`. Both surfaces bill the SAME ChatGPT account, so a
/// detected plan cannot disagree between them by construction. No entry for `claude-cli` /
/// `agy-cli` / `xai-oauth`: none of their tokens carry a plan claim, so they keep whatever
/// `[mesh.subscriptions]` says — never fabricated. Each half degrades independently to no entry
/// (missing session / missing file / no claim), never a panic or an error.
fn detect_subscription_plans_uncached() -> std::collections::HashMap<String, String> {
    merge_detected_codex_plans(
        fresh_live_codex_plan(),
        codex_oauth::detected_plan(),
        cli_provider::codex_cli_detected_plan(),
    )
}

/// Fold the three evidence tiers into the plan map without I/O, keeping the precedence testable.
/// A fresh header is one account-wide observation, so it deliberately synchronizes both Codex
/// surfaces rather than letting an older CLI JWT show a contradictory plan.
fn merge_detected_codex_plans(
    fresh_header: Option<String>,
    oauth_jwt: Option<String>,
    cli_jwt: Option<String>,
) -> PlanMap {
    let mut plans = PlanMap::new();
    if let Some(plan) = fresh_header {
        plans.insert("codex-oauth".to_string(), plan.clone());
        plans.insert("codex-cli".to_string(), plan);
        return plans;
    }
    if let Some(plan) = oauth_jwt {
        plans.insert("codex-oauth".to_string(), plan);
    }
    if let Some(plan) = cli_jwt {
        plans.insert("codex-cli".to_string(), plan);
    }
    plans
}

#[cfg(test)]
mod plan_cache_tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn second_call_within_ttl_does_not_refetch() {
        let cache: Mutex<Option<(Instant, PlanMap)>> = Mutex::new(None);
        let calls = AtomicUsize::new(0);
        let fetch = || {
            calls.fetch_add(1, Ordering::SeqCst);
            let mut m = HashMap::new();
            m.insert("codex-cli".to_string(), "plus".to_string());
            m
        };

        let first = cached_or_fetch(&cache, Duration::from_secs(60), fetch);
        let second = cached_or_fetch(&cache, Duration::from_secs(60), fetch);

        assert_eq!(first, second);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "second call must hit the cache"
        );
    }

    #[test]
    fn call_after_ttl_elapsed_refetches() {
        let mut stale = HashMap::new();
        stale.insert("codex-oauth".to_string(), "stale-plan".to_string());
        // Seed with a timestamp already past the TTL instead of sleeping 60s in a test.
        let expired_at = Instant::now() - Duration::from_secs(61);
        let cache: Mutex<Option<(Instant, PlanMap)>> = Mutex::new(Some((expired_at, stale)));
        let calls = AtomicUsize::new(0);
        let fetch = || {
            calls.fetch_add(1, Ordering::SeqCst);
            let mut m = HashMap::new();
            m.insert("codex-oauth".to_string(), "fresh-plan".to_string());
            m
        };

        let plans = cached_or_fetch(&cache, Duration::from_secs(60), fetch);

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "expired entry must be re-fetched"
        );
        assert_eq!(
            plans.get("codex-oauth").map(String::as_str),
            Some("fresh-plan"),
            "must return the fresh fetch, not the stale seeded value"
        );
    }

    #[test]
    fn empty_map_result_is_cached() {
        let cache: Mutex<Option<(Instant, PlanMap)>> = Mutex::new(None);
        let calls = AtomicUsize::new(0);
        let fetch = || {
            calls.fetch_add(1, Ordering::SeqCst);
            HashMap::new()
        };

        let first = cached_or_fetch(&cache, Duration::from_secs(60), fetch);
        let second = cached_or_fetch(&cache, Duration::from_secs(60), fetch);

        assert!(first.is_empty());
        assert!(second.is_empty());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "an empty result is still a cache hit on the second call"
        );
    }

    #[test]
    fn invalidate_forces_refetch() {
        // Exercises the same `clear_cache` helper `invalidate_plan_cache()` calls on the real
        // static, but on a local cache so this test never touches the real keyring/auth.json.
        let cache: Mutex<Option<(Instant, PlanMap)>> = Mutex::new(None);
        let calls = AtomicUsize::new(0);
        let fetch = || {
            calls.fetch_add(1, Ordering::SeqCst);
            HashMap::new()
        };

        let _ = cached_or_fetch(&cache, Duration::from_secs(60), fetch);
        let _ = cached_or_fetch(&cache, Duration::from_secs(60), fetch);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        clear_cache(&cache);

        let _ = cached_or_fetch(&cache, Duration::from_secs(60), fetch);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "invalidation must force the next call to re-fetch"
        );
    }

    #[test]
    fn concurrent_calls_do_not_deadlock_or_double_fetch() {
        let cache = Arc::new(Mutex::<Option<(Instant, PlanMap)>>::new(None));
        let calls = Arc::new(AtomicUsize::new(0));

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let calls = Arc::clone(&calls);
                std::thread::spawn(move || {
                    let plans = cached_or_fetch(&cache, Duration::from_secs(60), || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(20));
                        let mut m = HashMap::new();
                        m.insert("codex-oauth".to_string(), "pro".to_string());
                        m
                    });
                    assert_eq!(plans.get("codex-oauth").map(String::as_str), Some("pro"));
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("worker thread must not panic");
        }

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "concurrent cold-cache callers must serialize onto a single fetch"
        );
    }

    #[test]
    fn fresh_backend_plan_overrides_stale_jwts_for_both_codex_surfaces() {
        let plans = merge_detected_codex_plans(
            Some("pro".to_string()),
            Some("prolite".to_string()),
            Some("prolite".to_string()),
        );
        assert_eq!(plans.get("codex-oauth").map(String::as_str), Some("pro"));
        assert_eq!(plans.get("codex-cli").map(String::as_str), Some("pro"));
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// A non-retryable failure: bad request, malformed response, context-length, etc. It
    /// would fail the same way on any model, so the mesh must NOT fail over on it.
    #[error("provider request failed: {0}")]
    Request(String),
    /// Rate-limited / out of quota (HTTP 429, `RESOURCE_EXHAUSTED`). Retryable on another
    /// model; `retry_after` carries the server's cooldown when it told us one.
    #[error("rate limited: {message}")]
    RateLimited {
        message: String,
        retry_after: Option<std::time::Duration>,
    },
    /// The provider is down / the stream dropped (5xx, connection/timeout). Retryable.
    #[error("provider unavailable: {0}")]
    Unavailable(String),
    /// Authentication failed (HTTP 401/403) — the key is bad, missing, or lacks access. Failing
    /// over to *another* provider is correct, but the bad credential won't fix itself mid-session,
    /// so retrying THIS model auth-fails identically every turn (the per-turn failover churn). Like
    /// [`Capability`](Self::Capability) it's treated as PERMANENT: excluded on the long window +
    /// periodic re-probe (so it recovers automatically once the user fixes the key).
    #[error("provider auth failed: {0}")]
    Auth(String),
    /// A PERMANENT, model-specific incapability: this model can't serve Forge's (tool-using)
    /// turns at all — it rejects function calling, has no tool-supporting endpoint, mangles tool
    /// params, or the account can't afford it (HTTP 402 / "requires more credits"). Failing over
    /// to *another* model is correct, but retrying THIS one will fail identically every time, so
    /// the mesh excludes it (a long bench window) rather than benching it on a short cooldown.
    #[error("model unsupported: {0}")]
    Capability(String),
}

impl ProviderError {
    /// Whether the mesh should bench this model and fail over to another. True for
    /// rate-limit / unavailable / auth; false for [`Request`](Self::Request) (would fail
    /// identically everywhere).
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. } | Self::Unavailable(_) | Self::Auth(_) | Self::Capability(_)
        )
    }

    /// Whether this failure is PERMANENT for the model: it will recur on every call, so the model
    /// should be *excluded* (a long bench window + periodic re-probe), not benched on the short
    /// transient cooldown. True for [`Capability`](Self::Capability) (the model can't serve
    /// tool-using turns) and [`Auth`](Self::Auth) (the credential is bad/missing and won't fix
    /// itself mid-session) — both auth-fail/incapability-fail identically every turn otherwise.
    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::Capability(_) | Self::Auth(_))
    }

    /// Whether the credential itself is invalid or missing. Unlike a model capability failure,
    /// every alias for this provider will fail until the user re-authenticates.
    pub fn is_auth(&self) -> bool {
        matches!(self, Self::Auth(_))
    }

    /// Whether this is a rate-limit / quota-exhaustion failure (HTTP 429, `RESOURCE_EXHAUSTED`).
    /// Used by the failover loop to lazily skip the *same provider's* remaining chain entries
    /// after one of its models 429s — a rate limit is usually provider-wide, so the siblings would
    /// 429 too. Every other failure mode keeps strict mesh-rank failover order.
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Self::RateLimited { .. })
    }

    /// How long to bench the model: the server-provided `retry_after` when present,
    /// otherwise `default`.
    pub fn cooldown(&self, default: std::time::Duration) -> std::time::Duration {
        match self {
            Self::RateLimited {
                retry_after: Some(d),
                ..
            } => *d,
            _ => default,
        }
    }

    /// Heuristic: whether this failure is a context-length OVERFLOW (the prompt exceeded the
    /// model's window) rather than a genuine outage. Providers surface overflow inconsistently —
    /// often as a 4xx/5xx the generic classifier files under [`Unavailable`](Self::Unavailable) or
    /// [`Request`](Self::Request) — so we sniff the message. The correct response is to SHRINK the
    /// input (compact/trim) and retry the SAME model, not to bench a healthy model and fail over.
    pub fn is_context_overflow(&self) -> bool {
        let msg = match self {
            Self::Unavailable(m) | Self::Request(m) => m,
            Self::RateLimited { message, .. } => message,
            _ => return false,
        };
        let m = msg.to_lowercase();
        [
            "context length",
            "context window",
            "context_length",
            "maximum context",
            "maximum number of tokens",
            "too many tokens",
            "reduce the length",
            "prompt is too long",
            "input is too large",
            "exceeds the maximum",
            "string too long",
        ]
        .iter()
        .any(|k| m.contains(k))
    }

    /// A short reason string for the health record / UI ("rate-limited (429)", …).
    pub fn reason(&self) -> &'static str {
        match self {
            Self::RateLimited { .. } => "rate-limited",
            Self::Unavailable(_) => "unavailable",
            Self::Auth(_) => "auth failed",
            Self::Request(_) => "request error",
            Self::Capability(_) => "unsupported (no tool calling / unaffordable)",
        }
    }
}

#[cfg(test)]
mod error_tests {
    use super::*;

    #[test]
    fn is_cli_bridge_detects_both_forms() {
        assert!(is_cli_bridge("claude-cli::opus"));
        assert!(is_cli_bridge("codex-cli::gpt-5.5"));
        assert!(is_cli_bridge("claude_cli::opus"), "legacy underscore form");
        assert!(
            is_cli_bridge("codex_cli::gpt-5.5"),
            "legacy underscore form"
        );
        assert!(is_cli_bridge("agy-cli::gemini-3.5-flash"), "antigravity");
        assert!(
            is_cli_bridge("agy_cli::gemini-3.1-pro"),
            "antigravity legacy underscore form"
        );
        assert!(!is_cli_bridge("openrouter::google/gemini-3.5-flash"));
        assert!(!is_cli_bridge("gemini::gemini-3.5-flash"));
        assert!(!is_cli_bridge("ollama::llama3.2"));
        assert!(
            !is_cli_bridge("xai-oauth::grok-4"),
            "subscription-billed but not a bridge loop"
        );
    }

    #[test]
    fn normalize_model_id_handles_xai_oauth_legacy_underscore() {
        assert_eq!(normalize_model_id("xai_oauth::grok-4"), "xai-oauth::grok-4");
        assert_eq!(normalize_model_id("xai-oauth::grok-4"), "xai-oauth::grok-4");
        assert_eq!(
            normalize_model_id("codex_oauth::gpt-5.5"),
            "codex-oauth::gpt-5.5"
        );
    }

    #[test]
    fn is_context_overflow_sniffs_the_message_but_not_plain_outages() {
        // Providers surface overflow as Unavailable/Request/RateLimited with a telltale message.
        assert!(ProviderError::Unavailable(
            "This model's maximum context length is 128000 tokens".into()
        )
        .is_context_overflow());
        assert!(
            ProviderError::Request("input is too large for the context window".into())
                .is_context_overflow()
        );
        // A genuine outage / rate-limit is NOT an overflow — it must fail over, not compact.
        assert!(!ProviderError::Unavailable("502 bad gateway".into()).is_context_overflow());
        assert!(!ProviderError::RateLimited {
            message: "429 slow down".into(),
            retry_after: None
        }
        .is_context_overflow());
        assert!(!ProviderError::Auth("401".into()).is_context_overflow());
    }

    #[test]
    fn auth_and_capability_are_permanent_transient_outages_are_not() {
        // Permanent → excluded (long window + re-probe), never re-tried at the top of every turn.
        // A bad/missing credential and a tool-incapable model both recur identically every call.
        assert!(ProviderError::Auth("401 unauthorized".into()).is_permanent());
        assert!(ProviderError::Capability("no tool calling".into()).is_permanent());
        // Transient → short bench + fail over (the provider may recover on its own).
        assert!(!ProviderError::Unavailable("502".into()).is_permanent());
        assert!(!ProviderError::RateLimited {
            message: "429".into(),
            retry_after: None
        }
        .is_permanent());
        // All four still fail over to another model; only Request("…") is non-retryable.
        assert!(ProviderError::Auth("403".into()).is_retryable());
        assert!(!ProviderError::Request("malformed".into()).is_retryable());
    }
}

/// A tool advertised to the model so it can choose to call it.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema for the arguments object.
    pub schema: serde_json::Value,
}

/// The result of a single model completion: text, any requested tool calls, and usage.
#[derive(Debug, Clone, Default)]
pub struct ModelResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    /// Subscription quota observations surfaced by a CLI bridge this turn (Claude's
    /// `rate_limit_event` / Codex rollout). Empty for API providers / when the bridge
    /// reported nothing. Multiple entries when both the 5h and weekly windows were observed.
    pub quotas: Vec<QuotaHint>,
}

impl ModelResponse {
    pub fn wants_tools(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

/// A streamed event produced by a provider during a completion. Lets the UI animate not just
/// the answer but the model's *reasoning* and (for the agentic CLI bridge) its tool activity.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// A delta of the assistant's answer text (accumulates into [`ModelResponse::content`]).
    Text(String),
    /// A delta of the model's reasoning/thinking — shown live but NOT part of the final answer.
    Reasoning(String),
    /// The agent started a tool call. Emitted by the CLI bridge, whose agent loop runs tools
    /// itself; genai providers leave tool execution to forge-core and don't emit this.
    ToolStarted { name: String, args: String },
    /// A tool call finished (CLI bridge only).
    ToolFinished {
        name: String,
        ok: bool,
        summary: String,
    },
    /// A subagent was spawned. Emitted by the CLI bridge by tailing the out-of-band event sink
    /// that `forge mcp-serve` writes (so bridge-spawned subagents are visible in the TUI just
    /// like native ones — RFC subagent-orchestration Phase 3c).
    SubagentStarted {
        id: String,
        agent: String,
        task: String,
    },
    /// A live activity snippet from a still-running subagent (CLI bridge only).
    SubagentProgress { id: String, snippet: String },
    /// A subagent finished (CLI bridge only).
    SubagentFinished {
        id: String,
        agent: String,
        ok: bool,
        summary: String,
        cost_usd: f64,
    },
    /// The task list changed inside a bridged turn (the bridge model called `update_tasks` in the
    /// `mcp-serve` process). Tailed from the out-of-band sink so the TUI's sticky task panel
    /// updates LIVE during the turn, not only on completion (CLI bridge only).
    Tasks(Vec<forge_types::TodoItem>),
    /// The bridge model proposed a plan (`present_plan` in the `mcp-serve` process). Tailed from
    /// the out-of-band sink so the parent renders the plan card and runs the approval flow at turn
    /// end, exactly like the in-process path (CLI bridge only).
    Plan(forge_types::PlanProposal),
    /// Forge's own MCP tool server (`forge mcp-serve`, spawned by the bridged CLI to expose
    /// `mcp__forge__*` write tools) FAILED TO START this turn — the child logged an MCP-startup /
    /// `resources/list` failure, so the model ran with the filesystem effectively read-only and
    /// could not edit anything. Distinct from "the model chose not to edit": the tools were not
    /// available at all. The harness uses it to classify + retry a toolless bridge turn instead of
    /// scoring a silent empty completion as a clean run. `reason` carries the child's stderr signature
    /// (CLI bridge only). Evidence: a codex-cli::gpt-5.5 SWE-bench sweep hit this on ~7/15 instances,
    /// each of which then submitted an empty patch. Root cause of the ENOENT (sandbox vs load) is
    /// intermittent and unconfirmed; the respawn on retry usually clears it.
    ToolsUnavailable { reason: String },
}

/// A sink for [`StreamEvent`]s as they arrive (text, reasoning, tool activity).
pub type EventSink<'a> = dyn FnMut(StreamEvent) + Send + 'a;

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
    /// Parent's live permission temper key (`FORGE_PERMISSION_MODE`).
    pub mode: String,
}

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
    /// Checkpoint context handed explicitly to a CLI-bridge child. `None` for non-bridge calls and
    /// the base `complete` path, which fall back to inherited process env for legacy compatibility.
    pub checkpoint: Option<CheckpointContext>,
    /// Stable conversation identity for provider-side prompt caching. Main turns populate this via
    /// [`CheckpointContext::session`]; auxiliary calls leave it unset and therefore cannot consume
    /// a pinned session's subscription cache namespace.
    pub prompt_cache_key: Option<String>,
    /// Structured-output request (OpenAI `response_format`). `None` = provider default (free text).
    pub response_format: Option<ResponseFormat>,
}

/// A model backend. Implement this trait (and nothing in the core) to add a provider.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Run one completion against `model` given the transcript and the available tools.
    /// Streamed events (text, reasoning, tool activity) are delivered to `on_event` as they
    /// arrive; the full answer text is also returned in [`ModelResponse::content`].
    async fn complete(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError>;

    /// Like [`complete`] but accepts extra per-call options (e.g. effort / thinking intensity).
    /// The default implementation ignores `opts` and delegates to [`complete`], so existing
    /// backends need not change. Override in providers that support the options.
    async fn complete_with(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        opts: &CompletionOptions,
        on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError> {
        let _ = opts;
        self.complete(model, messages, tools, on_event).await
    }
}

/// Routes each turn to a backend by the model id's `provider::` prefix: `claude-cli::…` /
/// `codex-cli::…` go to the subscription CLI bridge; everything else goes to the genai-backed
/// API providers. This is the single `Provider` the CLI installs for a real session.
pub struct DispatchProvider {
    genai: GenAiProvider,
    claude_cli: CliProvider,
    codex_cli: CliProvider,
    /// Google Antigravity (`agy`) — text-mode only (no MCP), so always built `with_harness(false)`.
    agy_cli: CliProvider,
    xai_oauth: XaiOauthProvider,
    codex_oauth: CodexOauthProvider,
    /// One-time CLI-bridge ToS/discretion notice (FR-Part-B AC-B8).
    notice: std::sync::Once,
}

impl DispatchProvider {
    /// `harness` = run CLI-bridge turns through Forge's MCP tool server + permission gate
    /// (RFC Phase 2); `false` runs the CLI as its own agent (Phase 1).
    pub fn new(harness: bool) -> Self {
        Self {
            genai: GenAiProvider::new(),
            claude_cli: CliProvider::claude_code().with_harness(harness),
            codex_cli: CliProvider::codex().with_harness(harness),
            // agy has no MCP/`--tools` wiring → always text mode, never the Forge-MCP harness.
            agy_cli: CliProvider::antigravity().with_harness(false),
            xai_oauth: XaiOauthProvider::new(),
            codex_oauth: CodexOauthProvider::new(),
            notice: std::sync::Once::new(),
        }
    }

    /// Whether both supported MCP CLI bridges are using Forge's harness mode.
    pub fn harness_enabled(&self) -> bool {
        self.claude_cli.harness_enabled() && self.codex_cli.harness_enabled()
    }

    /// Cap output tokens on the genai (API-provider) and xAI-OAuth paths. `0` disables the cap.
    /// The CLI bridges manage their own output, so this doesn't affect them.
    pub fn with_max_output_tokens(mut self, cap: u32) -> Self {
        self.genai = self.genai.with_max_output_tokens(cap);
        self.xai_oauth = self.xai_oauth.with_max_output_tokens(cap);
        self.codex_oauth = self.codex_oauth.with_max_output_tokens(cap);
        self
    }

    fn cli_notice(&self) {
        self.notice.call_once(|| {
            tracing::warn!(
                "CLI-bridge runs your locally-installed claude/codex; Forge never sees your \
                 login. Using subscription CLIs from third-party tools may be restricted by \
                 Anthropic/OpenAI terms — you run this at your own discretion. See \
                 docs/features/provider-integrations.md."
            );
        });
    }
}

impl Default for DispatchProvider {
    fn default() -> Self {
        Self::new(true)
    }
}

#[async_trait]
impl Provider for DispatchProvider {
    async fn complete(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError> {
        let model = normalize_model_id(model);
        let model = model.as_ref();
        if model.starts_with("claude-cli::") {
            self.cli_notice();
            self.claude_cli
                .complete(model, messages, tools, on_event)
                .await
        } else if model.starts_with("codex-cli::") {
            self.cli_notice();
            self.codex_cli
                .complete(model, messages, tools, on_event)
                .await
        } else if model.starts_with("agy-cli::") {
            self.cli_notice();
            self.agy_cli
                .complete(model, messages, tools, on_event)
                .await
        } else if model.starts_with("xai-oauth::") {
            self.xai_oauth
                .complete(model, messages, tools, on_event)
                .await
        } else if model.starts_with("codex-oauth::") {
            self.codex_oauth
                .complete(model, messages, tools, on_event)
                .await
        } else {
            self.genai.complete(model, messages, tools, on_event).await
        }
    }

    async fn complete_with(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        opts: &CompletionOptions,
        on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError> {
        let model = normalize_model_id(model);
        let model = model.as_ref();
        if model.starts_with("claude-cli::") {
            self.cli_notice();
            self.claude_cli
                .complete_with(model, messages, tools, opts, on_event)
                .await
        } else if model.starts_with("codex-cli::") {
            self.cli_notice();
            self.codex_cli
                .complete_with(model, messages, tools, opts, on_event)
                .await
        } else if model.starts_with("agy-cli::") {
            self.cli_notice();
            self.agy_cli
                .complete_with(model, messages, tools, opts, on_event)
                .await
        } else if model.starts_with("xai-oauth::") {
            self.xai_oauth
                .complete_with(model, messages, tools, opts, on_event)
                .await
        } else if model.starts_with("codex-oauth::") {
            self.codex_oauth
                .complete_with(model, messages, tools, opts, on_event)
                .await
        } else {
            self.genai
                .complete_with(model, messages, tools, opts, on_event)
                .await
        }
    }
}
