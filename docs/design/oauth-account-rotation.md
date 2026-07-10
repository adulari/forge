# Design: Multi-account OAuth auto-rotation

**Status:** design only (`feat/oauth-account-rotation`)  
**Goal:** with ≥2 stored OAuth accounts, round-robin per request and fail over on rate-limit/quota — generic (xAI today; Claude/Codex later).

---

## 1. Where multi-account OAuth lives today

**Storage (generic, keyring-backed)**

| Piece | Location |
|-------|----------|
| `OAuthAccountStore { version, active, accounts }` | `crates/forge-config/src/oauth.rs:219` |
| v1→v2 migrate on read | `parse_account_store` (`oauth.rs:291`) |
| Active-only load/store | `load_active_tokens` / `store_active_tokens` (`oauth.rs:324–334`) |
| Add / list / switch / remove | `add_account`…`remove_account` (`oauth.rs:343–390`) |
| Provider wrappers | `crates/forge-config/src/provider_oauth.rs:127–180` |
| Keyring key | `provider-oauth:<provider>` (xAI → `provider-oauth:xai`) |

**Request-time resolution (single active account)**

1. `XaiOauthProvider::complete_with` → `fresh_access_token` (`xai_oauth.rs:541–569`).
2. `load_provider_oauth_tokens` → **only the active account**.
3. Refresh writes via `store_provider_oauth_tokens` → `set_active_tokens` (active only; others untouched — `oauth.rs:252–255`).

**CLI:** `forge auth xai-oauth --list / --switch --account <id>` (`local.rs:84–147`). One `* active`; all inference uses it.

---

## 2. Existing API-key rotation (mirror this)

| Piece | Location | Behaviour |
|-------|----------|-----------|
| Multi-key sources | `api_keys` (`forge-config/src/lib.rs:3724`) | env + numbered siblings + keyring list |
| Pool | `KeyPool` (`genai_provider.rs:239–276`) | only providers with **≥2** keys; `AtomicUsize` cursor |
| Pick | `KeyPool::next` | round-robin per request via service-target resolver |
| 429 retry | `complete_with` (`genai_provider.rs:1049–1066`) | **one** next-key retry if `has_rotation` |
| Mesh contract | `forge-core/src/lib.rs:371–374`, `3680–3682` | multi-key rotation already ran; then wait / failover |

Docs: `docs/features/mesh-routing.md` (“Multiple keys per provider”).

**Reusable:** in-process pool + atomic cursor; ≥2 only; one intra-provider 429 retry before mesh.  
**Not reusable as-is:** keys are static strings; OAuth needs per-account tokens + independent refresh.

---

## 3. Rotation point: per-request (not per-turn)

**Decision: per completion** (`Provider::complete` / `complete_with`).

- Matches `KeyPool` (mesh already assumes it).
- A turn issues many completions; per-request spreads subscription burn. Per-turn would pin one account for the whole tool loop.
- Manual `--switch` sets store `active` (rotation seed + list UX). Auto-rotation need not rewrite `active` every hop (keyring thrash); optional: advance `active` only on successful failover.

---

## 4. Failover-on-429 vs model-health (no double-retry)

```
complete()
  ├─ pick account A (round-robin) → refresh A if needed → request
  ├─ if RateLimited AND ≥2 accounts:
  │     pick B (cursor advanced) → refresh B → retry ONCE → return B’s result
  └─ else → mesh (bench / wait / cross-model failover)
```

1. **Account rotation first**, inside the OAuth provider (generic helper, not xAI-only).
2. Mesh never sees accounts — extend the existing “multi-key already ran” comment in `run_model_loop` to OAuth.
3. After accounts are limited, one `ProviderError::RateLimited` → existing wait / pin-backoff / same-provider skip / chain (`is_rate_limited`).
4. **One** next-account retry (not N−1), matching `KeyPool`. Cursor keeps spinning on later requests.
5. 401 / entitlement 403: **no** rotate-as-rate-limit; permanent `Auth` (mesh already excludes).

---

## 5. Token refresh

- Each account has its own `OAuthTokens`. Refresh must target **that id**, not only `store.active`.
- Add `store_account_tokens(key, id, tokens)` / `OAuthAccountStore::set_tokens(id, …)`. Today `set_active_tokens` is wrong if the request used a rotated non-active account.
- v1: refresh fail → `ProviderError::Auth`, no account hop. Account hop only on API 429/quota.
- No shared cross-account refresh lock; optional per-account mutex later.

---

## 6. `forge auth <provider> --list`

≥2 accounts:

```
xai-oauth: 3 account(s) · auto-rotation ON (round-robin)
  * alice@x.ai     — access token expires in 45m, refresh present
    bob@x.ai       — …
  (* = manual active / rotation seed; requests rotate across all)
```

Single account: keep today’s one-line “signed in” (rotation OFF). No tokens printed.

---

## 7. Implementation shape

1. **`OAuthAccountPool`** (generic on provider/keyring key): snapshot ids when ≥2; `next() -> (id, tokens)`; `has_rotation()`.
2. **`fresh_access_token_for(provider, id)`** — load, refresh, persist by id.
3. **Shared helper** used by `XaiOauthProvider` (future OAuth providers): pick → call → one 429 retry with next id.
4. Hold `Arc<OAuthAccountPool>` at provider construction (like `KeyPool::from_config`).
5. CLI list text only; rotation always on when ≥2 (no new flags).

**Out of scope v1:** sticky affinity, weights, unhealthy-account bench, MCP-server OAuth multi-account.

---

## 8. Test plan

| Layer | Tests |
|-------|--------|
| Store | per-id update; stable order; ≥2 detection |
| Pool | round-robin wrap; `<2` no rotation |
| Refresh | A refreshes A only; B untouched |
| Provider (httpmock) | A 429 → B ok; both 429 → one `RateLimited`; single account → no second request |
| Classify | 429/quota → rotate; 401/403 → no rotate |
| Mesh | after provider 429, no second OAuth account assumed (same as multi-key) |
| CLI | “auto-rotation ON” iff ≥2; single-account format unchanged |
| Regression | one account ≡ pre-rotation path |

---

## 9. Rollout

1. Config helpers (per-id store + pool) + unit tests.  
2. xAI provider + 429 retry.  
3. CLI list copy + `docs/features/xai-oauth.md`.  
4. Later providers reuse the same helper (no xAI-specific code).
