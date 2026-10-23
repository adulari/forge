# Feature: deferred tools — a lean tool surface on every path

> Status: **SHIPPED** (2026-10-14), on by default on the direct path. Code:
> `crates/forge-core/src/deferred_tools.rs` (the shared registry: core set, catalog, search,
> `tool_call` unwrap), `crates/forge-core/src/tool_dispatch/deferred.rs` (the direct path's surface,
> `tool_search` and the batch check), `crates/forge-cli/src/mcp_serve/deferred.rs` (the bridge's
> `rmcp::Tool` adapter). Tests: `crates/forge-core/src/tests/deferred_tools.rs`,
> `crates/forge-core/src/tests/deferred_dispatch.rs`.

## 1. Problem

Every request resends every advertised tool schema. On the direct path (every non-bridge
provider) that was about 32 tools and 42K characters (roughly 10K tokens) per request, most of
it platform tooling (device, browser, proxy, heartbeats, fleet messaging) that almost no session
calls. The CLI bridge already had a lean surface (`mesh.bridge_lean`); the direct path did not.
On a heavily cached cloud route the dollar saving is small. The real win is context headroom on
small windows and weak-cache routes (ollama, nvidia, opencode_go), where tools were about 40% of
a 32K window.

## 2. What ships

The direct path advertises the core tools plus two meta-tools:

| Advertised | Deferred (reached through `tool_search` / `tool_call`) |
|---|---|
| shell, read_file, edit_file, multi_edit, apply_patch, write_file, append_file, delete_file, search, glob, list_dir, update_tasks, present_plan (Plan mode), use_skill (with a catalog), dispatch_sessions (coordinators), worktree ([session-worktrees.md](session-worktrees.md)), ask_user, spawn_agents, run_code (codemode), tool_search, tool_call | browser, browser_network, device, device_logs, proxy, proxy_network, web_fetch, web_search, notebook_edit, shell_job, lattice, remember, manage_heartbeats, schedule_wakeup, push_notification, message_session, list_sessions, send_to_agent, list_subagents, cancel_subagent, run_workflow, open_pr, mcp_list_resources, mcp_read_resource, mcp_get_prompt |

With subagents enabled and no skills, lattice or MCP, the surface goes from 32 tools / 42,356
characters to 16 tools / 16,688 characters.

- **`tool_search{query}`**: its description lists every deferred tool with a short gloss (the
  first sentence of its description, capped at 64 bytes), sorted by name. The result gives each
  match's full description, input schema and an example call built from the required fields,
  e.g. `tool_call{"arguments":{"url":"…"},"name":"web_fetch"}`. Exact names win, else keywords.
  With MCP servers connected it also searches their catalog (`server__tool` names).
- **`tool_call{name, arguments}`**: runs the named tool. `arguments` may also arrive as a JSON
  string or under `args`/`input`; a missing one means `{}`. A `server__tool` name goes to the MCP
  manager through `mcp_call`.
- **Calling a deferred tool directly** by name still works; nothing is removed from dispatch.
- **`mcp_search_tools` / `mcp_call`** are no longer advertised on the direct path (the pair
  above replaces them) but stay callable as aliases indefinitely, for old transcripts and skills.

### `tool_call` is the inner call everywhere

The model loop turns each `tool_call{name, arguments}` into the real call (same id) **before**
anything looks at it. The phantom-edit gate's `mutations_ran`, the verification ledger, the
progress ledger, the env-fight cap, batch eligibility, task scope, the
permission broker, hooks, the audit row and the presenter all see `notebook_edit`, never
`tool_call`. A deferred write through `tool_call` is still denied in Plan mode, and a successful
one still counts as a mutation. The tool result keeps the original id, so it pairs with the
assistant message the model sent. `invoke_tool` resolves again on entry (idempotent), so no
caller can bypass the unwrap.

## 3. Configuration

```toml
[mesh]
direct_lean = true               # default; false restores the full surface (kill switch)
lean_core_extra = ["web_fetch"]  # deferred tools to keep advertised
```

`FORGE_DIRECT_LEAN=0` / `1` overrides the config either way. The bridge keeps its own switch
(`mesh.bridge_lean`, `FORGE_BRIDGE_LEAN`) and its measured core set, which leaves `ask_user` and
`spawn_agents` out.

## 4. Not changed (yet)

- `present_plan` is still advertised only in Plan mode, and Plan mode still appends its shell note
  to `shell`'s description. Advertising it in every mode would stop the tools block changing on a
  mode switch, but weak models may misfire it in Build mode; that waits for a bench run.
- The bridge still lists its deferred tools through the shared catalog but does not fold
  external MCP tools into `tool_search`: there `mcp_search_tools`/`mcp_call` are deferred tools.
  Its `tool_call` does use the shared unwrap, so string-encoded or `args`/`input` arguments
  work the same on both paths.
- `tool_search` does not run in the concurrent read-only batch; it is a local lookup.

## 5. Measuring it

A/B with the same pinned model: `FORGE_DIRECT_LEAN=0` vs default under `forge bench`. Compare
input tokens per attempt, cache hit rate, resolve rate and how often `tool_search` was called.

## Bridge: `shell_job` is deferred, so `shell` points at `tool_call`

`shell` starts a background job and says the job is managed with `shell_job`. On the bridge's lean
surface `shell_job` is deferred, so a model that took that sentence literally called
`mcp__forge__shell_job` and got "No such tool available" (one haiku run, task d05). `forge mcp-serve`
now rewrites, on the lean surface only, `shell`'s description and the "Check on it with shell_job:"
line of a started job's result to say the call goes through `tool_call {"name": "shell_job", ...}`
(`crates/forge-cli/src/mcp_serve/deferred.rs`; the shared hint text is `forge_tools::JOB_HINT`, so a
reworded result fails a test instead of silently skipping the rewrite). The full surface
(`mesh.bridge_lean = false`) lists `shell_job` and is left as it was.

Five other bridged calls failed with "could not be parsed as JSON". Those never reached Forge:
claude rejected the model's own tool input, a raw tab (three `multi_edit` calls on Go and Makefile
text) or raw newline (two `shell` `commands` calls) inside a JSON string of a nested array. The
`edits` and `commands` parameter descriptions now begin with the escape reminder, which survives the
lean surface's first-sentence cut; whether that lowers the rate is not measured.

## Bridge: the Codex preamble spells the core tools

Codex lists MCP tools as deferred: its code mode (`functions.exec`) reaches them as
`tools.mcp__forge__<name>` but names them only in `ALL_TOOLS`. The preamble used to tell the model
to dump every Forge description before it started (`ALL_TOOLS.filter(name.startsWith("mcp__forge__"))`):
17.7 KB for the 17 tools of the lean surface. In a 52-task baseline on `codex-cli::gpt-6-luna` that
was one model call in 50 sessions (64 of 739 calls ran the dump) and then about 4.7k tokens in every
later request, an estimated 15% of the arm's input tokens.

The lookup is not needed to call a tool. Against Codex's own Responses stub (no quota),
`tools.mcp__forge__list_dir({path: "."})` runs in the first script and answers
`{"content": [{"type": "text", "text": "..."}], "isError": false}`. The preamble now spells the
arguments of the 12 core tools in about 1 KB and says three more things the dump never did: print
`r.content.map(c => c.text).join("\n")` and never `JSON.stringify(r)` (which escaped every quote and
newline of every result into the context), never `return` from a script, and wait for a yielded cell
with one `wait` of a large `yield_time_ms` instead of a model call every 30 s. Tools off that list
(`present_plan`, `use_skill`, `worktree`) are read by exact name from `ALL_TOOLS`; the deferred
ones (`web_search`, `web_fetch`, `spawn_agents`) go through `tool_search` and `tool_call`.
`the_codex_preamble_signatures_match_the_served_tools` in `forge-cli` fails when a signature names an
argument the served schema does not have.

Thirteen tasks on the build with this change against the same thirteen in each baseline seed
(`results/runs/eff-fx-after1`, `codex-a2`), input counted from Codex's own rollout files:

| | baseline seed 1 | baseline seed 2 | this change |
|---|---|---|---|
| input tokens | 4.91M | 5.59M | 2.99M |
| model calls (per task) | 187 (14.4) | 204 (15.7) | 138 (10.6) |
| tool output into the context | 0.59 MB | 0.63 MB | 0.31 MB |
| full tool-list dumps, other `ALL_TOOLS` lookups | 13, 3 | 13, 3 | 2, 5 |
| failed `exec` scripts | 9 | 5 | 1 |
| `d04-long-build-await`: calls, wall | 27, 440 s | 27, 645 s | 9, 212 s |

The build includes the neighbouring bridge fixes (no `run_code` guidance, the review carrying its
evidence, the resumed-turn accounting), so the columns are the combined effect; the dump itself
accounts for the share above. The model still dumped the whole list in 2 of 13 tasks, the
preamble notwithstanding.

Wall time needs a concurrent control (the machine's load moved between the runs), so the stack and
the baseline binary ran six of those tasks at the same time, one run each (`results/runs/eff-fx-ab-*`):

| six tasks, concurrent | baseline binary | with the stack |
|---|---|---|
| input tokens (rollout files) | 2.76M | 1.79M |
| input tokens as Forge stored them | 6.15M | 1.79M |
| model calls | 95 | 74 |
| agent wall: sum, median | 532 s, 79.7 s | 413 s, 66.4 s |
| solved | 5 of 6 | 6 of 6 |

One run per task per arm is a small sample on tasks whose outcome itself varies between seeds: the
13-task run solved 8 against the baseline seeds' 11 and 10, with failures on tasks (`a08`, `a09`,
`b02`, `d01`) that also fail or flip at baseline.
