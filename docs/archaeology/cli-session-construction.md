# Code Archaeology: CLI session construction

## Summary

`run.rs` accumulated session construction because every CLI surface originally built the same `Session` directly. The construction path is now a cohesive boundary shared by one-shot runs, interactive chat, MCP agent mode, and daemon-hosted sessions. Moving that boundary is safe only if its startup ordering and self-MCP recursion protection remain unchanged.

## Timeline

- `2a4fbb7c` introduced the shared session builder and provider/tool assembly.
- `9252bd58` added MCP startup after session construction so connection state can use the presenter.
- `230f6864` added the `allow_self_mcp` boundary after recursive `forge mcp agent` spawning caused an observed fork bomb.
- `6c63abf1` broadened that protection to persisted, renamed self-server entries.
- `e3708644` added an explicit session working directory for daemon-hosted sessions.
- `43519393` and `fed8dcce` constrained Codex quota probing to sessions whose routing can use it, avoiding unnecessary subscription use and startup latency.
- `70ecdb87` prevented resuming sessions frozen by an Anywhere handoff.

## Key decisions explained

### Session startup ordering

**What it does:** validates Anywhere handoff state, injects credentials, loads configuration, normalizes the model pin, refreshes routing pressure, assembles providers and tools, restores or starts the session, then attaches watcher/MCP/LSP integrations.

**Evidence:** the commits above and the ordering in `build_session_with_self_mcp`.

**Why:** routing and persistence state must exist before optional background integrations, while slow watcher and MCP setup must not gate UI startup.

**Still applies?** Yes.

### Self-MCP filtering

**What it does:** MCP agent mode removes a persisted stdio server only when it invokes the current executable with both `mcp` and `agent`, regardless of the configured server name.

**Evidence:** `230f6864` and `6c63abf1` document an observed unbounded process-spawn chain and the persisted-config bypass.

**Why:** checking only the server name or only suppressing dynamic injection leaves a fork-bomb path.

**Still applies?** Yes. Characterization tests now make this policy an explicit interface surface.

### Background integrations

**What it does:** lattice watcher setup and MCP connection run asynchronously after the session is usable.

**Evidence:** watcher commits `21eac57f` through `3f7d9a83` and MCP commits `14a020d8`/`71b77ae7`.

**Why:** remote filesystems and unavailable MCP servers previously blocked startup.

**Still applies?** Yes.

## What's safe to change

- Move the complete construction boundary into a dedicated owner without reordering its operations.
- Extract pure startup policies, such as recursive self-MCP filtering, and test them directly.
- Keep call sites using the same two builder entry points so one-shot, TUI, MCP, and daemon sessions cannot drift.

## What to leave alone

- Do not split provider/tool assembly into pass-through wrappers merely to reduce line count.
- Do not make watcher or MCP connection synchronous.
- Do not weaken Anywhere handoff validation, workspace rooting, model-pin normalization, quota refresh gating, or recursive self-MCP filtering.

## Questions still open

The construction owner remains below the 500-line canonical target after extraction. Further subdivision is not justified unless a future integration develops an independently testable lifecycle or policy boundary.
