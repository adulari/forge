# xAI/Grok subscription OAuth (`xai-oauth`)

**Status: experimental (Phase 1).** A second auth path for xAI, alongside the existing API-key
`xai::` provider. Instead of an `XAI_API_KEY`, this signs in with a SuperGrok / X Premium
**account** via an RFC 8628 device-code flow, and bills usage against that subscription instead of
metered API credits.

Modeled on [Hermes'](https://github.com/NousResearch/hermes-agent) `xai-oauth` provider — the
reference implementation this ships from — and the same shared public OAuth client xAI names in
its [OpenClaw announcement](https://x.ai/news/grok-openclaw).

## Using it

```
forge auth xai-oauth
```

prints a verification URL (and a code, if the URL doesn't embed it) and waits for you to approve
the sign-in in a browser. On success it runs a one-shot **entitlement probe** — see below — and
either confirms API access or explains why it can't.

Once signed in, pin a model with the `xai-oauth::` namespace:

```
forge --model xai-oauth::grok-4
```

Other commands:

```
forge auth xai-oauth --list     # session status (token expiry, scopes)
forge auth xai-oauth --remove   # sign out (deletes the stored tokens)
```

## The entitlement gotcha

A successful device-code login proves the **account** signed in — it does not prove xAI's servers
will actually grant that account's subscription tier OAuth API access. xAI enforces this
server-side and can 403 even a genuinely active SuperGrok subscriber. This is not a bug in Forge
and will not fix itself by signing in again.

`forge auth xai-oauth` runs a tiny probe request right after login specifically to catch this and
say so plainly, instead of silently retrying. If inference later 403s despite the probe having
passed (entitlement can change), the error tells you to run `forge auth xai` and use an API key
instead — the same fallback the probe itself suggests on a 403.

## Cost & context accounting

`xai-oauth::` models report as `$0` cost (usage is billed to the subscription, not per-token) but
are **not** treated as free in the mesh's free/paid split — `forge_mesh::catalog::is_subscription`
marks them, same as the claude-cli/codex-cli bridges. Unlike those bridges, though, `xai-oauth` is
a normal single-request API call (not an internal multi-step agent loop), so its reported
`input_tokens` is accurate and the context gauge uses it directly — `is_cli_bridge` deliberately
excludes `xai-oauth::` for this reason (see `forge_provider::is_cli_bridge`'s doc comment).

## Security

The OAuth bearer token is only ever attached to a request built from a hardcoded
`https://api.x.ai/v1` base — never a custom-provider endpoint, env override, or user-supplied base
URL. See `forge_provider::xai_oauth::is_pinned_xai_url` and the module's security-invariant doc
comment.

Tokens (access + refresh) live in the OS keyring under `provider-oauth:xai` — a namespace distinct
from both the API-key `xai` provider and any MCP server named `xai`
(`forge_config::provider_oauth`).

## Deferred / out of scope (Phase 1)

- Model auto-discovery for `xai-oauth` — pin `xai-oauth::<model>` explicitly.
- `web_search` / `x_search` / `code_execution` server-side tool passthrough on the Responses API.
- Mesh-routing auto-selection of `xai-oauth` models as a default tier — they participate only when
  explicitly pinned.
- A dedicated Forge OAuth client id from xAI. Phase 1 ships the same public client id Hermes and
  OpenClaw use; a Forge-specific registration is a follow-up, not a blocker.
- `forge setup`/`forge init` wizard integration beyond the `provider_label` string.
- Subscription quota/window reporting — xAI exposes no queryable window for this.
