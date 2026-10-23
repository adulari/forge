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
| Claude bridge process (`claude` + its `forge mcp-serve` child + the tool handshake, ~1.0-1.5 s) | `Session::prewarm_pinned_bridge`, `Provider::prewarm` | A new conversation pinned to one `claude-cli::` model starts the process at TUI start, while the composer is empty. A prompt that arrives mid warm-up waits for that process (the conversation's slot is held and marked warming) instead of spawning a second one. A conversation with history is not warmed: it resumes claude's own session, and the full-transcript prompt a fresh process takes would cost more tokens than the second it saves. |
| Pinned model's context window (a network sweep when the store has never learned it, 0.6-1.1 s) | `pin_window::start_pinned_window_lookup`, `Session::settle_window_lookup` | Starts at session build and never holds the TUI's first frame. The session reads windows from the store each time it sizes a request, so a lookup that lands later applies by itself; the one thing that must not happen is a first request sized from the 32k floor, so the turn waits for the lookup right before sizing it. By then a person has typed a prompt and the wait is zero. |
| Model discovery on a machine's first launch (no cached catalog, bounded at 12 s) | `run/session.rs` | Skipped for a pinned, attended session: a pin routes nothing through the catalog and the TUI outlives the detached refresh that fills it in. Unpinned or unattended sessions still wait once, because their first route reads the catalog. |
| Claude rate-limit probe (a real `claude` start plus a model turn) | `run_chat_tui` | Skipped when the session is pinned: a pin bypasses mesh selection, so the reading cannot change what the session does. Otherwise it runs in the background with `--tools "" --setting-sources ""`, so it no longer loads the user's hooks, CLAUDE.md and every tool definition (about 116 KB of request) to ask for one period. |

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

## After an interrupt

Esc on a bridged turn abandons the live `claude` process, and the next prompt replaces it. The
teardown used to sleep the whole 2 s kill grace after SIGTERM even when the process was already
gone, so the first prompt after Esc paid 2 s before its replacement could spawn. It now waits for the
process group to exit, up to the same grace, and only then sends SIGKILL.

## Measuring

`RUST_LOG=forge::startup=debug` prints one line per startup phase. `RUST_LOG=forge::turn=debug`
prints one line per phase between Enter and the first streamed token (turn spawned, routed, request
dispatched, `bridge: spawn claude`, `bridge: forge mcp ready`, `bridge: prompt written`, `bridge:
first line from claude`, first stream event, first text delta); the log's own timestamps give the
deltas. In the full-screen TUI the log is `.forge/forge.log` in the project. For a headless turn,
time the stream-json events: `system/routing`, the first `assistant` text, and `result`.

Every model-loop step also prints its own boundaries, so a slow turn splits into the model's time
and Forge's: `step N: request` (the request is being built), `dispatch <model>`, `first stream
event`, `step N: response complete`, then `tools start` and `tools done` around the step's tool
calls. `shell diagnose: start` / `done`
wrap the failed-command explanation that runs inside a tool call, and `post-turn side calls:
start` / `done` wrap the recap, suggestion and memory capture after the answer. The final answer's
step has no `response complete` line: it ends the loop, so its model time is the gap before
`post-turn side calls: start`.

Five `inferhub::cb/deepseek-v4.1-flash` tasks on the native loop (`forge bench tasks`, one seed,
`RUST_LOG=forge::turn=debug`; a sixth, whose test run hung for 776 s on a busy machine, is left
out), as a share of the time from `run_turn_with start` to the end of the turn (314 s, 44
steps):

| phase | share |
|---|---|
| model, dispatch to response | 74.0% |
| tools | 11.1% |
| recap, suggestion and memory capture after the answer | 9.0% (4.8 to 8.2 s a task) |
| shell diagnosis (a model call inside a failed command's tool call) | 0.5% (1.7 s, one call) |
| building the request, the guards, the gaps between steps | 0.4% |
| the last step's model time (it has no `response complete` line) | 5.0% |

Forge's own work between a tool result and the next request is under 10 ms in every step of the
trace; the only Forge-side time that is not the model or a tool is the work after the answer.

What is left after these steps is the model's own time: a bridged `claude` runs with the person's
own `~/.claude` (CLAUDE.md, hooks, a global `effortLevel`), so a prompt it decides to think about
costs seconds of hidden reasoning before the first character. Claude Code started with the same
config takes just as long, so compare the two with the same config on both sides.
