# Core session controls split

This phase isolates Session's runtime control surface into private
`session_controls.rs`: persistent context insertion/terminal answer publication,
attachments, model/effort/tier/mode pins, MCP/Lattice/LSP/skills attachment,
checkpoint-root configuration, lifecycle hooks, persisted view state, usage and
quota projection, and temper persistence.

`Session` remains the only mutable state owner. The extraction preserves
permission-mode persistence, catalog calibration, MCP connection semantics,
workspace-bound lattice watcher behavior, quota freshness/pace publishing, and
presenter event ordering.

Core root implementation lines reduce from 8,725 to 8,134; the owner is 599
lines, a justified cohesive state-control boundary below the 800-line guard.
Core Clippy, 507 Core tests, three long-session endurance tests, formatting,
and the architecture guard passed.
