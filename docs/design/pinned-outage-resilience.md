# Design: pinned-model outage resilience (turn must not die on transient stalls)

Incident: `forge run --model xai-oauth::grok-4.5` turn died with "provider unavailable:
stream stalled (no data for 90s)" after 2 hot retries. Root cause chain:

1. Provider stall → `ProviderError::Unavailable` (xai_oauth.rs:835, genai_provider.rs:1083).
2. run_model_loop hot-retries transients `MAX_TRANSIENT_RETRIES`× with 500ms<<n backoff
   (forge-core/src/lib.rs:3651-3665) — total ~a few seconds, far too short for a real blip.
3. `failover_policy(pinned=true, pin_failover=false, rate_limited=false)` → `FailoverPolicy::FailTurn`
   (lib.rs:427-433, enacted at 3732-3737). Turn hard-fails. Only `is_rate_limited()` earns
   `BackoffSameModel` (5s/15s/45s/60s-capped, ±20% jitter, ≤6 attempts, ≤180s budget).

## Changes

### 1. Policy: pinned + transient outage → backoff, not fail  (forge-core/src/lib.rs)
- `failover_policy(pinned, pin_failover, rate_limited)` → add `transient_outage: bool` param
  (true for `Unavailable`/`is_retryable() && !is_permanent() && !is_rate_limited()` after hot
  retries exhausted). Pinned + transient_outage → `BackoffSameModel`. Pinned + permanent
  (auth/capability) stays `FailTurn`. Table test extended (existing tests at lib.rs:11916).
- In the `BackoffSameModel` arm: keep ONE shared attempt/budget counter pair but use a separate,
  longer budget for outages: `mesh.pin_outage_wait_secs` (new config, default 600) with the same
  `pinned_backoff_delay` schedule, 60s-capped intervals, jitter as today. Rationale: RL resets are
  fast+signaled (Retry-After); outages need minutes. Rate-limit path keeps PINNED_RL_* unchanged.
- Warning UX (user decision: pin must always pin; warn about mesh fallback instead of doing it):
  - at each retry: existing status-bar ModelSearch event (no scrollback spam);
  - at 50% budget: `PresenterEvent::Warning` "…still unreachable — will keep retrying ~Ns more;
    /model to unpin or `mesh.pin_failover = true` allows mesh fallback";
  - on exhaustion: fail with the real error + same hint (mirrors 3723-3730 RL wording).

### 2. Provider: rotate OAuth account on stall, not just 429  (forge-provider/src/xai_oauth.rs)
The new rotation helper hops accounts once on `RateLimited`. Extend the hop trigger to
`Unavailable` connect/stall errors (fresh account = fresh edge session; stalls are often
per-connection). Same contract: ONE hop, only when `pool.has_rotation()`; second failure
surfaces to the core loop. Permanent Auth still never hops. Mirror in the shared helper so
codex-oauth inherits it. Tests: A stalls → B succeeds; both stall → single Unavailable up.

### 3. Config  (forge-config)
`mesh.pin_outage_wait_secs: u64`, default 600, 0 = disable outage backoff (old behavior).
Document in docs/features/mesh-routing (or wherever pin_failover is documented today).

## Non-goals
- No cross-model fallback under pin, ever (explicit user decision) — `mesh.pin_failover`
  remains the only escape hatch and is off by default.
- No turn checkpoint/resume machinery: the loop already re-sends the full transcript per
  attempt; nothing is lost while we keep retrying inside the turn.
- No change to unpinned mesh failover.

## Test plan
- failover_policy table: (pinned, outage) → Backoff; (pinned, permanent) → Fail; unpinned unchanged.
- Backoff arm: outage budget independent from RL budget; exhaustion fails with hint; 50% warning fires once.
- Provider: stall-hop tests as §2; single-account no-hop regression.
- Config default + 0-disables.
