# Turn latency: what runs before the first token and after the answer

A turn's latency is the work Forge does around the model call, not only the call. This page lists
that work and the rule each piece follows, so a new step does not quietly land in front of the
user again.

## Before the first token

| Step | Where | Rule |
|---|---|---|
| Tokenizer vocabulary (`o200k_base`, ~150 ms of CPU) | `forge_core::tokens::warm_in_background`, started at session build | Built on a background thread while startup continues; the first count waits for it instead of building a second copy. |
| Codex plan detection (OS keyring read) | `forge_provider::detect_subscription_plans`, prefetched at session build | Same: started at session build, cached 60 s. |
| OpenCode Go / Kimi usage polls | `refresh_polled_quotas`, spawned at session build | Never awaited before routing. The route reads the last stored reading (at most one freshness window old); the poll's result lands for later routes. Awaiting it held every unpinned first token for ~0.9 s (up to its 5 s timeout). |
| LLM tier classification | `LlmRouter::route_with_context` | Skipped when the deterministic floor is already Complex (the floor is a maximum, so no verdict can change the decision). Asked at low reasoning effort. Only the deterministic tier is computed before the call; the model is selected once, for the winning tier. |
| Model selection | `HeuristicRouter::decide_with_pin` / `decide` | A strict single pin builds no failover chain (it would be discarded). The unpinned path ranks the routed tier once and reuses it for the chain. |
| Memory recall embedding | `prefetch_recall_embedding` | Started before routing so it overlaps the route; skipped when the store holds no project or global memories. |
| Rejected embedding model | `forge_provider::embedder` | A model that rejected a request (4xx other than a rate limit) is not asked again in the process. |

## After the answer

The result (`Done`, the stream-json `result` event) is emitted before any optional side call:

- **One-shot `forge run`** makes no recap, suggestion or memory call: nobody reads a recap after
  the process exits, and a memory capture held the exit for a model round trip.
  `mesh.auto_memory_one_shot = true` keeps the capture (it runs after the result is printed).
- **TUI** detaches recap, suggestion and memory onto the presenter's sink; input frees at once.
- **Synchronous surfaces** (`forge mcp agent`) run them inline after the result. The MCP agent
  presenter consumes neither recaps nor suggestions, so those calls are skipped there.

## Unknown model ids

A pinned id that no discovered catalog lists gets a startup warning. If the provider then refuses
it quickly, the turn fails on that first refusal instead of waiting the first outage backoff step
(~5 s) for a second identical one (`failure_verdict::rejection_warning`).

## Measuring

`RUST_LOG=forge::startup=debug` prints one line per startup phase. For a turn, time the stream-json
events: `system/routing`, the first `assistant` text, and `result`.
