# Core orchestration split

This phase moves delegated work orchestration into private `orchestration.rs`:
parallel subagent spawning and event drainage, persistent child follow-up,
workflow execution/saved workflow persistence, and duel candidate lifecycle.

The module keeps one Session-owned coordinator for child session links, route
choices, per-provider concurrency, workspace inheritance, child audit rows,
workflow event ordering, cancellation, and presenter results. No child protocol,
permission boundary, parent transcript, Store transaction, or scheduler
behavior changes.

Core root implementation lines reduce from 7,388 to 6,738; the orchestration
owner is 658 lines, under the 800-line guard. Core Clippy, all 507 Core tests,
three long-session endurance tests, formatting, and the architecture guard
passed.
