# Code archaeology: Core session controls

Session controls mediate mutable state that presenters, CLI surfaces, and later
turns share. Configuration cannot be split into independent adapter state:
model/tier/effort pins participate in route selection; temper changes must
persist to Store and preserve Plan-mode restoration; attached Lattice/MCP tools
capture the session workspace; quota display must use Store-normalized readings
rather than raw bridge caches.

The lifecycle hook runner emits warning notes after injecting the canonical
workspace into every payload. Context insertion persists messages before it
updates the in-memory transcript and audit pack; terminal answer publication
persists the sole user-visible final answer before emitting AssistantDone. These
orders are retained verbatim. Core characterization and endurance tests cover
them through public Session behavior.
