# Core session program — workspace context phase

This architecture-only Core phase extracts immutable session workspace identity
from the crate root into `workspace_context.rs`.

## Scope

The private owner canonicalizes the session root and exposes root/display access
needed by Core lifecycle code. `forge_core::WorkspaceContext` remains a deliberate
root export. Session orchestration, workspace transition ordering, tool rebinding,
hook behavior, checkpoint handling, persistence, and public callers are unchanged.

## Result

| Measure | Before | After |
|---|---:|---:|
| Core root implementation lines | 10,685 | 10,656 |
| `workspace_context.rs` implementation lines | — | 39 |
| Workspace implementation files ≤500 | 120/185 | 121/186 |
| Workspace implementation files ≤800 | 150/185 | 151/186 |

## Verification

- warnings-denied Core Clippy across targets/features
- Core library suite: 507 passed
- long-session endurance integration: 3 passed
- formatting and architecture guard passed

No model-visible context, event ordering, persistence transaction, permission,
provider, or runtime routing behavior changed; provider benchmarks are not
applicable.
