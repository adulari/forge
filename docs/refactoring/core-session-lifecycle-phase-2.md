# Core session lifecycle extraction

This phase moves Session start/resume/build lifecycle construction into private
`session_lifecycle.rs` while keeping the Session state owner and turn program in
the Core root.

## Invariants retained

- fresh sessions create the durable Store row from the canonical workspace;
- resumed sessions use persisted CWD, permission mode, transcript and next
  sequence (not loaded-message count after compaction);
- tool bindings, cached project guidance/branch context, route pricing and
  affinity state are constructed exactly once before the `SessionStarted` event;
- construction remains the sole owner of its initial presenter-event ordering.

No public Session constructor path, Store transaction, compaction behavior,
provider request, permission decision, or presenter event changes.

## Result and verification

Core root implementation lines reduce from 10,656 to 10,478; the private
lifecycle owner is 200 lines. Core warnings-denied Clippy, 507 library tests,
the 3-test long-session endurance suite, formatting, and the architecture guard
passed. No provider benchmark applies to this mechanical construction move.
