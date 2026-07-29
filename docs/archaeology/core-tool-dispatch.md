# Code archaeology: Core tool dispatch

ADR-0008 makes Core's permission broker the sole side-effect chokepoint. Native
and MCP dispatch carry the same surrounding guarantees: hooks may block/rewrite
and inject context; runtime Always permits only the requested tool; snapshots
precede writes; audit rows and transcript tool results are durable and ordered;
post hooks receive the actual result; failure/doom-loop feedback reaches the
model after the result.

MCP meta-wrapper dispatch has a second inner-tool permission gate so an external
per-tool rule cannot be bypassed through `mcp_call`. Read-only batching is only
safe when every call is a registry ReadOnly action and no hooks are installed;
its results are deliberately persisted in original model order.

The extraction retains these bodies together in one private stateful owner.
Core permission, hook, concurrency, MCP, failure-loop, and endurance tests
characterize the behavior.
