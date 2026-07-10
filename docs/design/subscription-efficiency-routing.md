# Design: subscription usage-efficiency routing + per-account plan detection

Applies to every subscription surface: `claude-cli`, `codex-cli`, `codex-oauth`, `agy-cli`,
`xai-oauth`.

## The defects (verified against the live binary, 2026-07-10)

**D1 — per-model plan burn is invisible to the mesh.**
`catalog.rs::cost_class(id, cost)` returns `1` for *every* subscription model, and
`cost_pref(tier, class)` therefore hands Sol, Terra, Luna, `gpt-5.4-mini`, opus and haiku the
*identical* cost preference. `pricing.rs::cost_for` returns `0.0` for any id with no rate entry
(subscription ids have none), so `estimated_cost` is 0 for all of them. Net: the router cannot
see that one Sol request burns ~5x the plan of one Luna request.

**D2 — the "speed"/efficiency term is a NAME heuristic, blind to the new tier naming.**
`capability_score_b` = `q` (from benchmarks) + `s` where `s = speed_class(id)` and
`speed_class` is a pure function of `quality_class(id)` — a *name* matcher looking for `mini`,
`-30b`, family words. `sol` / `terra` / `luna` carry no size or speed marker, so all three land
in `quality_class == 3` → `speed_class == 1` (the *slowest* class). Luna, the fastest and
cheapest model in the family, is scored exactly as slow as Sol.

Arithmetic reproducing the observed scores confirms this (no guessing):
- complex `gpt-5.6-sol`: `q=58.9/20=2.945` → `2.945*2 + 1*0.25 + cost_pref(Complex,1)=0.8` = **6.94** ✓ observed 6.94
- standard `gpt-5.4-mini`: `q=2.0`, `s=3` → `2.0 + 3 + cost_pref(Standard,1)=0.6` = **5.60** ✓ observed 5.60

**D3 — quota is tracked per PROVIDER, not per model.**
`route_score` consults `quota.status_for(provider_of(id))`. The conservation + pace machinery
counts a Sol turn and a Luna turn identically against `codex-oauth`.

**D4 — plan type is config-only and unsynced across surfaces of the same account.**
`with_plans(config.mesh.subscriptions)` (forge-core lib.rs:2319, subagent.rs:355, duel.rs:101) is
the only source. `codex-cli` → `plus`, but `codex-oauth` has no entry → renders `plan ?` and gets
the smallest-headroom default, despite being **the same ChatGPT account**.

## Key enabling fact (verified)

The OAuth access token's JWT carries the plan, so it never has to be guessed or hand-synced:
`https://api.openai.com/auth` claim → `chatgpt_account_id` **and** `chatgpt_plan_type` (observed:
`plus`). The codex CLI's `~/.codex/auth.json` holds the same account id
(`tokens.account_id`) and the same JWT. Both surfaces can therefore *derive* the plan from the
account they actually use — they cannot disagree, and it is inherently per-account.

## Fix 1 — `subscription_burn_weight(id) -> f64` (forge-mesh)

Relative plan consumption per equivalent request, normalized so the cheapest sibling of a family
= 1.0. Computed from published list prices on the vendor's nominal mix (1000 in / 500 out), since
subscription quotas meter approximately in proportion to token price.

| family | in/out per 1M | weighted (in + 0.5·out) | weight |
|---|---|---|---|
| GPT-5.6 Sol | $5 / $30 | 20 | **5.0** |
| GPT-5.6 Terra | $2.50 / $15 | 10 | **2.5** |
| GPT-5.6 Luna | $1 / $6 | 4 | **1.0** |
| Claude Fable 5 | $10 / $50 | 35 | **10.0** |
| Claude Mythos 5 | $10 / $50 | 35 | **10.0** |
| Claude Opus 4.8 | $5 / $25 | 17.5 | **5.0** |
| Claude Sonnet 5 | $3 / $15 | 10.5 | **3.0** |
| Claude Haiku 4.5 | $1 / $5 | 3.5 | **1.0** |

Prices verified 2026-07-10 against `platform.claude.com/docs/en/about-claude/pricing`.
Fable 5 / Mythos 5 are the fleet's most expensive models (2x Opus 4.8) — omitting them would
have left the single most expensive model on the neutral 1.0 default, i.e. *unpenalized*.

**Sonnet 5 introductory pricing.** Sonnet 5 bills at $2/$10 per 1M through 2026-08-31, then
$3/$15 from 2026-09-01. The weight encodes the **steady-state $3/$15 → 3.0**, not the intro
price, so the constant does not silently go stale in September. Documented in code.

**Not modelled: tokenizer skew.** Opus 4.7+, Fable 5, Mythos 5 and Sonnet 5 use a newer
tokenizer emitting ~30% more tokens for the same text; Haiku 4.5 and Sonnet 4.6 use the old
one. Per unit of *text* the true Opus:Haiku burn ratio is therefore nearer 6.5 than 5.0. Left
unmodelled deliberately — it compounds an approximation (subscription quotas do not meter on
API list price to begin with), and half-modelling it is worse than documenting it.

**Unknown models default to 1.0 (neutral, no penalty)** — never fabricate a weight. A neutral
default means an unpriced model keeps *exactly* today's behaviour, so nothing regresses. Config
override: `[mesh.burn_weights]` (`"gpt-5.5" = 4.0`).

## Fix 2 — spend the weight in `route_score`, scaled by tier AND live quota pressure

```
penalty = BURN_K[tier] * ln(weight) * pressure_multiplier
```
- `BURN_K`: Trivial 1.0, Standard 0.7, Complex 0.15 — cheap tiers avoid flagships hard; complex
  still wants the flagship but tie-breaks toward the cheaper sibling when capability is close.
- `pressure_multiplier`: 0.5 when the plan is fresh (fraction < 0.25) … 2.0 when it is nearly
  spent — reuses the existing `SubscriptionQuota` fraction + `QuotaPace` (#573), so this composes
  with `conserve_decision` instead of duplicating it.
- `ln(weight)` keeps a 5x burn from swamping a genuine capability gap (ln 5 ≈ 1.61).
- Weight 1.0 → `ln(1) = 0` → zero penalty. Unknown models are untouched, by construction.

## Fix 3 — derive `speed_class` from burn weight when known (forge-mesh)

`speed_class` keeps its name heuristic as a fallback, but when `subscription_burn_weight` is
known, the cheapest sibling maps to the fast class and the flagship to the slow class. This fixes
Luna being scored as slow as Sol without adding `luna`/`terra` to a name list that would rot on
the next release.

## Fix 4 — per-account plan detection (forge-provider / forge-core)

`detect_subscription_plans() -> HashMap<String, String>`:
- `codex-oauth`: decode the **active account's** access-token JWT (no signature verification —
  same as the existing account-id extraction), read `chatgpt_plan_type`.
- `codex-cli`: read `~/.codex/auth.json` → `tokens.access_token` → same claim. Same account id ⇒
  same plan, by construction.
- Merge order at the three `with_plans` call sites: **detected plan wins over config**, config
  fills anything undetected (`agy-cli`, `xai-oauth` stay config/`?` until they expose a claim).

Surface the detected plan in `forge auth codex-oauth --list` and `forge mesh`.

## Non-goals
- No change to `conserve_decision` / pace (#573) — the pressure multiplier *reads* it.
- No per-model quota accounting in the store (would need vendor-published quota weights).
- No claude/antigravity OAuth (ToS).

## Correction found while verifying
`pricing.rs::DEFAULT_RATES` prices `anthropic::claude-opus-4-8` at `0.015 / 0.075` per 1k
(= $15 / $75 per 1M). Opus 4.8's actual list price is **$5 / $25** per 1M. The bundled rate is
~3x too high; corrected in this change.

## Test plan
- `burn_weight`: known families exact; unknown → 1.0; config override wins.
- `route_score`: Luna beats Sol at Trivial/Standard; Sol still wins Complex; a fresh plan
  penalizes less than a nearly-spent one; unknown-weight models score identically to before.
- `speed_class`: luna > terra > sol (faster→slower); name fallback unchanged for `mini`/`-30b`.
- Plan detection: JWT claim parsed; codex-cli and codex-oauth on the same account id agree;
  detected beats config; missing claim → config fallback; no keyring/secret access in tests.
