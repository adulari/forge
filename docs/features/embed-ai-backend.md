# Feature: embed Forge as your app's AI backend (`forge api`)

> **Status (shipped):** `forge api` — an opt-in local HTTP server
> (`crates/forge-cli/src/api_serve.rs`) exposing an **OpenAI-compatible** chat-completions
> endpoint backed by Forge's model mesh. Point any OpenAI-compatible client's `base_url` at
> `http://<host>:<port>/v1` and every request gains tier-based model selection, cross-provider
> failover, subscription quota-spread, and cost tracking — with **no code change** beyond the
> base URL. Nothing here runs unless `forge api` is invoked; it changes no default behavior.

## Why

Most apps wire their AI to a single provider: one `base_url`, one API key, one model. That's a
single point of failure (the provider rate-limits or goes down), a single cost profile (no cheap
model for cheap work), and a migration cost every time you want to try another model.

Forge already solves this internally with its **mesh**: it classifies each task's difficulty,
routes it to the best usable model for that tier, fails over across providers when one is
rate-limited or down, spreads load off near-exhausted subscriptions, and prices every call. `forge
api` exposes that mesh behind the endpoint every LLM SDK already speaks — OpenAI's
`/v1/chat/completions`. Swap one base URL and a single-model integration becomes a multi-model,
self-healing one.

## Quickstart

1. **Start the server** (loopback by default):

   ```bash
   forge api                       # http://127.0.0.1:8787/v1
   forge api --port 9000           # pick a port
   forge api --host 0.0.0.0        # accept LAN / container traffic
   forge api --api-key $SECRET     # require Authorization: Bearer $SECRET
   ```

   It prints the base URL, the auth mode, and where to list models.

2. **Point any OpenAI client at it.** Only the `base_url` (and, if you set one, the key) changes:

   ```python
   from openai import OpenAI
   client = OpenAI(base_url="http://127.0.0.1:8787/v1", api_key="unused")  # any key when --api-key unset
   r = client.chat.completions.create(
       model="auto",                                  # let the mesh route
       messages=[{"role": "user", "content": "Summarise this in one line: ..."}],
   )
   print(r.choices[0].message.content)
   ```

   ```bash
   curl http://127.0.0.1:8787/v1/chat/completions \
     -H 'content-type: application/json' \
     -d '{"model":"auto","messages":[{"role":"user","content":"hello"}]}'
   ```

3. **Pick models via the mesh.** `GET /v1/models` lists everything the mesh can route to plus the
   `auto` sentinel:

   - `"model": "auto"` (or `"mesh"`, or omit it) → the mesh classifies the request and routes it,
     with automatic failover down its ranked chain.
   - `"model": "anthropic::claude-opus-4-8"` (any concrete Forge id) → pins that model, but the
     mesh **still fails over** to alternatives if the pinned model is rate-limited or down.

## Endpoints

All under the server's origin; the OpenAI base URL is `http://<host>:<port>/v1`.

| Method & path              | Purpose                                                                 |
| -------------------------- | ----------------------------------------------------------------------- |
| `GET  /health`             | Liveness probe → `{"status":"ok"}`. No auth.                            |
| `GET  /v1/models`          | `{"object":"list","data":[{"id","object":"model","owned_by":"forge"}]}` — routable models + `auto`. |
| `POST /v1/chat/completions`| One chat completion. `stream:true` → OpenAI SSE chunks. |

### Request fields (an OpenAI-compatible subset)

- `messages` (**required**) — `system` / `user` / `assistant` / `tool` roles. `content` may be a
  string or an array of content parts (only text parts are used today).
- `model` — `auto`/`mesh`/omitted for mesh routing, or a concrete Forge id to pin (see above).
- `stream` — `true` to receive `text/event-stream` `chat.completion.chunk` frames ending in
  `data: [DONE]`.
- `temperature` — forwarded to the model.
- `reasoning_effort` — `low` / `medium` / `high` (also accepts Forge's `xhigh` / `whitehot`).
- `tools` — OpenAI function specs. Advertised to the model; any `tool_calls` it makes are returned
  with `finish_reason:"tool_calls"`, so your app runs its own tool loop exactly as with the OpenAI
  API.

### Response

Standard OpenAI `chat.completion` (or `chat.completion.chunk` when streaming), plus a non-standard
`x_forge` object for routing/cost visibility (strict OpenAI clients ignore unknown fields):

```jsonc
{
  "object": "chat.completion",
  "model": "groq::llama-3.3-70b-versatile",
  "choices": [{ "index": 0, "message": { "role": "assistant", "content": "..." }, "finish_reason": "stop" }],
  "usage": { "prompt_tokens": 41, "completion_tokens": 12, "total_tokens": 53 },
  "x_forge": { "routed_model": "groq::llama-3.3-70b-versatile", "rationale": "...", "cost_usd": 0.00002 }
}
```

## What you get from the mesh (for free, via the base URL)

- **Multi-model routing** — the request's difficulty tier decides the model; cheap work goes to
  cheap models, hard work to strong ones (`docs/features/mesh-classifier.md`,
  `docs/features/provider-cost-routing.md`).
- **Cross-provider failover** — a rate-limited or down model transparently falls over to the next
  ranked candidate; for streaming this is transparent as long as no tokens were emitted yet
  (`docs/features/model-health-failover.md`).
- **Subscription quota-spread** — near-exhausted subscription plans are demoted so you don't
  overrun a window (`docs/features/provider-cost-routing.md`).
- **Auto-discovery** — with `[mesh] auto_discover` on, the mesh routes across every usable model it
  can enumerate from your configured providers, not just built-in defaults
  (`docs/features/auto-discovery-mesh.md`).
- **Cost tracking** — every response carries `x_forge.cost_usd`, priced from token counts.

## Auth & exposure

- **Auth** — set `--api-key <KEY>` or the `FORGE_API_KEY` env var to require
  `Authorization: Bearer <KEY>` on `/v1/*`. Unset ⇒ open, intended for loopback or a trusted
  private network. `/health` is always open.
- **Binding** — `--host 127.0.0.1` (default) is loopback-only; `--host 0.0.0.0` accepts LAN and
  container traffic (e.g. a Dockerised app reaching the host via `host.docker.internal`). The
  server speaks **plain HTTP** — terminate TLS and gate public exposure with a reverse proxy in
  front (nginx/Caddy/Cloudflare). It never sees your provider logins; it uses whatever keys /
  subscriptions your local Forge config already has.
- **Coexistence** — `forge api` (default port 8787) is independent of `forge serve` (the remote
  daemon, port 7420); both can run at once.

## Implementation

- `crates/forge-cli/src/api_serve.rs` — the axum router, the OpenAI request/response mapping, the
  routing + failover loop, and SSE streaming. Tests cover the completion shape, `tool_calls`
  surfacing, SSE framing, `/v1/models`, the 400/401 error shapes, and content-part flattening.
- Routing reuses the production wiring: `build_provider_and_router` (the same
  `DispatchProvider` + `HeuristicRouter` `forge run` builds) and `HeuristicRouter::route`, so the
  endpoint's routing behavior is identical to an interactive session's.
- CLI: `forge api [--host] [--port] [--api-key] [--mock]`.
