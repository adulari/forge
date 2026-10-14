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
| shell, read_file, edit_file, multi_edit, apply_patch, write_file, append_file, delete_file, search, glob, list_dir, update_tasks, present_plan (Plan mode), use_skill (with a catalog), dispatch_sessions (coordinators), ask_user, spawn_agents, tool_search, tool_call | browser, browser_network, device, device_logs, proxy, proxy_network, web_fetch, web_search, notebook_edit, shell_job, lattice, remember, manage_heartbeats, schedule_wakeup, push_notification, message_session, list_sessions, send_to_agent, list_subagents, cancel_subagent, run_workflow, mcp_list_resources, mcp_read_resource, mcp_get_prompt |

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
failure-loop and doom-loop guards, the env-fight cap, batch eligibility, task scope, the
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
