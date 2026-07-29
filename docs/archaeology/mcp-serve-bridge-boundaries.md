# Code archaeology: bridge budget and bridge subagents

## Boundaries

Two owners split out of `mcp_serve.rs`, both defined by the same fact — on the CLI-bridge path the
*caller* is claude or codex running its own loop:

- `mcp_serve/bridge_budget.rs` — what the bridge is allowed to spend. Result caps for `read_file`
  and `shell` (mirroring the caps those CLIs apply to their own tools), UTF-8-safe head/tail
  clamping, the lean tool surface, the hard cap on the advertised skill list, and the external-MCP
  connect gate with its env override.
- `mcp_serve/subagents.rs` — `spawn_agents` / `send_to_agent` with no presenter: resolving a child
  among the parent session's persisted children, rebuilding its transcript, and reporting progress
  out of band through the sink.

`mcp_serve.rs` keeps the server: the tool list, `call_tool` dispatch through the permission
broker, and the stdio/HTTP transports.

## Why the budget is a domain, not a constant

Every tool schema, description, and result crosses the bridge again on every turn of the bridged
CLI's loop. A cost paid once on the direct path is paid per turn here, which is why these caps
exist at all and why they are deliberately bridge-only — the direct path keeps forge-tools' own
(much larger) caps. Grouping them makes that asymmetry legible instead of looking like arbitrary
magic numbers scattered through the server.

## Interfaces

Both modules are `pub(super)`; the subagent handlers stay inherent methods on `ForgeMcp`, so
`call_tool` dispatch is unchanged, and `SubagentSupport` is re-exported for the server's own
construction.

## Characterization

The cap tests (pass-through, head/tail markers, shell advice, multibyte safety), the external-MCP
gate truth table, and the skill-description cap moved with their code; the lean-surface and
dispatch tests stay with the server. Full `mcp` test selection passes.
