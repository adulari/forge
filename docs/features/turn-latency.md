# Turn latency: what runs before the first token and after the answer

A turn's latency is the work Forge does around the model call, not only the call. This page lists
that work and the rule each piece follows, so a new step does not quietly land in front of the
user again.

## Before the first token

| Step | Where | Rule |
|---|---|---|
| Tokenizer vocabulary (`o200k_base`, ~150 ms of CPU, about 45 MB) | `forge_core::tokens::warm_in_background` at session build for headless runs; `warm_now` at the first input event of the TUI | Built on a background thread; the first count waits for it instead of building a second copy. The TUI does not build it at launch: a session that is opened and left idle never holds the 45 MB, and typing a prompt takes longer than the build. `--continue`, `--resume <id>` and a prompt on the command line start it at dispatch instead, because a resumed session counts its transcript for the gauge while it is built. |
| Codex plan detection (OS keyring read) | `forge_provider::detect_subscription_plans`, prefetched at session build | Same: started at session build, cached 60 s. |
| OpenCode Go / Kimi usage polls | `refresh_polled_quotas`, spawned at session build | Never awaited before routing. The route reads the last stored reading (at most one freshness window old); the poll's result lands for later routes. Awaiting it held every unpinned first token for ~0.9 s (up to its 5 s timeout). |
| LLM tier classification | `LlmRouter::route_with_context` | Skipped when the deterministic floor is already Complex (the floor is a maximum, so no verdict can change the decision). Asked at low reasoning effort. Only the deterministic tier is computed before the call; the model is selected once, for the winning tier. |
| Model selection | `HeuristicRouter::decide_with_pin` / `decide` | A strict single pin builds no failover chain (it would be discarded). The unpinned path ranks the routed tier once and reuses it for the chain. |
| Memory recall embedding | `prefetch_recall_embedding` | Started before routing so it overlaps the route; skipped when the store holds no project or global memories. |
| Rejected embedding model | `forge_provider::embedder` | A model that rejected a request (4xx other than a rate limit) is not asked again in the process. |
| Claude bridge process (`claude` + its `forge mcp-serve` child + the tool handshake, ~1.0-1.5 s) | `Session::prewarm_pinned_bridge`, `Provider::prewarm` | A new conversation pinned to one `claude-cli::` model starts the process at TUI start, while the composer is empty, and a one-shot `forge run` starts it as soon as its session is built, beside the turn's hooks and routing instead of after them. A prompt that arrives mid warm-up waits for that process (the conversation's slot is held and marked warming) instead of spawning a second one. A conversation with history is not warmed: it resumes claude's own session, and the full-transcript prompt a fresh process takes would cost more tokens than the second it saves. |
| Pinned model's context window (a network sweep when the store has never learned it, 0.6-1.1 s) | `pin_window::start_pinned_window_lookup`, `Session::settle_window_lookup` | Starts at session build and holds neither the TUI's first frame nor the first request. The session reads windows from the store each time it sizes a request, so a lookup that lands later applies by itself. What must not happen is a request the 32k floor sizes differently from the real window (a compaction at 25.6k tokens, a trimmed history), so the session waits for the lookup only when the context is past a quarter of the floor's compaction trigger or the request would not fit the floor's trim budget. A first request (system prompt, tools, one prompt) is under both and goes out at once. |
| Model discovery on a machine's first launch (no cached catalog, bounded at 12 s) | `run/session.rs` | Skipped for a pinned, attended session: a pin routes nothing through the catalog and the TUI outlives the detached refresh that fills it in. Unpinned or unattended sessions still wait once, because their first route reads the catalog. |
| Claude rate-limit probe (a real `claude` start plus a model turn) | `run_chat_tui` | Skipped when the session is pinned: a pin bypasses mesh selection, so the reading cannot change what the session does. Otherwise it runs in the background with `--tools "" --setting-sources ""`, so it no longer loads the user's hooks, CLAUDE.md and every tool definition (about 116 KB of request) to ask for one period. |

## Before the first frame and the first prompt

`forge chat` has to open the store, read the keyring, build the provider and router and start the
session before the composer accepts text. On a clean profile with the keyring unlocked (the
benchmark's fresh store, 32 cores, load average 12-20) that took 270-385 ms in the process; most of
it was the same work done more than once. Each row is one step and the rule it follows.

| Step | Where | Rule |
|---|---|---|
| First-run check (`known_key_providers().any(has_api_key)`) | `maybe_first_run_setup`, `needs_onboarding_with` | A saved config settles it with one `stat`. The key scan is a Secret Service round trip per provider on a machine with no key in the environment, about 205-235 ms, and it ran on every `forge chat` start; it now runs only for a machine with no config. |
| Provider secrets (a D-Bus round trip, with a Diffie-Hellman handshake, per provider) | `forge_config::warm_provider_secrets`, started at `forge chat` dispatch; `inject_provider_keys` | Read on a detached thread beside the terminal setup and the store, so the injection that follows finds them cached. The injectors join the warm-up before they touch the environment: setting a variable must not overlap another thread's secret-service call. |
| The store (open, schema batch, migrations, a pool of connections) | `Store::open_shared`, `open_store` after `share_store_handles()` | One file, one pool per chat process. The resume lookup, the presence guard, the session and the pricing table each opened, migrated and pooled the file again (five opens, 15-60 ms and 7-10 MB each). A handle opened while another is alive reuses its connections; the pool closes with the last handle, so the WAL is checkpointed as before. The daemon keeps independent handles. |
| Connections per pool | `Store::build` | Two are opened up front, the rest when a second reader needs one (the pool keeps eight, sixteen when shared). Eight eager connections cost every open their setup and each keeps its own parsed schema and page cache. |
| Daemon reachability probe (builds an HTTP client, asks the daemon) | `run_chat_tui` | Runs beside the first frame instead of in front of the composer; the fleet-messaging tools join the session when the answer is in. |
| Model listings sweep (several MB of JSON parsed into trees, about 100 MB allocated) | `context_windows::fetch_and_persist` | Frees the pages when it returns (`malloc_trim`). Everything was dropped already, but the allocator kept about 65 MB for the life of the session. It runs on a fresh store, for a pinned model nothing knows yet and on a catalog refresh. |

`RUST_LOG=forge::startup=debug` shows each step with its offset from `main` and the resident set
(`first-run check (main+34 ms, rss 28 MB)`, `open_store took 3 ms`), and `memory: 126 -> 60 MB`
lines whenever the resident set changes in the first ten seconds, so a step in memory can be lined
up with the line before it.

### Memory at idle

The Claude bridge runs three processes: the Forge terminal, the bridged `claude` and the `forge
mcp-serve` tool server it starts. `/proc/<pid>/smaps_rollup` fifteen seconds after launch, three
runs per arm (MB):

| Process | Resident before | Resident after | What it held |
|---|---|---|---|
| Forge terminal (`forge chat`) | 176-180 (anonymous 121-125) | 61-63 (anonymous 10-12) | before: the tokenizer vocabulary (45), a store pool per `open_store` call, the leftover pages of the model-listing sweep (65). The shared binary pages (about 48) stay. |
| `claude` | 231-232 | 232-233 | Not Forge's. |
| `forge mcp-serve` | 41-42 (anonymous 6) | 38-39 (anonymous 5) | One store pool and the tool registry. No Lattice, no grammars, no catalog: the tool server is small already. |
| Sum | 450-454 | 333-335 | Claude Code alone reads 280 in one process. |

The sum counts the shared pages of the Forge binary once per Forge process; the proportional set
(`Pss`) sums to 298-302 before and 181-183 after.

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
(~5 s) for a second identical one (`failure_verdict::rejected_model_error`). A provider's explicit
"no such model" answer is permanent and fails the turn on the first call with no wait at all, and
so does a bare id nothing recognises when no local Ollama answers for it (S11: 5.4 to 6.0 s to the
error, now about 0.26 s). See [tui-errors-and-exit.md](tui-errors-and-exit.md).

## The tool server's handshake

Before the prompt is written, the bridge asks the live `claude` for its MCP status until Forge's
tool server reports `connected`. The server answers `initialize` about 45 ms after claude spawns
it and claude asks for the status around then, so the poll that finds it connected landed up to
one poll interval late. The interval was 50 ms and is 10 ms (`MCP_STATUS_POLL` in
`cli_provider/live.rs`). Measured as the time from `bridge: spawn claude` to `bridge: forge mcp
ready`, with claude run without a login so no model call is made (zero quota), arms interleaved:

| run | 50 ms poll (median, range) | 10 ms poll (median, range) |
|---|---|---|
| 12 spawns per arm | 351 ms (282-418) | 298 ms (262-711) |
| 20 spawns per arm | 316 ms (281-448) | 282 ms (255-436) |

What remains of that wait is claude's own start. Forge's share before it spawns claude was the
process start, the keyring injection (`forge::startup: inject keyring keys`, 50-64 ms in a clean
profile and 100-131 ms with a live keyring), config, provider and routing: about 110 ms to
`run_turn_with start` and 55 ms more to the spawn. A one-shot run now prewarms the process when the
session is built, so the routing runs beside claude's start. The keyring injection stays in front:
the tool server adopts the parent's injected keys, so claude cannot start before them, and
widening the prefetch from 8 to 36 threads measured no gain (45 vs 48 ms median, n=10): each lookup
opens its own Secret Service connection.

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
calls. `shell diagnose: detached` marks a failed-command explanation handed to a background
task (it no longer runs inside the tool call), and `post-turn side calls:
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
| shell diagnosis (a model call inside a failed command's tool call; now off the turn) | 0.5% (1.7 s, one call) |
| building the request, the guards, the gaps between steps | 0.4% |
| the last step's model time (it has no `response complete` line) | 5.0% |

Forge's own work between a tool result and the next request is under 10 ms in every step of the
trace; the only Forge-side time that is not the model or a tool is the work after the answer.

What is left after these steps is the model's own time: a bridged `claude` runs with the person's
own `~/.claude` (CLAUDE.md, hooks, a global `effortLevel`), so a prompt it decides to think about
costs seconds of hidden reasoning before the first character. Claude Code started with the same
config takes just as long, so compare the two with the same config on both sides.
