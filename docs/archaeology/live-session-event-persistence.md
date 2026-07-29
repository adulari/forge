# Code archaeology: live session event persistence

## Summary

Remote event replay and live MCP-agent presence are one ephemeral durable-state boundary. They are now isolated from the store root while preserving per-session ordering, bounded retention, session-local stale-flag recovery, and top-level session filtering.

## Boundary and invariants

`live_session_store.rs` owns append/read of remote protocol events and the durable active-agent flag. Event ids remain the replay cursor, reads are ascending after an exclusive cursor, and a durable per-session counter amortizes pruning while retaining the latest 2,000 events plus at most 255 writes across Store handles. Startup clears only the entering session's stale flag, preserving other live agents. Active-agent listings exclude subagent child sessions.

## Interface as test surface

`mcp_live_observer_events` characterizes append order, cursor filtering, active-flag transitions, and cleanup. Long-run and remote reconnect tests exercise the same event log through CLI/daemon surfaces.

## Leave alone

- Event append ordering is database id order.
- Cursor replay is strictly `id > after_id`.
- Retention pruning remains amortized rather than scanning every append.
- Presence flags are reset on MCP server startup after abnormal termination.
- Only top-level active sessions appear in observer listings.
