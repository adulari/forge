# Code archaeology: Tools workspace confinement

## Summary

`WorkspaceTool` is the security-critical adapter that roots relative arguments,
rejects path and symlink escapes, scopes task-local CWD, and retargets tools
atomically during a Core workspace transition. It is not authorization: the
central Core permission broker remains ADR-0008's sole side-effect gate. This
extraction gives confinement its own private owner without altering tool
schemas or public registry behavior.

## Timeline

- **2026-07-20, `b8e30871` / PR #770:** added per-session workspace isolation
  after daemon sessions could otherwise use ambient process CWD.
- Earlier tool and sandbox work established local validation as defence in depth
  below Core's centralized permission decision.

## Invariants

- Every scoped tool shares a mutable canonical binding; a rebind updates all
  registered and later-registered tools atomically.
- Relative `path`, `cwd`, and batch `paths` root at the session workspace.
- Explicit default paths apply only to tools where an omitted path is valid.
- Preview and execution share the same containment validation and task-local
  workspace.
- Confinement never authorizes a side effect or bypasses Core permission rules.

## Evidence

Focused tests cover default injection, late registration/rebinding, peer and
traversal rejection, existing/nonexistent symlink escapes, and preview
confinement. Core integration/endurance tests exercise transitions and
workspace isolation through the session seam.
