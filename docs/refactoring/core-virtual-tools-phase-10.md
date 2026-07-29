# Core session virtual-tools split

This phase moves Session-owned virtual tool execution into private
`session_virtual_tools.rs`: `ask_user`, task updates and durable task state,
plan proposal/approval/temper restoration, on-demand memory, and skill loading.

The owner preserves the central distinction between model-facing virtual tools
and registry/MCP dispatch. Task state is persisted before its presenter update;
plans become active tasks only after Build; plan cancellation restores the
captured permission mode; memory and skill calls retain Store audit records and
model-visible guidance ordering.

Core root implementation lines reduce from 6,738 to 6,363; the owner is 387
lines. Core Clippy, 507 Core tests, three long-session endurance tests,
formatting, and the architecture guard passed.
