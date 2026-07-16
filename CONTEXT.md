# Forge Domain Context

Forge is a local-first coding harness. One session core drives terminal, headless, remote web,
mobile, and desktop surfaces while the Model Mesh chooses providers and the Store persists the
audit trail.

## Domain language

- **Session** — one persisted conversation and agent loop, scoped to one workspace.
- **Turn** — one user prompt and its bounded model/tool continuation until a final outcome.
- **Interaction** — surface-independent events, questions, confirmations, and replay items emitted
  by the session core. TUI, headless, and remote presenters are adapters.
- **Remote session protocol** — the versioned snapshot/input contract used by `forge serve`, the
  companion app, the legacy PWA, and `forge attach`.
- **Fleet** — the live set of daemon-hosted sessions, ordered to surface sessions waiting on a
  human before busy and idle work.
- **Temper** — the session's permission posture: Read-only, Ask, Auto-edit, or Full.
- **Model Mesh** — deterministic task classification, provider ranking, budget pressure, health,
  and failover that choose a model and record the rationale.
- **Lattice** — the local code-intelligence graph and retrieval index.
- **Assay** — parallel analysis/review with critic and verification phases.
- **Workflow** — a saved or model-authored script that sequences phases and agent calls.

## Load-bearing decisions

- Preserve the modular monolith and single-binary delivery (ADR-0002).
- The session core owns the Interaction interface; renderers stay adapters (ADR-0004).
- SQLite access remains encapsulated in the Store (ADR-0005).
- Side effects always cross the permission broker (ADR-0008).
- Native iOS push may use the disclosed, opt-out hosted APNs relay (ADR-0012).
