# ChatGPT subscription OAuth (`codex-oauth`)

**Status: experimental.** A native `codex-oauth::` provider so a ChatGPT Plus/Pro subscription
backs Forge turns **directly** — no `codex` CLI bridge. OpenAI permits subscription OAuth in
third-party tools (unlike Anthropic/Google, whose ToS ban it; their CLI bridges stay).

The ChatGPT Codex backend API (`chatgpt.com/backend-api/codex/*`) is **undocumented** and may
drift; treat this path as best-effort. Design: `docs/design/codex-oauth.md`.

## Using it

```
forge auth codex-oauth
```

starts an OAuth 2.0 PKCE flow: opens a browser (or prints the URL) and listens on
`http://localhost:1455/auth/callback` (the official Codex public client redirect — the redirect
URI must be `localhost`, byte-exact, or OpenAI's Hydra authorize server rejects the request; the
loopback listener itself still binds `127.0.0.1:1455`). On success tokens are stored in the OS
keyring under `provider-oauth:codex`.

Headless / no browser:

```
FORGE_NO_BROWSER=1 forge auth codex-oauth
```

prints the authorize URL; open it on a machine that can reach this host’s port **1455**. Port
1455 is fixed by the public client — free it if another Codex/Forge auth holds it.

Once signed in:

```
forge --model codex-oauth::gpt-5.5
```

Other commands:

```
forge auth codex-oauth --list
forge auth codex-oauth --switch --account <id>
forge auth codex-oauth --remove
forge auth codex-oauth --remove --account <id>
```

## Multiple accounts

Re-running `forge auth codex-oauth` **adds** another account (labeled from the JWT
`chatgpt_account_id` / email when present, else `account-N`). With ≥2 accounts Forge
auto-rotates round-robin per completion and hops once on 429 or connection-level Unavailable
(same as `xai-oauth` / `docs/design/oauth-account-rotation.md`).

## Discovery

`forge models` only probes `codex-oauth` when a session is stored. There is no public `/models`
list on the ChatGPT backend — Forge seeds:

- `gpt-5.5`, `gpt-5.4`, `gpt-5.3-codex`, `gpt-5.2`, `gpt-5.4-mini`

## Cost & routing

`codex-oauth::` models report **$0** cost (subscription-billed) and are marked **subscription**
in the mesh (`is_subscription`), not free and not a CLI bridge (`is_cli_bridge` is false — tool
calls surface to Forge’s own loop).

## Security

Bearer tokens are only attached to host-pinned `https://chatgpt.com/...` URLs. Tokens live in the
keyring (`provider-oauth:codex`), never in config/logs (ADR-0007).

## Not in scope

- Claude / Antigravity OAuth (ToS forbid it — keep `claude-cli` / `agy-cli` bridges).
- Replacing the `codex-cli::` harness path.
- ChatGPT server-side tool passthrough.
