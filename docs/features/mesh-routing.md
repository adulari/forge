# Mesh Routing — the normative reference

**Status: normative.** This document describes, 1:1 with the code, how Forge's mesh routing
system works today — classification, candidate ranking, subscription tracking, per-model burn,
metered-API cost, conservation, pins, and failover. Where this document and the code disagree,
the code wins and this document has a bug.

**Sync contract.** Every scoring constant here is asserted against the live symbols by
`crates/forge-mesh/src/doc_sync.rs` (test `mesh_routing_doc_matches_live_constants`): change a
constant without updating this file and CI fails. The documented functions carry a
"Documented in docs/features/mesh-routing.md" comment in source; if you change one of them,
update this file in the same PR.

Decision records (kept, not superseded):

- [ADR-0006 — Model Mesh: rule-based, pluggable routing](../architecture/decisions/0006-model-mesh-rule-based-routing.md)
- [ADR-0011 — Benchmark-driven model ranking](../architecture/decisions/0011-benchmark-driven-model-ranking.md)
- Design records: [subscription-efficiency-routing](../design/subscription-efficiency-routing.md)
  (burn weights, plan detection), [pinned-outage-resilience](../design/pinned-outage-resilience.md),
  [oauth-account-rotation](../design/oauth-account-rotation.md), [codex-oauth](../design/codex-oauth.md).

This file supersedes and replaces the former `mesh-classifier.md`, `provider-cost-routing.md`,
`quota-pace-tracking.md`, `free-models.md`, `model-health-failover.md`, `auto-discovery-mesh.md`
and `mesh-routing-finish.md`.

---

## 1. The pipeline at a glance

Every prompt goes through the same deterministic pipeline (no model call is spent on routing
itself, unless the opt-in LLM classifier is configured):

```
prompt
  │ 1. classify → TaskTier (Trivial | Standard | Complex)         §2
  │ 2. derive RouteHints: code_heavy + a stable per-prompt seed   §2.4
  │ 3. candidate set: the discovered catalog (or [mesh.models])   §3
  │ 4. conservation decision: spread this prompt off the subs?    §6
  │ 5. score every routable candidate (route_score)               §4
  │ 6. order by effort-dependent sort + tie-breaks                §7
  │ 7. filter usable (keys, health, quota, credit mode, context,  §8
  │    vision) and apply budget pressure
  │ 8. pick = first usable; rest = failover chain                 §8–9
  ▼
RoutingDecision { tier, model, rationale, fallbacks, pinned }     crates/forge-mesh/src/lib.rs:99
```

The router is `HeuristicRouter` (`crates/forge-mesh/src/lib.rs:334`), behind the `Router` trait
(`crates/forge-mesh/src/lib.rs:118`, async so the opt-in LLM classifier can do I/O). It is
constructed per turn via `HeuristicRouter::new(config)` (`crates/forge-mesh/src/lib.rs:607`) and
the builder methods `with_pin` (`lib.rs:622`), `with_catalog` (`lib.rs:629`),
`with_context_windows` (`lib.rs:637`) and `with_repo_boosts` (`lib.rs:644`).
`Router::route` (`lib.rs:1092`) classifies then delegates to `decide` (`lib.rs:915`), which is
the single selection path — pins, budget pressure, and chain building all live there.
`route_hinted` (`lib.rs:1115`) lets a command/skill `tier:` frontmatter replace classification
(the rest of the path is identical); `route_candidates` (`lib.rs:1146`) returns the top-n
distinct-provider decisions for `/duel`.

To see all of this live for a real prompt, run `forge mesh "<your task>"` (§11).

## 2. Task classification

### 2.1 The weighted heuristic

`score_prompt` (`crates/forge-mesh/src/lib.rs:488`) scores a prompt from local signals.
Thresholds (`lib.rs:574-581`): **score ≤ 0 → Trivial, score ≥ 5 → Complex, else Standard.**

Hard override first: any `COMPLEX_HINTS` phrase (`lib.rs:185` — "think hard", "ultrathink",
"step by step", "in depth", "comprehensive", "thorough", …) returns Complex immediately with
`score = i32::MAX` (certain when the heuristic is used as the availability fallback).

Otherwise, points accumulate:

| Signal | Points | Source |
|---|---|---|
| Very long prompt (> 120 words) | +3 | `lib.rs:506` |
| Long prompt (> 40 words) | +1 | `lib.rs:509` |
| Any `REASONING_TERMS` hit ("design", "debug", "why", "prove", "plan", "audit", …) | +5 | `lib.rs:215`, `lib.rs:515` |
| Code present (``` fence or a `CODE_TOKENS` symbol, `lib.rs:321`) | +3 | `lib.rs:514` |
| Any `ACTION_VERBS` hit ("implement", "migrate", "add a ", "write a ", …), word-boundary matched | +2 | `lib.rs:282`, `lib.rs:523` |
| Multi-step scope (`is_multistep`, `lib.rs:596`: " then ", bullet lists, "1."+"2.", "after that") | +2 | `lib.rs:530` |
| "test" / "benchmark" / "edge case" | +1 | `lib.rs:534` |
| Any `ERROR_MARKERS` hit ("panic", "traceback", "error[", …) | +1 | `lib.rs:323`, `lib.rs:541` |
| Each `ANALYSIS_TERMS` hit ("performance", "security", "review", …) | +3 each | `lib.rs:271`, `lib.rs:545` |
| Self-hosting infra term while working on Forge's own source (`SELF_HOSTING_INFRA_TERMS`, gated on `ProjectContext::is_self_hosting`) | +5 | `lib.rs:258`, `lib.rs:550` |
| Any `TRIVIAL_HINTS` hit ("quick", "simple", "one-liner", …) | −5 | `lib.rs:203`, `lib.rs:556` |
| Any `TRIVIAL_PATTERNS` hit ("typo", "rename", "add a comment", …), whole-word matched | −8 | `lib.rs:301`, `lib.rs:560` |

Length is deliberately one capped signal, never the decider — a 24-char "design a lock-free
queue" scores +5 from "design" and classifies Complex.

Word-boundary matching: `contains_word_boundary` (`lib.rs:423`) stops "port " matching inside
"report "; `contains_whole_word` (`lib.rs:448`) additionally checks the trailing boundary so
"rename" doesn't fire inside "renames".

### 2.3 Classifier kinds

`mesh.classifier` (`crates/forge-config/src/lib.rs:1698`) selects:

- `heuristic` — explicit opt-in, `score_prompt` only, zero added cost/latency.
- `llm` (default) — `LlmRouter` tries the explicit override first, then up to three fast free
  Standard-tier catalog choices, finally the configured trivial model. Health is checked per turn;
  benched models are skipped. Each candidate has a 5-second timeout and the total classification
  budget is 15 seconds. The first parseable answer wins. Only when every candidate fails, times
  out, or returns an unparseable reply does the mesh fall back to the heuristic.
- `hybrid` — legacy alias for `llm`; it no longer skips LLM classification on any normal turn.

All classifiers feed the same `HeuristicRouter::decide` selection path.

### 2.4 RouteHints: code-heaviness and the per-prompt seed

`RouteHints::from_prompt` (`crates/forge-mesh/src/lib.rs:401`) derives:

- `code_heavy` — `is_code_heavy` (`lib.rs:474`): a ``` fence, a `CODE_TOKENS` symbol, or an
  `ACTION_VERBS` hit. Switches the benchmark quality term to the *coding* index and enables the
  coding-provider prior (§4.4).
- `seed` — `stable_hash(prompt)` (`crates/forge-mesh/src/catalog.rs:578`, FNV-1a). Everything
  "random" in routing (provider rotation, the conservation roll) is derived from this hash, so a
  given prompt always routes identically while different prompts spread.

## 3. The candidate set

### 3.1 The discovered catalog

`ModelCatalog` (`crates/forge-mesh/src/catalog.rs:31`) holds the `provider::model` ids the user
can actually use right now. Discovery (querying each keyed provider's model list) lives in
forge-cli, not forge-mesh; the catalog is attached with `with_catalog`, benchmark scores with
`with_benchmarks` (`catalog.rs:691`), and `[mesh.burn_weights]` overrides with
`with_burn_weights` (`catalog.rs:700`). Bare bridge ids (`claude-cli::`) are rejected as catalog
entries (`catalog.rs:669-674`) — they remain valid manual pins. Models matching
`mesh.disabled` never enter the catalog (`crates/forge-cli/src/cli/commands/models.rs:304`).

Auto-discovery routing is active when `mesh.auto_discover` (default `true`,
`crates/forge-config/src/lib.rs:1279`) is on AND a non-empty catalog is attached
(`auto_active`, `crates/forge-mesh/src/lib.rs:659`). When active, `candidates_for_tier`
(`lib.rs:666`) ranks **every** routable catalog model (not a top-N — the tail feeds the failover
chain). When inactive (or the ranking comes back empty), the configured `[mesh.models]` lists
are used verbatim (`Config::candidates_for`, `crates/forge-config/src/lib.rs:2008`; a tier with
no entry falls back to the `standard` list).

> Note: when auto-discovery is active, `[mesh.models]` is **not** consulted per-tier — the
> catalog ranking replaces it wholesale. To route strictly from `[mesh.models]`, set
> `mesh.auto_discover = false`.

### 3.2 The routability filter

`is_routable` (`crates/forge-mesh/src/catalog.rs:121`) excludes task-specific endpoints that
cannot serve a general chat turn: image/video/audio generation (`imagen`, `veo`, `sora`,
`-tts`, `whisper`, …), embeddings/rerankers, translation-only models, `deep-research`,
`computer-use`, moderation/guard models, realtime voice, speech-to-text, and legacy base
completion models (`babbage`, `davinci`). Excluded models stay visible in `forge models` and can
still be pinned explicitly — the filter only guards the *general* routing pool.

### 3.3 Vision

`supports_vision` (`crates/forge-mesh/src/catalog.rs:163`) is a name-heuristic **allowlist** of
image-capable families (gpt-4o/4.1/5, o1/o3/o4, Claude 3+, Gemini, Llama-3.2-vision/Llama-4,
pixtral, Qwen-VL, Grok). When the current turn carries image attachments, routing prefers
vision-capable candidates and fails open to the unfiltered list if none is usable
(`ordered_usable_for_tier`, `crates/forge-mesh/src/lib.rs:818-830`).

## 4. Scoring a candidate: `route_score`

`route_score` (`crates/forge-mesh/src/catalog.rs:369`) is the full per-model score:

```
route_score(id) = capability_score_b(id, tier, code_heavy, bench)        §4.1
                + cost_pref(tier, cost_class(id, cost))                  §4.3
                + code_prior(provider, code_heavy, tier)                 §4.4
                − tool_reliability_penalty(id)                           §4.5
                − BRIDGE_SUPERSEDE_PENALTY                               §4.8 (superseded bridges only)
                − subscription_burn_penalty(id, tier, quota, overrides)  §4.6 (subscriptions only)
                − quota-status penalty                                   §4.7 (subscriptions only)
```

### 4.1 Capability fit

`capability_score_b` (`crates/forge-mesh/src/capability.rs:318`) blends a quality term `q` and a
speed term `s` per tier:

| Tier | Formula |
|---|---|
| Trivial | `s * 2.0 + q * 0.5` |
| Standard | `q + s` |
| Complex | `q * 2.0 + s * 0.25` |

**Quality `q`.** When the model has a benchmark score (§10), `q = (index / BENCH_INDEX_DIVISOR).clamp(0.0, 4.0)`
with `BENCH_INDEX_DIVISOR` = 20.0 (`capability.rs:17`) — the index is the Artificial Analysis
*coding* index for code-heavy prompts, else the *intelligence* index. The divisor maps the
~0–70 AA scale onto the 0–3-ish scale the name heuristic produces, so the cost/conservation
terms layered on top keep working whether or not bench data is present. Without a score, `q`
falls back to `quality_class` (`capability.rs:21`): a family-name heuristic mapping ids to
0–3 (small size markers like `-mini`/`-8b`/`haiku` → 1; frontier markers like `opus`/`gpt-5`/
`sonnet`/`-405b`/`-70b`/`grok-4` → 3; explicit 100 B+ param counts → 3 even when the product
name says "small"; known narrow/stale families (`codellama`, `palmyra`, `stockmark`) capped at
2; everything unknown defaults to 2, a capable default).

**Speed `s`.** `speed_class` (`capability.rs:285`) is 1–3, roughly inverse size: quality 3 →
speed 1, quality 2 → speed 2, else speed 3. One deliberate exception: the GPT-5.6 family
(`gpt-5.6-sol` / `-terra` / `-luna`) derives its speed class from relative burn weight instead
(`gpt56_family_speed_weight`, `capability.rs:268`: sol → 1, terra → 2, luna → 3), because all
three carry no size marker in their name and previously tied at the slowest class — scoring
Luna (the cheapest, fastest sibling) exactly as slow as Sol. **This substitution is scoped to
the GPT-5.6 family ONLY.** The Claude entries in the burn-weight table are intentionally
excluded from it: `speed_class` feeds a shared 1–3 scale compared against every other family,
most of which (including `gpt-5.4`/`gpt-5.5`) have no burn-weight entry and stay at the coarse
heuristic speed. Deriving Claude's speed from burn weight gave `claude-cli::sonnet` a speed bump
its peers couldn't match — verified live, it outscored `codex-cli::gpt-5.4`/`gpt-5.5` on every
Standard AND Complex prompt, silently re-introducing the single-provider monopoly the
`routing_spreads_across_providers_not_only_claude` test exists to prevent (see the doc comment
on `gpt56_family_speed_weight`).

### 4.2 Cost classes

`cost_class` (`crates/forge-mesh/src/catalog.rs:207`) sorts every model into one of three
classes — the two-currencies split (§5) starts here:

- **0 = genuinely free** — `is_free` (`catalog.rs:80`) requires *positive* evidence, not just a
  missing price: local `ollama::`, free-tier `groq::`, `gemini::` non-Pro models, `:free`
  variants on the paid gateways (`openrouter::`, `opencode_go::`), and custom OpenAI-compatible
  providers whose registry row says `free: true`. An unpriced model on any *metered* provider is
  **paid-with-unknown-cost, not free** — reading "no price in our table" as "free" is exactly
  the bug that billed a user by routing to `gpt-5-pro` at "cost $0".
- **1 = subscription** — `is_subscription` (`catalog.rs:61`): $0 marginal, burns plan quota (§5.2).
- **2 = metered/paid** — everything else; carries a real USD estimate (§5.1).

### 4.3 Per-tier cost preference

`cost_pref` (`crates/forge-mesh/src/catalog.rs:222`) is how much a tier *wants* each class,
added to the capability score. The full table (columns are cost classes 0 free / 1 subscription
/ 2 metered):

| Row | free (0) | subscription (1) | metered (2) |
|---|---|---|---|
| `cost_pref(Trivial, ·)` | 1.0 | 0.3 | -0.6 |
| `cost_pref(Standard, ·)` | 0.5 | 0.6 | -0.4 |
| `cost_pref(Complex, ·)` | 0.4 | 0.8 | 0.0 |

Policy: Trivial prefers genuinely-free so easy tasks never burn quota or dollars; Standard rates
subscription ≈ free with a slight subscription edge; Complex prefers the subscription flagship
($0 marginal, strongest reliable) with free as backup and metered as a neutral last option.

### 4.4 The coding/provider prior

`code_prior` (`crates/forge-mesh/src/catalog.rs:244`): a mild tiebreak nudge, never a hard rule.
Code-heavy → +0.3 for `codex-cli` / `claude-cli` / `anthropic` / `openai` / **`codex-oauth`**.
Trivial non-code → +0.2 for `groq` / `gemini` (fast cheap bulk).

`codex-oauth` was added to the code-heavy arm for parity with `codex-cli` — as of #586 it
dispatches the same coding-flagship models the bridge does, so it deserves the same lift.
**`xai-oauth`/`xai` are deliberately excluded**: there is no xai CLI bridge twin, so there is no
surface asymmetry to correct for, and granting grok the coding-flagship bonus here was out of
scope for the OAuth-supersedes-bridge work (§4.8) that motivated the `codex-oauth` addition.

### 4.5 Tool-reliability penalty

`tool_reliability_penalty` (`crates/forge-mesh/src/capability.rs:154`) subtracts
`TOOL_UNRELIABLE_PENALTY` = 3.0 (`capability.rs:146`) from any id containing both "gemini" and
"flash" — evidence-based: that family leaks structured tool calls as text (`<function=…>`)
across providers. Sized to drop an offender below an equal-bench tool-reliable peer while
keeping it in the failover chain. Reversible: remove the entry when upstream parsing is fixed.

### 4.6 The subscription burn-weight penalty

Subscriptions are $0 marginal, but not all requests burn the plan window equally — Anthropic and
OpenAI meter their plan windows roughly in proportion to list price, so one Fable request costs
the window ~10× what a Haiku request does. `subscription_burn_penalty`
(`crates/forge-mesh/src/catalog.rs:288`):

```
penalty = burn_k(tier) * ln(weight) * pressure_multiplier(fraction)
```

**The weight table** — `known_burn_weight` (`crates/forge-mesh/src/capability.rs:209`).
Derivation: published list prices on a nominal 1000-in/500-out mix (`in + 0.5·out` per 1M),
normalized so the cheapest sibling of each family = 1.0 (Haiku 4.5 and GPT-5.6 Luna are the
1.0 baselines). Matched on family *tokens* via `bench::tokens` — the same tokenizer benchmark
matching uses — so `anthropic::claude-fable-5`, `claude-cli::fable` and `codex-oauth::gpt-5.6-sol`
all resolve identically:

| Family | List price (per 1M in/out) | Burn weight |
|---|---|---|
| GPT-5.6 Sol | $5 / $30 | weight 5.0 |
| GPT-5.6 Terra | $2.50 / $15 | weight 2.5 |
| GPT-5.6 Luna | $1 / $6 | weight 1.0 |
| Claude Fable 5 | $10 / $50 | weight 10.0 |
| Claude Mythos 5 | $10 / $50 | weight 10.0 |
| Claude Opus 4.8 | $5 / $25 | weight 5.0 |
| Claude Sonnet 5 | $3 / $15 steady-state | weight 3.0 |
| Claude Haiku 4.5 | $1 / $5 | weight 1.0 |

Any model not in this table gets weight 1.0 (`subscription_burn_weight`, `capability.rs:244`).
Config overrides win: `[mesh.burn_weights]` is keyed by the **bare** model name (§13.1).

**Why `ln(weight)`:** the penalty must tie-break, not dominate. A 10× burn difference is
`ln(10) ≈ 2.30`, a 5× one `ln(5) ≈ 1.61` — big enough to matter between near-equal siblings,
small enough that a genuine capability gap (a full bench band ≈ 1.0 quality point ≈ 2.0 Complex
score points) still wins. Linear weights would make Fable's 10.0 swamp everything.

**Why unknown = 1.0 = zero penalty:** `ln(1.0) = 0`, so a model with no table entry (and no
override) contributes *exactly* zero — behaviour is provably unchanged for every model outside
the table. That is load-bearing: an unknown subscription model must not be silently demoted by
a guessed weight.

**Tier scaling** — `burn_k` (`catalog.rs:264`), constants at `catalog.rs:259-261`:

- `BURN_K_TRIVIAL` = 1.0 — cheap tiers avoid flagship burn hard; a trivial task never needs the
  expensive sibling's extra capability.
- `BURN_K_STANDARD` = 0.7
- `BURN_K_COMPLEX` = 0.3 — Complex still allows a meaningful capability advantage to win, but
  a near-capability tie now prefers the materially cheaper sibling.

**Quota pressure** — `pressure_multiplier` (`catalog.rs:279`): a linear map of the live
pace-projected consumed-window fraction (§5.4) to `(0.5 + 1.5 * fraction).clamp(0.5, 2.0)` —
0.5 on a fresh window, 2.0 near-spent. This *scales the same-subscription tie-break* (Sol vs
Luna); it deliberately composes with — and does not duplicate — the conservation layer (§6),
which moves whole prompts off the subscription.

### 4.7 Quota-status penalties

Still inside `route_score` (`catalog.rs:386-391`): a subscription whose provider is at
`QuotaStatus::Warning` takes −5.0 (below any plausible alternative); `Exhausted` takes −100.0
(effectively last). Applied in the *score*, not as a post-sort, so non-subscription alternatives
make it into any truncated shortlist.

### 4.8 OAuth supersedes bridge: per-model preference and suppression

Native OAuth (`codex-oauth::`) runs Forge's own harness against the provider directly; the CLI
bridge (`codex-cli::`) shells out to the vendor CLI's own agent loop instead. Once an OAuth
surface can dispatch a given model, it is structurally the better surface for that model. The mesh
therefore removes its CLI bridge twin from normal routing candidates and from the inspector when
that OAuth twin passes every eligibility check for the current turn.

**The pair list** — `OAUTH_SUPERSEDES` (`crates/forge-mesh/src/catalog.rs:314`):
`&[("codex-oauth", "codex-cli")]`, `(oauth, bridge)`. As of #586, `codex-oauth` dispatches every
seeded codex model (including `gpt-5.6-luna` over the WS transport), so this pair rule already
covers every codex model — there is no per-model allowlist beyond the provider pairing itself.

**The penalty** — `BRIDGE_SUPERSEDE_PENALTY` (`catalog.rs:325`) = **1.0**. Applied inside
`route_score` (`catalog.rs:369-395`) when the caller marks a candidate `superseded`.

**Which ids get marked** — `superseded_bridge_ids` (`catalog.rs:333`) takes the FULL catalog model
list and returns the set of `bridge::model` ids whose OAuth twin (same bare model name, under the
paired OAuth provider) is also present in the catalog. It runs once per ranking pass
(`ranked_seeded`/`ranked_rows`, `catalog.rs:827` and `:953`) rather than per candidate, so the
demotion is O(n) over the catalog, not an O(n²) rescan. Catalog presence of a `codex-oauth::`
model already implies a live OAuth session — discovery is gated on `has_codex_oauth_session()` in
forge-cli — so `superseded_bridge_ids` does no session-probing of its own; it is a pure function
of the catalog's model ids.

**Why score then suppress.** With both twins present they otherwise score identically:
same bare model name → same `capability_score_b` (§4.1), same `cost_class`/`cost_pref` (§4.2–4.3,
both class 1 subscription), same `code_prior` (§4.4, now tied by the `codex-oauth` addition
above), the same burn weight (`bench::tokens` matches both ids to the same family, §4.6), and the
same account-wide quota reading (store layer — §5.3). A flat 1.0 penalty is large enough to
guarantee the OAuth twin outranks the bridge twin at every tier (Trivial/Standard/Complex all
compare on the same `route_score`, so a fixed additive penalty wins regardless of tier weighting)
before the final eligibility pass. Then `ordered_usable_for_tier` compares each bridge with its
paired OAuth twin via `oauth_twin_for_bridge`: if the OAuth model is available, enabled, healthy,
within quota, context-compatible, and permitted by credit mode, the bridge is removed from the
normal route, fallback chain, and `forge mesh` candidate table. This is generic over
`OAUTH_SUPERSEDES`, not Codex-specific. If any of those OAuth checks fails, the bridge remains
routable; on a later turn an OAuth dispatch failure is benched and the bridge becomes eligible.

**Pins bypass this entirely.** An explicit `--model codex-cli::gpt-5.6-sol` pin (§9) never goes
through `route_score` — `decide`'s pin branch dispatches the pinned id directly if it's usable. The
supersede penalty only affects *auto-routed* ranking, never a deliberate pin.

## 5. Two currencies: dollars and quota

The mesh spends two different resources and `route_score` is where they meet: **metered API
models spend dollars** (a real USD estimate feeds `cost_class` → `cost_pref`), **subscription
models spend plan quota** (a burn weight scaled by live window pressure feeds the burn penalty).
Free models spend neither. This section covers each side end to end.

### 5.1 Metered API models: USD pricing

**The rate table.** `Pricing` (`crates/forge-mesh/src/pricing.rs`) maps model id →
`ModelRate { input_per_1k, output_per_1k, cache_read_per_1k }` (`pricing.rs:10`). The bundled
defaults are `DEFAULT_RATES` (`pricing.rs:37`), USD per 1k tokens:

| Model | in / 1k | out / 1k |
|---|---|---|
| `openai::gpt-4o-mini` | 0.00015 | 0.0006 |
| `anthropic::claude-opus-4-8` | 0.005 | 0.025 |
| `anthropic::claude-fable-5` | 0.010 | 0.050 |
| `anthropic::claude-mythos-5` | 0.010 | 0.050 |
| `gemini::gemini-2.5-flash` | 0.0003 | 0.0025 |
| `gemini::gemini-2.5-pro` | 0.00125 | 0.01 |
| `deepseek::deepseek-chat` | 0.00027 | 0.0011 |
| `xai::grok-4` | 0.003 | 0.015 |

Precedence: bundled defaults **<** prices fetched live from a provider's model API (e.g.
OpenRouter) **<** explicit `[mesh.pricing]` config overrides
(`from_config_with_fetched`, `pricing.rs:107`).

**Cost computation.** `cost_for` (`pricing.rs:139`) is
`(in/1000)·rate_in + (out/1000)·rate_out`; **a model with no rate entry returns `0.0`** (never
panics). `cost_for_usage` (`pricing.rs:154`) prices cache-read tokens at the discounted
`cache_read_per_1k` when known (fresh input = `input_tokens − cached_input_tokens`); with no
cache rate it equals `cost_for`. This is what actual spend accounting uses.

**The routing comparator.** `estimated_cost` (`pricing.rs:170`) prices a nominal turn of
`NOMINAL_INPUT_TOKENS` = 1000 input and `NOMINAL_OUTPUT_TOKENS` = 500 output tokens
(`pricing.rs:177-178`). It is a *relative* comparator for ranking candidates, not a forecast.

**The unpriced-model footgun (real, live).** Because `cost_for` returns `0.0` for an absent
entry, a metered model with no `DEFAULT_RATES` row, no fetched price, and no config price is
*indistinguishable from free* on the cost axis. This is exactly how Fable 5 was briefly scored
as free — the fleet's most expensive metered model ranked as its cheapest until
`anthropic::claude-fable-5` got its entry. `is_free` (§4.2) now refuses to call unpriced
metered-provider models free, but their *estimated cost* still compares as 0.0.
**Invariant: any metered model added to the catalog defaults MUST get a `DEFAULT_RATES` entry**
(the comment on `DEFAULT_RATES` restates this).

**What counts as free.** `is_free` (`crates/forge-mesh/src/catalog.rs:80`) — positive evidence
only: `ollama::` (local) and `groq::` (standing free tier) are free when unpriced; `gemini::` is
free *unless* the id contains "pro" (Pro left the free tier in Apr 2026); `openrouter::` and
`opencode_go::` are paid gateways where **only `:free`-suffixed variants are free** (OpenCode
Zen bills premium models against one shared key balance — treating its unpriced models as free
silently drained it); custom OpenAI-compatible providers carry their own `free` flag in
`CUSTOM_OPENAI_PROVIDERS` (`forge-config`); every other metered provider (openai, xai,
anthropic, deepseek, …) has no standing free tier, so unpriced ⇒ paid. A config price of `0`
is also positive evidence of free.

**Budget caps.** Metered spend accumulates in the store and feeds `BudgetState`
(`crates/forge-mesh/src/lib.rs:27`; built by `Session::budget_snapshot`,
`crates/forge-core/src/lib.rs:2419`) with daily/weekly/monthly axes. `BudgetState::status`
(`lib.rs:82`) takes the strictest axis; `DEFAULT_WARN_FRACTION` = 0.8 (`lib.rs:69`).
`Exhausted` downshifts routing to Trivial (§8.4). `mesh.credit_mode = "strict"` goes further:
`allowed_under_credit_mode` (`lib.rs:761`) drops every paid metered model from auto-routing and
failover entirely (free + subscription only); an explicit `--model` pin still bypasses it.

### 5.2 Subscriptions: which surfaces, and what "plan" means

`catalog::is_subscription` (`crates/forge-mesh/src/catalog.rs:61`) names the five subscription
surfaces: **`claude-cli::`**, **`codex-cli::`**, **`agy-cli::`** (CLI bridges) and
**`codex-oauth::`**, **`xai-oauth::`** (subscription OAuth providers). $0 marginal cost, real
plan burn.

The plan slug per provider (how much headroom the user pays for) comes from two sources, merged
by `resolved_subscription_plans` (`crates/forge-core/src/lib.rs:1205`):

1. `[mesh.subscriptions]` config (`crates/forge-config/src/lib.rs:1400`), captured by
   `forge init` — e.g. `claude-cli = "max-20x"`, `codex-cli = "plus"`.
2. Live detection — `detect_subscription_plans` (`crates/forge-provider/src/lib.rs:102`):
   `codex-oauth` from the active OAuth account's access-token JWT, `codex-cli` from the official
   CLI's own `~/.codex/auth.json` (`detect_subscription_plans_uncached`,
   `forge-provider/src/lib.rs:157`). No detection exists for `claude-cli` / `agy-cli` /
   `xai-oauth` (their tokens carry no plan claim) — they keep whatever config says, never a
   fabricated value.

Merge semantics (`merge_subscription_plans`, `forge-core/src/lib.rs:1187`): **detected wins per
key** (it is read from the account actually in use), config fills the rest. Detection is
memoized for `PLAN_CACHE_TTL` = 60 s (`forge-provider/src/lib.rs:82`) because the uncached
lookup does an OS keyring read + a file read on the hot routing path; the cache is deliberately
TTL-based, not process-lifetime, so long-lived `forge mcp` daemons self-heal after a plan/account
switch. `invalidate_plan_cache` (`forge-provider/src/lib.rs:115`) forces an immediate refresh
right after a fresh `forge auth codex-oauth` / `xai-oauth` login.

**Invariant:** every production `SubscriptionQuota::with_plans` call site goes through
`resolved_subscription_plans`. There are four: `Session::live_quota`
(`crates/forge-core/src/lib.rs:2410`), `subagent::route_child`
(`crates/forge-core/src/subagent.rs:355`), `duel::run` (`crates/forge-core/src/duel.rs:101`),
and the `forge mesh`/`forge models` inspector
(`crates/forge-cli/src/cli/commands/models.rs:548`). A new call site that passes raw
`config.mesh.subscriptions` instead would render `plan ?` for detected-plan surfaces (the D4
defect this function exists to fix) — route any new site through it.

### 5.3 How consumed-window fraction is measured

The unit of subscription tracking is a `QuotaHint`
(`crates/forge-types/src/lib.rs:967`): `{ provider, window ("five_hour"/"weekly"/…), status,
resets_at, fraction_used }`. Hints are produced by:

- **Claude Code bridge** — its stream-json emits a `rate_limit_event` per turn; `CliProvider`
  parses it inline (`crates/forge-provider/src/cli_provider.rs:1745`, and `:2193` on the
  persistent-bridge path).
- **Codex bridge** — `codex exec --json` omits quota from stdout but writes it to its session
  rollout file (`~/.codex/sessions/...jsonl`) as `token_count` events with
  `rate_limits.{primary,secondary}.{used_percent,window_minutes,resets_at}`; Forge reads that
  file after the turn and emits one hint per non-stale window
  (`cli_provider.rs:1000-1027` — 300 min → "five_hour", 10080 min → "weekly").
- **Codex OAuth (provider layer)** — two independent paths, both mapping to the same window
  labels/status thresholds as the bridge above: the WS transport (#586, gated to `gpt-5.6-luna`)
  parses an in-band `codex.rate_limits` frame (`parse_rate_limits_frame`, function in
  `crates/forge-provider/src/codex_websocket.rs`); the HTTP path parses the `x-codex-*` response
  headers ChatGPT's backend sends on every `POST /responses` call (`parse_codex_quota_headers`,
  function in `crates/forge-provider/src/codex_oauth.rs`) — previously this path hardcoded
  `quotas: Vec::new()` and reported no usage at all.
- **Seeding from outside-Forge usage** — usage racked up in the raw CLIs would read as 0%
  otherwise. `Session::seed_subscription_quota` (`crates/forge-core/src/lib.rs:2060`) and the
  `forge mesh` path (`seed_store_quota`, `crates/forge-cli/src/cli/commands/models.rs:581`)
  record externally-observed percentages: codex from its rollout files, claude from a gated
  one-shot `claude --debug` rate-limit probe (skipped when the store row is younger than
  5 minutes, `models.rs:534-537`; freshness via `subscription_age_secs`,
  `crates/forge-store/src/lib.rs:1888`). Seeded rows map fraction ≥ 0.98 → Exhausted, ≥ 0.80 →
  Warning, else Ok, and carry no reset time so a real in-turn hint replaces them.

Persistence — `Store::record_quota` (`crates/forge-store/src/lib.rs:1761`) writes each hint
twice:

- `subscription_usage` — one row per (provider, window), **upserted**: the latest snapshot,
  what routing reads.
- `quota_history` — **append-only** (`record_quota_history`, `forge-store/src/lib.rs:1793`),
  one row per observation carrying a `fraction_used`: the time series pace projection needs.

The router's snapshot is built by `Store::quota_at` (`forge-store/src/lib.rs:1929`;
`current_quota`, `:1882`, is `quota_at(now)`): per provider it takes the strictest non-stale
window's status (only Warning/Exhausted rows are carried — Ok is the default), the **maximum
fraction** across active windows, and a pace projection (§5.4) derived from that same strictest
window's history. The result is a `SubscriptionQuota` (`crates/forge-types/src/lib.rs:984`),
enriched by `Session::live_quota` (`crates/forge-core/src/lib.rs:2410`) with the resolved plans
(`with_plans`, `forge-types:1016`) and the conservation opt-out (`with_conserve`,
`forge-types:1029` ← `mesh.subscription_conserve`, default `true`).

**Shared-account merge (store layer).** `codex-cli` and `codex-oauth` bill the SAME underlying
ChatGPT account, so their `subscription_usage`/`quota_history` rows must be read as one bucket,
never summed — otherwise dispatching a turn through each surface would double-count against a
single account's window. `Store::quota_at` resolves this via a provider-alias group
(`QUOTA_ALIAS_GROUPS`, const, and `quota_alias_members`, function — both in
`crates/forge-store/src/lib.rs`): every window row from either provider is folded into one
reading, and the merged result is surfaced back under BOTH provider keys — the latest observation
across either surface wins per window, they are never added together. This means `route_score`'s
burn-penalty pressure (§4.6) and quota-status penalty (§4.7) already see the same account-wide
picture for `codex-cli::*` and `codex-oauth::*` candidates, which is part of why the OAuth-
supersedes-bridge twins (§4.8) tie on everything except the flat supersede penalty.

Accessors the mesh uses: `status_for` (`forge-types:1036`, default Ok), `fraction_for`
(`:1044`, default 0.0), `plan_for` (`:1074`, default ""), `pace_for` (`:1050`),
`is_exhausted` / `is_pressured` (`:1084` / `:1089`).

**Observation gap (documented behaviour, not aspiration):** `claude-cli` and the merged
`codex-cli`/`codex-oauth` account (§5.3 above) are actually observed today (bridge streams,
rollout files, the WS `codex.rate_limits` frame, the HTTP `x-codex-*` headers, the claude probe).
`xai-oauth` and `agy-cli` still emit no quota hints, so their fraction reads 0.0 and their status
Ok unless something seeds them — their conservation pull comes from the tier base probability and
plan factor alone (§6).

### 5.4 Pace projection: `QuotaPace`

A window at 30% that got there in one hour is in more danger than one at 60% that took six days.
`compute_quota_pace` (`crates/forge-types/src/lib.rs:1154`) — pure, caller supplies `now` —
derives from the window's history:

1. Requires ≥ 2 points spanning ≥ `QUOTA_PACE_MIN_ELAPSED_SECS` = 300 s
   (`forge-types:1115`) — else `None` ("not enough data yet"; guards the near-zero-denominator
   spike right after a reset).
2. `rate_per_sec = (latest.fraction − earliest.fraction).max(0.0) / elapsed` (a mid-range
   rollover clamps to 0 rather than a negative rate); scaled to `rate_per_hour` / `rate_per_day`.
3. `projected_fraction_at_reset = latest.fraction + rate_per_sec · (resets_at − now).max(0)`
   (`None` without a known reset).
4. `time_to_exhaustion_secs = (1 − latest.fraction) / rate_per_sec` (`Some(0.0)` if already
   ≥ 100%; `None` if the rate isn't positive).
5. `exhaustion_warning = time_to_exhaustion < time remaining in the window`.

History is read back over `QUOTA_PACE_LOOKBACK_SECS` = 691200 seconds (8 days — wide enough for
a weekly window; `forge-types:1123`), both by the store's `quota_at` pace attachment and the
statusline's `emit_quota_pace` (`crates/forge-core/src/lib.rs:2084`), so both project off the
same series (`quota_history_since`, `crates/forge-store/src/lib.rs:1828`).

**The routing input** is `effective_fraction_for` (`crates/forge-types/src/lib.rs:1063`):
`max(fraction_for(provider), projected_fraction_at_reset.min(1.0))`. The projection can only
*raise* the fraction, never lower it — a cooling-down window still conserves on what is already
spent. Both the burn-penalty pressure multiplier (§4.6) and conservation (§6) consume this
pace-projected value, so protection ramps up *ahead of* a projected overrun instead of reacting
to one already at Warning.

## 6. Conservation: spreading whole prompts off the subscriptions

Independently of per-model scoring, the mesh decides per prompt whether to route *off* the
subscriptions entirely, onto a free-frontier alternative — so a complex-heavy workload doesn't
exhaust the plan even while every window is still green. `conserve_decision`
(`crates/forge-mesh/src/catalog.rs:455`), returning the fully-inspectable `ConserveDecision`
(`catalog.rs:434`):

1. **Enabled?** `quota.conserve_enabled()` ← `mesh.subscription_conserve` (default `true`,
   `crates/forge-config/src/lib.rs:1331`).
2. **Eligible?** At least one subscription is present in the catalog AND a *capable*
   non-subscription alternative exists for the tier (`has_nonsub_alternative`,
   `catalog.rs:396`). The capability bar is bench-aware (`is_capable_alternative`,
   `catalog.rs:381`): Complex requires a frontier alternative (`is_frontier_b`,
   `crates/forge-mesh/src/capability.rs:168` — measured intelligence ≥
   `FRONTIER_BENCH_THRESHOLD` = 20.0, `capability.rs:133`, else name-heuristic class 3);
   Standard requires a capable mid (intelligence ≥ `CAPABLE_BENCH_THRESHOLD` = 8.0,
   `capability.rs:138`, else class ≥ 2); Trivial always passes. This guard is why conservation
   never drops a hard task onto a nominally-large but measurably-weak model (Hermes 405B
   scores 9.0 — fails the frontier bar).
3. **Probability.** For each subscription provider present, `conserve_probability`
   (`catalog.rs:364`) computes a spread probability; the decision takes the **max** across
    provider. Each provider compares that same deterministic roll against its own probability, so a
    pressured subscription is demoted without abandoning a fresh sibling.

   ```
   base(tier) = 1.0  Trivial          (subscriptions are never worth spending on it)
                0.65 Standard
                0.30 Complex          (0.15 when code_heavy — subs earn their keep on code)
   ramp       = (fraction / 0.80).clamp(0, 1) * (1 - base)      # fraction is pace-projected
    P          = 1.0                                                 # Trivial
                 ((base + ramp) * plan_factor(plan)).clamp(0, 1)     # Standard / Complex
   ```

   `plan_factor` (`catalog.rs:342`): slug containing "20x" → 0.8; "max" or "pro" → 0.85;
   anything else (plus/team/unknown) → 1.0. A bigger plan has more headroom, so it is conserved
   *less*. The ramp reaches 1.0 exactly at the 0.80 Warning line: by the time a window is at
   80% (or *projected* to be — §5.4), every eligible prompt spreads.
4. **The roll.** `roll = stable_hash("{seed}:conserve") % 10000 / 10000` — deterministic per
   prompt. `fired = roll < P`.

When it fires, every subscription model takes a **soft** score demotion of
`CONSERVE_PENALTY` = 4.0 (`catalog.rs:336`, applied in `ranked_seeded`/`ranked_rows`): large
enough to drop an `Ok` subscription below the best free-frontier alternative, small enough that
the subscriptions stay in the shortlist as fallbacks if every alternative fails.

Division of labour, restated: **conservation** moves whole prompts off subscriptions;
the **burn penalty** (§4.6) chooses *which sibling* pays when a prompt does stay on one;
the **quota-status penalties** (§4.7) are the hard backstop at Warning/Exhausted.

## 7. Ordering and tie-breaks

`ranked_seeded` (`crates/forge-mesh/src/catalog.rs:739`) scores every routable model and sorts
by the active effort level (`ranked_rows`, `catalog.rs:869`, is the untruncated,
score-broken-out twin behind the inspector):

- **High / XHigh / WhiteHot:** integer bench-score **band** first (`bench_band`,
  `catalog.rs:20` — `floor(score)`; unbenched models band to `i64::MIN`, below every benched
  one: at high effort you asked for proven quality), then route score, then the tie chain.
  Banding replaced a pairwise "prefer the higher score when the gap ≥ 1.0" rule that was
  intransitive and panicked Rust's sort ("comparison function does not correctly implement a
  total order") on the first white-hot full-catalog ranking.
- **Medium (default):** route score, then the tie chain.
- **Low:** pure cheapest-first — cost class, then estimated cost, then speed class, then the
  score + tie chain.

The tie chain, in order:

1. `cost_class` — cheaper class wins at equal score.
2. `provider_rotation` (`catalog.rs:502`) — `stable_hash("{seed}:{provider}")`: different
   prompts rotate which provider wins a genuine tie, so a workload spreads across equally-good
   providers (claude ↔ codex) instead of always the alphabetically-first — while staying fully
   deterministic per prompt. (The claude-cli monopoly was literally an alphabetical tie-break
   bug.)
3. `model_weight` (`catalog.rs:513`) — how heavy a model is on its subscription: 3 for
   `opus`/`-pro`/`-max`/`ultra`, 1 for `haiku`/`-mini`/`nano`/`flash`/`-lite`/`instant`,
   else 2. At a genuine score tie the mesh spends the *lighter* sibling.
4. `fine_capability` (`catalog.rs:536`) — the first version number in the id (`gpt-5.5` → 5.5,
   `claude-opus-4-8` → 4.8), higher first. A *late* tiebreak, after the rotation, so it only
   orders same-provider/same-class siblings — never lets a higher raw version make one provider
   always win. Digit accumulation is capped at 9 digits so an embedded hash/timestamp can't
   overflow.
5. Model id (stable).

After ranking, `apply_repo_boosts` (`crates/forge-mesh/src/lib.rs:706`) stable-reorders by any
per-repo boost learned from past `/duel` outcomes (unboosted models keep ranked order; empty map
= no-op).

## 8. From ranking to a decision: `decide`

`decide` (`crates/forge-mesh/src/lib.rs:915`) applies, in order: pin handling (§9), budget
pressure, then candidate filtering and chain building.

### 8.1 Usability

`ordered_usable_for_tier` (`lib.rs:798`) filters the ranked candidates through three predicates:

- `is_usable` (`lib.rs:740`): the provider has a usable key (or is keyless), the model is not
  currently **benched** (`ModelHealth`, built from the store's `model_health` table — a model
  that failed with a retryable error is benched for the server's `Retry-After` when given, else
  `mesh.failover_cooldown_secs`, default 60 s, kept short because free-tier limits typically
  reset per minute), and it is not an exhausted-quota subscription (routed around entirely,
  like a benched model), via `catalog::is_subscription` — all five surfaces (§13.6).
- `allowed_under_credit_mode` (`lib.rs:753`): under `credit_mode = "strict"`, only free +
  subscription models pass (§5.1), same `catalog::is_subscription` predicate.
- `context_fits` (`lib.rs:651`): the model's known context window must exceed the required
  minimum; models with no recorded window are assumed to fit (fail-open). The requirement is
  `effective_min_context` (`lib.rs:411`): the caller's `min_context_tokens` scaled ×1.5 at High
  effort and ×2 at XHigh/WhiteHot. Windows come from the store (fetched from provider APIs);
  the CLI bridges have no queryable API, so theirs are hardcoded in `context_limit`
  (`crates/forge-mesh/src/pricing.rs:194`): `claude-cli` 1,000,000 tokens (200,000 for haiku),
  `codex-cli` 272,000, `agy-cli` 1,000,000; every other provider returns `None` and the core
  falls back to `CONSERVATIVE_CONTEXT_WINDOW` = 32,000 (`pricing.rs:184`) only when it must
  bound a request.

Then: the vision preference (§3.3) when the turn has images; a **stable** demotion of
`Warning`-pressured subscriptions to the back of the list (`quota.is_pressured`, `lib.rs:837`) —
still fallbacks, tried last; and (configured path only) a cheapest-first sort by
`(prefer_subscription && subscription, estimated_cost)` — the auto path keeps the ranked order
verbatim.

### 8.2 The failover chain

`build_chain` (`lib.rs:851`): the routed tier's usable models first, then the other tiers
(Complex → Standard → Trivial) as cross-tier fallbacks, deduped. The chain follows the mesh
ranking **verbatim** — the Nth model tried is the Nth-best ranked model, not the top model of
the Nth provider (a previous round-robin interleave destroyed cross-provider rank order).
Rate-limit storms are handled lazily downstream instead: forge-core skips a provider's
*remaining* chain entries only after one of its models actually returns a rate-limit error.
Because `candidates_for_tier` ranks the full catalog (§3.1), the chain is deep — a few dead
providers cannot exhaust it. Mid-turn, a short rate-limit reset on the best model is *waited
out* rather than degraded past: up to `mesh.rate_limit_wait_secs` (default 75 s,
`crates/forge-config/src/lib.rs:1317`) — the per-minute free-tier case; longer resets fall
through to bench + failover.

### 8.3 Rationale

The human-readable `rationale` records what happened: `"auto-selected best of N usable
<tier> models: <model>"`, or on a skipped primary the *reason* it was skipped ("no usable key" /
"model benched" / "excluded by strict credit mode" / "quota exhausted", `lib.rs:1044-1052`),
plus "(paid subscription)" when `prefer_subscription` applied. It is persisted per turn in the
store's `routing_decision` table — that table, not the statusline, is the ground truth for
"what did the mesh actually do".

### 8.4 Budget pressure

With no pin: an `Exhausted` budget (§5.1) downshifts the tier to Trivial before selection
(`lib.rs:986-995`). With a pin: an exhausted budget overrides the pin only when
`mesh.budget.cap_overrides_pin` is set (`lib.rs:931-935`).

## 9. Pins and failover semantics

An explicit pin (`--model` / `/model` / a hard duel pin) bypasses classification and the
credit-mode filter. `pin_is_dispatchable` (`crates/forge-mesh/src/lib.rs:367`) is the single
source of truth for "can this pin be dispatched at all" (provider key present or keyless) shared
by `forge run --model` and the OpenAI-compatible `forge api` endpoint, so the two paths cannot
diverge. A pin that is usable routes with `pinned: true` and — unless `mesh.pin_failover = true`
(default `false`, `crates/forge-config/src/lib.rs:1292`) — an **empty fallback chain**
(`lib.rs:971-973`): a pin must pin. An unusable pin (no key) falls back to the mesh pick with
`pinned: false`.

What a mid-turn provider error may do is decided by exactly one chooser, `failover_policy`
(`crates/forge-core/src/lib.rs:434`):

| pinned | condition | policy |
|---|---|---|
| no (or `mesh.pin_failover = true`) | any retryable error | `SwitchModels` — bench + walk the chain |
| yes | rate-limited | `BackoffSameModel` — wait out and retry the SAME model |
| yes | transient outage AND `mesh.pin_outage_wait_secs > 0` | `BackoffSameModel` (outage budget) |
| yes | permanent error, or outage with the budget disabled | `FailTurn` — surface the real error |

The pinned rate-limit backoff schedule (`pinned_backoff_delay`,
`crates/forge-core/src/lib.rs:394`; constants at `lib.rs:377-388`): a server `Retry-After` is
honored verbatim; otherwise exponential `PINNED_RL_BASE_SECS` = 5 s growing ×`PINNED_RL_GROWTH`
= 3 per attempt (5 s/15 s/45 s), capped at `PINNED_RL_DELAY_CAP_SECS` = 60 s per attempt, with
±20% jitter, at most `PINNED_RL_MAX_ATTEMPTS` = 6 attempts and `PINNED_RL_TOTAL_WAIT_SECS` =
180 s total. The transient-*outage* case uses the same delay schedule under its own, longer
budget — `mesh.pin_outage_wait_secs`, default 600 s (`crates/forge-config/src/lib.rs:1573`) —
via separate counters, since an outage recovers in minutes and must not eat the rate-limit
budget. `0` disables outage backoff (immediate `FailTurn`).

**Why context overflow is excluded from outage backoff** (`forge-core/src/lib.rs:3789-3793`):
a context-overflow error rides the same `Unavailable` classification, but after the compact
retries are spent, *waiting can never shrink the input* — backing off would burn the whole
outage budget on a lost cause, so it is explicitly carved out of `transient_outage`.

## 10. The benchmark layer (ADR-0011)

When `mesh.benchmark_ranking` is on (default `true`, `crates/forge-config/src/lib.rs:1336`) and
a dataset is cached, ranking uses measured performance instead of the name heuristic.
`BenchScore` (`crates/forge-mesh/src/bench.rs:19`) carries two Artificial Analysis indices
(each roughly 0–70 today): the composite `intelligence` index and the `coding` index. The
binary fetches + caches them; `BenchmarkScores` is pure data + matching.

**Matching is the hard part** — AA says "Claude 4.5 Sonnet", Forge says
`anthropic::claude-sonnet-4-5`, the bridge says `claude-cli::opus`:

- `tokens` (`bench.rs:180`) reduces a name to lowercased alphanumeric tokens, split on
  separators AND letter↔digit boundaries; a leading gateway path (`anthropic/claude-…`) is
  dropped to its last segment; parenthetical decoration is stripped first by `strip_parens`
  (`bench.rs:224`) — "(xhigh)", "(… Opus 4.8 Fallback)" would otherwise cross-match unrelated
  models (Fable's AA row name literally contains "Opus 4.8 Fallback"). Noise tokens
  ("latest", "preview", "instruct", "fallback", …) are dropped; disambiguating tier words
  (mini/nano/flash/max/pro/air) are kept.
- `insert` (`bench.rs:53`) collapses effort variants of one model ("GPT-5.5 (low)"/"(xhigh)")
  to a single canonical row keeping the highest-intelligence one — a model is represented by
  its best effort.
- `score_for` (`bench.rs:92`) tries an exact sorted-token-set match first (`canon`,
  `bench.rs:239`), then a fuzzy fallback: the row sharing the most tokens (`overlap`,
  `bench.rs:247`), **required** to share a *family word* (an alphabetic token ≥ 3 chars that is
  not a `ROLE_WORDS` member — "coder"/"chat"/"instruct"/"vision"/… describe capabilities many
  families share, and let deepseek-coder inherit Qwen-Coder's score before the exclusion) with
  ≥ 2 shared tokens, and **refused** on a version conflict (both sides carry numeric tokens
  with zero overlap): a brand-new `claude-sonnet-5` must not silently inherit Sonnet 4.6's
  stale score — which would also defeat the "no score yet → refetch" trigger. A versionless
  bridge alias (`claude-cli::opus`) is unaffected and maps to the best matching family row.
- `id_tokens` (`bench.rs:160`) injects a family token per bridge (`claude-cli`/`anthropic` →
  "claude", `codex-cli` → "gpt", `agy-cli` → "gemini") so bare aliases match at all.
- `exact_score_for` (`bench.rs:82`) is the no-fuzzy variant for precisely-named local tags
  (`ollama::qwen2.5-coder:14b`), where the fallback would cross-match sizes.

Unmatched models simply fall back to the family heuristic — no wrong guess is forced. The
measured index feeds `capability_score_b` (§4.1) and the frontier/capable thresholds (§6);
`bench_band` (§7) governs high-effort ordering.

## 11. The inspector: `forge mesh` and `/mesh`

`RoutingExplanation` (`crates/forge-mesh/src/explain.rs:49`) is produced by
`HeuristicRouter::explain` (`explain.rs:74`), which **re-runs the exact production scoring** (it
calls `decide` and `ranked_rows` — no parallel logic) and exposes every step: the classified vs
routed tier (they differ on a budget downshift), the classifier reasons, code-heaviness, the
seed, the full `ConserveDecision`, a `ProviderQuotaView` per subscription provider
(`explain.rs:29` — status, fraction, plan, pace projection, and the *spread probability*
computed from the same pace-projected fraction real routing uses), the ranked `CandidateRow`
table (`explain.rs:17` — each row's usability flag applies the FULL routing filter, credit mode
and context fit included), and the final pick + fallbacks + rationale. `Session::explain_routing`
(`crates/forge-core/src/lib.rs:2436`) feeds the `/mesh` overlay; `forge mesh "<prompt>"` prints
it (add `--json` for machine-readable output); bare `forge mesh` prints the quota + per-tier
overview. `/mesh explain` describes a hypothetical prompt's text-only routing — it has no notion
of a live turn's image attachments.

### 11.1 A fully worked example (live output, captured 2026-07-10, forge 2.5.7)

```
$ forge mesh "design and prove correct a lock-free concurrent queue in Rust"
classified: complex  ·  code-heavy: no  ·  reasons: reasoning/algorithmic term

quota:
  claude-cli  [███░░░░░░░]  26% · plan max-20x · Ok · spread P=42%
  codex-cli   [░░░░░░░░░░]   0% · plan plus · Ok · spread P=30%
  codex-oauth [░░░░░░░░░░]   0% · plan plus · Ok · spread P=30%
  ...
conservation: not fired (roll 0.62 ≥ P 0.42) → subscription kept

candidates (top 8):
  * #1  codex-cli::gpt-5.6-sol             score   6.70  cap  6.14  subscription · frontier
    #2  claude-cli::fable                  score   6.68  cap  6.49  subscription · frontier
    ...
pick: codex-cli::gpt-5.6-sol
```

Every number reproduces from this document:

- **Classification:** "design" and "prove" are `REASONING_TERMS` (+5); nothing else fires;
  score 5 ≥ 5 → Complex. No code fence / code token / action verb → `code_heavy = false`, so
  scoring uses the *intelligence* index.
- **Spread probabilities (§6):** claude-cli — base(Complex) 0.30; the 5-hour window is at
  fraction 0.26 (no pace projection raised it), so ramp = (0.26/0.80)·(1−0.30) = 0.2275;
  (0.30 + 0.2275) · plan_factor("max-20x") = 0.5275 · 0.8 = **0.422 → 42%**. codex ("plus",
  factor 1.0, fraction 0) — 0.30 → **30%**. The decision takes the max (0.42); the
  deterministic roll for this prompt is 0.62 ≥ 0.42, so conservation does not fire and no
  `CONSERVE_PENALTY` is applied.
- **`claude-cli::fable`, score 6.68:** its AA intelligence index measured 59.9 at capture time,
  so q = 59.9 / 20.0 = 2.995; `speed_class(fable)` = 2 (quality-class fallback — fable is
  deliberately NOT in the GPT-5.6 speed substitution, §4.1). Capability (cap column) =
  2·2.995 + 0.25·2 = **6.49**. Plus `cost_pref(Complex, subscription)` = 0.8; `code_prior` 0
  (not code-heavy); burn penalty = `BURN_K_COMPLEX` 0.30 · ln(10.0) · (0.5 + 1.5·0.26)
  = 0.30 · 2.3026 · 0.89 = **0.614**. Score = 6.49 + 0.8 − 0.614 = **6.68**.
- **`codex-cli::gpt-5.6-sol`, score 6.70:** index 58.9 → q = 2.945; `speed_class(sol)` = 1
  (burn-derived, §4.1). cap = 2·2.945 + 0.25·1 = **6.14**. Plus 0.8; burn = 0.30 · ln(5.0) ·
  pressure(0.0 → 0.5) = 0.30 · 1.6094 · 0.5 = **0.241**. Score = 6.14 + 0.8 − 0.241 = **6.70**.
- Sol leads Fable here: Fable's 0.35 capability advantage is not enough to justify twice the
  burn weight while Claude is 26% consumed. A materially larger benchmark lead still wins.

## 12. Configuration reference (routing-relevant `[mesh]` keys)

All in `crates/forge-config/src/lib.rs` (`MeshConfig`, line 1213):

| Key | Default | Effect |
|---|---|---|
| `models` | shipped free-first lists | Per-tier candidate lists; used verbatim when auto-discovery is off/empty (§3.1) |
| `auto_discover` | `true` | Rank the discovered catalog instead of `[mesh.models]` (`lib.rs:1279`) |
| `benchmark_ranking` | `true` | Use AA indices when cached (`lib.rs:1336`) |
| `classifier` / `classifier_model` | `llm` / unset | §2.3 |
| `prefer_subscription` | `true` | Configured-path ordering: subscriptions before metered (`lib.rs:1221`) |
| `subscriptions` | `{}` | Plan slug per provider, captured by `forge init` (`lib.rs:1400`) |
| `subscription_conserve` | `true` | Enable conservation spreading (§6) (`lib.rs:1331`) |
| `burn_weights` | `{}` | Per-model burn-weight overrides, keyed by BARE model name (§13.1) (`lib.rs:1266`) |
| `pricing` | `{}` | Per-model USD overrides (per 1k tokens), win over defaults + fetched (§5.1) |
| `daily_budget_usd` / `weekly_budget_usd` / `monthly_cap_usd` | unset | Budget axes (§5.1) |
| `warn_threshold` | 0.8 | Budget warning fraction |
| `budget.cap_overrides_pin` | `true` | Exhausted budget may override a pin (§8.4) (`lib.rs:1714-1721`) |
| `credit_mode` | `normal` | `strict` = free + subscription only in auto-routing/failover (§5.1) |
| `disabled` | `[]` | Models/providers excluded from discovery + routing |
| `failover` | `true` | Bench + retry on retryable errors (`lib.rs` `default_failover`) |
| `failover_cooldown_secs` | 60 | Default bench duration without a server `Retry-After` (`lib.rs:1308`) |
| `rate_limit_wait_secs` | 75 | Longest in-turn wait for the best model's rate-limit reset (`lib.rs:1317`) |
| `pin_failover` | `false` | Escape hatch: allow cross-model failover off a pin (§9) (`lib.rs:1292`) |
| `pin_outage_wait_secs` | 600 | Pinned transient-outage wait budget; 0 disables (§9) (`lib.rs:1301`) |

Multi-key rotation: every key-based provider accepts multiple API keys (repeat
`forge auth <provider>`, a comma-separated env value, or numbered `_2`… env siblings —
`api_keys`, `crates/forge-config/src/lib.rs:3751`); with ≥ 2 keys the provider client
round-robins per request and a 429 retries on the next key
(`crates/forge-provider/src/genai_provider.rs:239-273`). Adding an OpenAI-compatible provider is
one `CustomProvider` row in `CUSTOM_OPENAI_PROVIDERS` (`forge-config`) — namespace, endpoint,
env var, `free` flag (which is what `is_free` §4.2 consults), seed models.

## 13. Sharp edges and deliberate approximations

### 13.1 `[mesh.burn_weights]` overrides key on the BARE name; the bundled table keys on family tokens

`subscription_burn_weight` (`crates/forge-mesh/src/capability.rs:244`) checks
`overrides.get(bare_model(id))` — an **exact** match on the model name after `provider::`.
The bundled table, by contrast, matches on tokenized *family words*. So:

```toml
[mesh.burn_weights]
"claude-fable-5" = 12.0   # matches anthropic::claude-fable-5  (bare name, exact)
"fable" = 12.0            # matches claude-cli::fable ONLY — NOT anthropic::claude-fable-5
```

The intuitive family-word override does **not** reach the API id form. To override one model
under every id form it appears as, add one entry per bare name. (Asserted by the doc-sync test.)

### 13.2 Sonnet 5's weight encodes steady-state pricing

Sonnet 5 is on introductory pricing ($2/$10 per 1M) through 2026-08-31, reverting to $3/$15 on
2026-09-01. The table encodes the steady-state 3.0 deliberately, so the constant doesn't
silently go stale the day the intro price expires (`capability.rs:226-231`). Until then the
mesh slightly *over*-counts Sonnet burn.

### 13.3 Tokenizer skew is deliberately not modelled

The burn weights price *tokens*; they don't model that different tokenizers emit different token
counts for the same text (Opus 4.7+/Fable/Mythos/Sonnet 5 emit roughly ~30% more tokens per unit
text than Haiku 4.5 / Sonnet 4.6). The weights are therefore a small systematic *under*estimate
of the heavy Claude models' true relative burn. Accepted: the error is second-order next to the
5–10× price ratios, and modelling it would require per-model tokenizer calibration the mesh has
no data source for.

### 13.4 The `with_plans` invariant

All four production `with_plans` call sites route through `resolved_subscription_plans` (§5.2).
Passing raw `config.mesh.subscriptions` at a new site will silently render `plan ?` for
detected-plan surfaces (`codex-oauth`/`codex-cli`) and mis-scale their `plan_factor` to 1.0.

### 13.5 Unpriced metered models compare as $0

See §5.1. `estimated_cost` = 0.0 for any model without a rate. `is_free` refuses to *label* them
free, but on the cost axis they still tie with genuinely-free models. The invariant stands: give
every metered catalog model a `DEFAULT_RATES` entry.

### 13.6 Two `is_subscription` predicates — FIXED (was: disagreed)

Forge-mesh used to carry two separate `is_subscription` predicates: `catalog::is_subscription`
(public, all five surfaces incl. `codex-oauth::`/`xai-oauth::`) and a *private* copy in
`crates/forge-mesh/src/lib.rs` that only recognized the three CLI bridges. The private copy has
been deleted; every call site in `lib.rs` (`is_usable`, `allowed_under_credit_mode`, `cost_rank`,
the `decide` "(paid subscription)" rationale label, and the tests) now calls the single public
`catalog::is_subscription` (`crates/forge-mesh/src/catalog.rs:61`) directly. Live consequences of
the fix:

- An `Exhausted` `codex-oauth`/`xai-oauth` quota is now hard-excluded by `is_usable`, same as an
  exhausted bridge — previously it was only demoted by the −100.0 score penalty (§4.7).
- Under `credit_mode = "strict"`, `codex-oauth::`/`xai-oauth::` models are now **included** in
  auto-routing (`allowed_under_credit_mode` recognizes them as $0-marginal subscription surfaces,
  same as the bridges) — previously they were wrongly excluded (neither bridge-subscription nor
  free by the old predicate's categories).
- `cost_rank`'s `prefer_subscription` ordering (configured path, §12) now sorts them rank 0
  (preferred) alongside the CLI bridges, not rank 1 (behind).

There is no longer an asymmetry to track here — this section is kept only as the historical
record of the fix (regression-tested by `cost_rank_treats_codex_oauth_and_xai_oauth_as_subscription`,
`crates/forge-mesh/src/lib.rs`, and the `is_usable`/strict-credit-mode assertions folded into the
existing test suite).

### 13.7 Quota observation: still a real gap for `xai-oauth`/`agy-cli`

§5.3: `claude-cli` and the merged `codex-cli`/`codex-oauth` account are now actually observed
(bridge streams, rollout files, the WS `codex.rate_limits` frame, the HTTP `x-codex-*` headers,
the claude probe). `xai-oauth` and `agy-cli` still produce no `QuotaHint`s, so their windows read
0% / Ok unless externally seeded. Their burn penalty runs at minimum pressure (×0.5) and their
conservation pull is the tier base × plan factor only. They additionally show `plan ?` unless
`[mesh.subscriptions]` names their plan (no detection exists for either).

### 13.8 Other deliberate approximations

- `estimated_cost`'s nominal 1000/500 mix (§5.1) is a comparator, not a forecast — real turns
  are often 100× larger; only the *relative* order matters to routing.
- `quality_class` defaults unknown families to 2 ("capable default") — an unknown model is
  neither punished nor crowned until a benchmark row matches it.
- Bridge context windows are hardcoded (§8.1) because the CLIs expose no queryable model API.
- The claude quota probe is gated to at most one `claude --debug` run per 5 minutes (§5.3) —
  the fraction can be up to that stale.

## 14. Keeping this document honest

- `crates/forge-mesh/src/doc_sync.rs::mesh_routing_doc_matches_live_constants` asserts every
  constant above against the live symbols (BURN_K_*, BENCH_INDEX_DIVISOR, CONSERVE_PENALTY,
  BRIDGE_SUPERSEDE_PENALTY, the full cost_pref table, every bundled burn weight, the nominal
  token mix, and the bare-name override semantics of §13.1). It fails naming the constant, its
  new value, and this file's path.
- The documented functions carry `Documented in docs/features/mesh-routing.md.` comments at
  their definitions. If you touch one, update the relevant section here in the same PR.
- The worked example in §11.1 uses live benchmark indices and quota fractions as captured on
  2026-07-10; those inputs drift (AA republishes, windows fill), but the *arithmetic* must
  keep reproducing from the formulas in §4–§6.
