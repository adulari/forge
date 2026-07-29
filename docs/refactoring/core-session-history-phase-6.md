# Core session history split

This architecture-only phase extracts session rewind/undo, checkpoint context,
checkpoint listing, visible and full replay projection, compaction reload,
workspace transition, and fresh/resumed reset lifecycle into private
`session_history.rs`.

The module preserves one deep Session owner for the coupled state transition:
DB sequence ↔ compacted transcript offsets, snapshot restoration ordering,
workspace/tool/lattice rebinding, cached project guidance refresh, and
`SessionStarted`/task presenter ordering all remain unchanged.

Core root implementation lines reduce from 9,019 to 8,725; the extracted owner
is 302 lines. Core Clippy, all 507 Core tests, 3 long-session endurance tests,
formatting, and the architecture guard passed. No persistence transaction,
replay, context compaction, workspace isolation, or session-affinity behavior
changed.
