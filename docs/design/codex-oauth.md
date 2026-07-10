# Design: native ChatGPT subscription OAuth (`codex-oauth`)

**Status:** design only (`feat/codex-oauth`)  
**Goal:** native `codex-oauth::` provider so ChatGPT Plus/Pro backs Forge turns directly (no `codex` CLI bridge). OpenAI permits subscription OAuth in third-party tools; Anthropic/Google do not — their CLI bridges stay; **do not** design claude/antigravity OAuth.

**Template:** mirror `xai-oauth` (`crates/forge-provider/src/xai_oauth.rs`, `docs/features/xai-oauth.md`, `docs/design/oauth-account-rotation.md`).

---

## 1. Auth flow (PKCE + local callback; headless fallback)

OpenAI uses **OAuth 2.0 authorization-code + PKCE** (not xAI device-code). Verified:

| Piece | Value |
|-------|--------|
| Authorize | `https://auth.openai.com/oauth/authorize` |
| Token | `https://auth.openai.com/oauth/token` |
| Public client id | `app_EMoamEEZ73f0CkXaXp7hrann` |
| Official codex CLI callback | loopback port **1455** |

**Reuse pure PKCE:** `Pkce` / `authorize_url` / `random_state` (`crates/forge-config/src/oauth.rs:76–148`). Token exchange mirrors MCP OAuth (`crates/forge-mcp/src/oauth.rs:161–177` `exchange_code`, `183–214` `refresh_token`).

**Primary UX (interactive TTY):**

1. Bind `http://127.0.0.1:1455/auth/callback` (match official CLI redirect).
2. Open browser to authorize URL (S256 PKCE + state).
3. Callback validates `state`, exchanges `code` → access + refresh + expiry.
4. Decode access-token JWT payload **without signature verification** (same as `extract_email_from_id_token`, `provider_oauth.rs:197–212`); account id = `chatgpt_account_id`. Label store id with email/sub when present, else `account-N` (`next_provider_oauth_account_id`, `provider_oauth.rs:164–166`).
5. Persist via `add_provider_oauth_account` (`provider_oauth.rs:140–145`); optional entitlement probe like `probe_entitlement` (`xai_oauth.rs:196–218`).

**Headless / no browser / port busy:**

| Situation | Behaviour |
|-----------|-----------|
| No TTY / no display / `FORGE_NO_BROWSER=1` | Print full authorize URL; still run loopback listener; open URL from a machine that can reach this host’s `:1455` |
| Port 1455 taken | Clear error — free 1455; **do not** invent alternate ports (redirect fixed by public client) |
| Listener unreachable from browser | Run `forge auth codex-oauth` on a machine with a browser; no remote paste-code protocol in v1 |
| Device-code | **Not offered** — this client is PKCE/loopback only |

CLI: `forge auth codex-oauth` with `--list` / `--switch --account` / `--remove` like `auth_xai_oauth` (`local.rs:84–147`, `dispatch.rs:156–160`).

---

## 2. Token storage + refresh by account id

| Piece | Location / rule |
|-------|-----------------|
| Keyring key | `provider-oauth:codex` via `provider_oauth_keyring_key("codex")` (`provider_oauth.rs:127–129`) — distinct from API-key `openai` and MCP `mcp-oauth:*` |
| Store shape | Shared `OAuthAccountStore` v2 (`oauth.rs:219–224`) |
| Active load/store | `load_provider_oauth_tokens` / `store_provider_oauth_tokens` (`provider_oauth.rs:134–138`) |
| Per-id refresh | `load_provider_oauth_account_tokens` + `store_provider_oauth_account_tokens` (`provider_oauth.rs:169–180`); `set_tokens` (`oauth.rs:257–264`) so rotated non-active accounts don’t clobber active |
| Expiry skew | 120s (`xai_oauth.rs:39` `REFRESH_SKEW_SECS`) |
| Secrets | ADR-0007; tests under `test-secrets` only (`secret_store.rs:19–24`) |

On 401: refresh that account once, persist by id, retry same request once; refresh fail → permanent `Auth` (no hop — `should_hop_account` excludes Auth, `xai_oauth.rs:290–292`).

---

## 3. Request / streaming — shared vs provider-specific

**Endpoint:** `POST https://chatgpt.com/backend-api/codex/responses`  
**Headers:** `Authorization: Bearer <access>`, **`ChatGPT-Account-Id: <id>`**, `Accept: text/event-stream`.  
**Body:** OpenAI Responses schema (map like `build_responses_request`, `xai_oauth.rs:304–380`).

### Shared (new `forge-provider/src/oauth_responses.rs`)

| Piece | Today | Share |
|-------|-------|-------|
| SSE frame parse | `parse_sse_frame` / `take_event` (`xai_oauth.rs:492–520`) | yes |
| Event fold | `apply_sse_event` + `ResponseAccumulator` (`xai_oauth.rs:384–489`) | yes |
| Connect/idle timeouts | 60s / 90s (`xai_oauth.rs:41–42`) | yes |
| Phantom-truncation | `execute_responses_request` tail (`xai_oauth.rs:850–860`) | yes |
| One-hop rotation shell | `complete_with` + `should_hop_account` (`xai_oauth.rs:890–920`, `290–292`) | yes |
| `AccountSource` seam | Keyring \| Memory (`xai_oauth.rs:527–530`) | yes |
| `cost_usd: 0.0` | `xai_oauth.rs:464` | yes |

### Provider-specific (`codex_oauth.rs`)

Host pin to `chatgpt.com` (mirror `is_pinned_xai_url`, `xai_oauth.rs:54–62`); PKCE login + JWT account id; `ChatGPT-Account-Id` header; ChatGPT entitlement 403 copy; seed models / `codex-oauth::` strip. Wire like xAI in `DispatchProvider` (`lib.rs:424–439`, `496–497`, `530–531`).

---

## 4. Model catalog + tier mapping

Namespace `codex-oauth::<model>` (normalize `codex_oauth::` like `xai_oauth::` at `lib.rs:46–48`).

**Seed** (no public `/models` — same static table as codex CLI, `cli_provider.rs:101–107`): `gpt-5.5` (flagship), `gpt-5.4`, `gpt-5.3-codex` (coding), `gpt-5.2`, `gpt-5.4-mini` (fast).

Discovery only if session stored (`models.rs:223–228` pattern). Extend `is_subscription` (`catalog.rs:50–55`) with `codex-oauth::`. **Not** `is_cli_bridge` (`lib.rs:56–63`) — normal tool loop. KEYLESS (`run.rs:765`). Cost $0.

---

## 5. Rotation / failover reuse

Reuse `OAuthAccountPool` (`oauth.rs:440–520`) + `docs/design/oauth-account-rotation.md` + stall hop (`docs/design/pinned-outage-resilience.md` §2): ≥2 accounts → round-robin per completion; one hop on `RateLimited` or connection-level `Unavailable`; never hop on permanent `Auth`; mesh sees one error after hop.

---

## 6. Error classification

Mirror `classify_xai_status` (`xai_oauth.rs:253–274`):

| HTTP | Error | Notes |
|------|-------|-------|
| 401 | Auth (after refresh+retry) | re-login hint |
| 403 entitlement | Auth permanent | Plus/Pro required; `forge auth openai` for API key |
| 429 | RateLimited | hop once if pool |
| 5xx | Unavailable | hop once if pool |
| other 4xx | Request | no hop |

---

## 7. Risks + mitigations

| Risk | Mitigation |
|------|------------|
| Undocumented `chatgpt.com/backend-api/codex/*` drift | Loose SSE matching; seed models; experimental docs; host pin |
| Client id / redirect revocation | Document public-client dependency; fail auth clearly |
| Port 1455 conflict | Explicit error; no silent port change |
| JWT claim rename | Probe claim aliases; fallback `account-N` |
| ToS confusion | Docs: OpenAI-only; keep claude-cli bridge |

---

## 8. Test plan (`test-secrets` guard)

| Layer | Tests |
|-------|--------|
| Pure | JWT claim extract; host pin; classify; `should_hop_account` |
| Store | multi-account; per-id refresh leaves sibling (`xai_oauth.rs:1360–1405`) |
| HTTP (httpmock) | SSE ok; 401→refresh→retry; 429/503 A→B; single no hop; 403 no hop; `ChatGPT-Account-Id` asserted |
| Catalog | `is_subscription("codex-oauth::gpt-5.5")`; not free; not cli_bridge |
| Guard | `AccountSource::Memory` + `test-secrets` (`secret_store.rs:19–24`, tripwire `191–195`) |

---

## 9. Rollout order

1. Config constants (`CODEX_OAUTH_KEYRING_PROVIDER = "codex"`) + claim/pin unit tests.  
2. Extract shared `oauth_responses` from xAI (behaviour-neutral).  
3. `CodexOauthProvider` + httpmock rotation/401.  
4. `forge auth codex-oauth` (PKCE listener + list/switch/remove).  
5. `DispatchProvider` + discovery + `is_subscription` / KEYLESS / normalize.  
6. `docs/features/codex-oauth.md` (experimental); keep `codex-cli::` bridge.

**Non-goals:** claude/antigravity OAuth; replacing `codex-cli` harness; ChatGPT server-tool passthrough; quota-window API (none stable).
