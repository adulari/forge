# Core tool dispatch split

This phase extracts the 747-line direct/MCP tool dispatch owner into private
`tool_dispatch.rs`. It retains the end-to-end order for every tool call:
task-scope check, loop guard, presenter start, hook rewrite/block/injection,
permission decision and runtime "always", snapshot, execution, audit record,
presenter result, durable tool transcript row, post-hook, and failure feedback.

Read-only batch dispatch remains in the same owner because it has the same
observable ordering contract while concurrently executing only side-effect-free
registry tools.

Core root implementation lines reduce from 8,134 to 7,388. The owner is 754
lines, below the 800-line guard and deliberately deep rather than forwarding.
Core Clippy, 507 Core tests, 3 long-session endurance tests, formatting, and
the architecture guard passed.
