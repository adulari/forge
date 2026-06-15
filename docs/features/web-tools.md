# Feature: Web tools (`web_search` + `web_fetch`)

Status: building (Wave 2, P1). Closes the largest capability gap by usage evidence —
WebSearch 373 + WebFetch 187 = **560** uses in the owner's history; the agent today cannot
research the web at all.

## Problem (JTBD)

> When I'm coding and hit something I don't know (an API, an error, a library version), I
> want the agent to **search the web and read a page** inline, so it can ground its answer in
> current facts instead of stale training knowledge.

Forge's tool set is read/write/edit/list/search/shell — all local. No network reach. A 2026
coding harness that can't look anything up is crippled for real work.

## Scope (MoSCoW)

- **Must:** a `web_fetch` tool (keyless: GET a URL → clean text) and a `web_search` tool
  (BYOK: query → ranked results). Both gated by a new `Network` side-effect class. SSRF guard
  on fetch. Pluggable search backend; Brave Search as the reference backend.
- **Should:** `forge auth brave` to store the search key in the OS keyring; clear actionable
  error when `web_search` runs with no key configured.
- **Could:** additional search backends (Tavily/Serper/SearXNG) behind the same trait.
- **Won't (this iteration):** rendering JS pages, crawling/recursion, a `forge init` step for
  the search key (documented follow-up), caching.

### Non-goals
This feature does not execute page JavaScript, does not crawl beyond the single fetched URL,
and does not cache responses.

## Network side effect (permission model)

Network egress is a distinct effect class from a local read (SSRF, data exfiltration, cost).
Add `SideEffect::Network`. Mode mapping (`decide_mode`):

| Mode | Network | Rationale |
|------|---------|-----------|
| `Plan` | **Deny** | read-only contract = no side effects, incl. egress |
| `Default` | **Ask** | confirm the first network call; an allow-rule pins it |
| `AcceptEdits` | **Allow** | auto-runs reads/edits; a network read is low-risk |
| `Bypass` | **Allow** | "do anything" |

The FR-10 rules engine layers on top: a user can `allow`/`deny` `web_fetch`/`web_search` by
host/query pattern regardless of mode (e.g. deny `web_fetch` to internal hosts).

## `web_fetch` (keyless)

- Args: `{ "url": string, "max_chars"?: number }` (default cap ~10000 chars of extracted text).
- GET via `reqwest` (rustls), bounded redirects, request timeout, body size cap, a Forge
  User-Agent. HTML → plain text: drop `<script>/<style>`, strip tags, decode common entities,
  collapse whitespace; prepend the `<title>` when present.
- **SSRF guard** (`is_safe_url`): scheme must be `http`/`https`; reject literal private,
  loopback, link-local, and unique-local IPs and `localhost`/`*.local` hostnames. Known limit
  (documented): no DNS resolution, so DNS-rebinding to a private IP is not caught in v1.

## `web_search` (BYOK, pluggable)

- Args: `{ "query": string, "count"?: number }` (default 5, capped at 10).
- `SearchBackend` trait → `BraveSearch` reference impl. **Verified Brave contract** (official
  docs, 2026-06): `GET https://api.search.brave.com/res/v1/web/search?q=…&count=…`, header
  `X-Subscription-Token: <key>`, results at `web.results[].{title,url,description}`.
- Key resolution: env `BRAVE_API_KEY` first, then OS keyring entry `brave`
  (`forge auth brave`). No key → `ToolError` with a one-line fix hint, never a panic.
- **Pricing note (medium confidence, secondary sources):** Brave removed its free tier in
  early 2026 — now metered (~$0.003–0.005/query, $5 prepaid). The trait keeps us
  backend-agnostic so a free backend can be added later.

## Acceptance criteria

```
AC1  Given the registry,  Then web_fetch and web_search are registered, both SideEffect::Network.
AC2  decide_mode: Network → Plan Deny, Default Ask, AcceptEdits Allow, Bypass Allow.
AC3  is_safe_url rejects http://127.0.0.1, http://localhost, http://10.0.0.1,
     http://169.254.169.254, file://…, ftp://…; accepts https://example.com.
AC4  html_to_text strips tags + script/style and decodes &amp;/&lt;/&gt;/&quot;; title is surfaced.
AC5  parse_brave_results(sample) → ordered [{title,url,description}] from web.results[].
AC6  web_search with no backend + no BRAVE_API_KEY → Err with an actionable "set a key" message.
AC7  web_search with a mock backend → formatted, numbered results (title / url / description).
```

## Impact

| Layer | File | Change |
|------|------|--------|
| Types | `forge-types` SideEffect | add `Network` variant |
| Permission | `forge-core::permission::decide_mode` | map `Network` per the table |
| Tools | `forge-tools` (new `web.rs`) | `WebFetchTool`, `WebSearchTool`, `SearchBackend`/`BraveSearch`, pure `is_safe_url`/`html_to_text`/`parse_brave_results`; register in `with_core_tools` |
| Deps | workspace + `forge-tools/Cargo.toml` | `reqwest` (rustls, json) — already in the lock via genai |
| Config | `forge-config` | `known_search_providers()` (`brave`), `inject_search_keys()`; `forge auth` accepts search providers |
| CLI | `forge-cli` | call `inject_search_keys()` in `build_session_with`; `forge auth brave` |

## Definition of done

- [ ] Both tools registered + `Network` side effect; mode mapping tested (AC1–AC2).
- [ ] SSRF guard + HTML→text + Brave parse are pure, unit-tested (AC3–AC5).
- [ ] No-key path returns an actionable error; mock-backend path renders results (AC6–AC7).
- [ ] `forge auth brave` stores to keyring; key injected before a session runs.
- [ ] clippy -D warnings + fmt clean; full workspace green.
