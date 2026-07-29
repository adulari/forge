# Code archaeology: Core session workspace context

## Summary

`WorkspaceContext` is the immutable canonical filesystem identity of a Session.
It prevents daemon-hosted sessions from accidentally using the process CWD,
which is load-bearing for tool paths, checkpoints, hooks, worktree transitions,
persistence metadata, and session isolation. The extraction moves this cohesive
owner into a private Core module while retaining the root export.

## Timeline

- **2026-07-20, `b8e30871` / PR #770:** introduced explicit workspace isolation
  because daemon sessions can serve different worktrees concurrently.
- Later Core hardening and long-session work retained workspace transitions and
  persisted cwd as session lifecycle invariants.

## Invariants

- Construction canonicalizes and rejects non-directories once per session
  binding; callers never infer a workspace from ambient process state.
- All metadata and tool/hook/checkpoint paths use the same canonical root.
- Workspace transitions rebind tools and refresh project metadata explicitly.
- Public callers retain `forge_core::WorkspaceContext`; only Core internals use
  the narrow crate-private display helper.

## Evidence

Core tests cover fresh sessions after ambient CWD deletion, distinct daemon
workspace metadata, workspace-transition handling, peer-repository rejection,
hook CWD propagation, and long-session isolation/endurance.
