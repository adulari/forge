# ADR-0022: A read-only LSP navigation tool, Python first

- **Status:** Accepted (v0, behind `[harness] lsp_nav`, default off)
- **Date:** 2026-10-31
- **Deciders:** Forge maintainers

## Context

forge-lsp only produced post-edit diagnostics. Models answered "where is this defined" and "who
calls this" with grep and `read_file`, and could not read an installed library's source at all
(`read_file` stops at the workspace). A navigation tool was designed twice before; the shipped scope
is the part that survived critique.

## Decision

- One deferred `ReadOnly` tool, `lsp`, with `definition`, `hover` and `references`. The model gives
  path, line and the identifier; Forge computes the UTF-16 column.
- Python only (`NAV_LANGS`), through `ty server` with pyright fallbacks. These do not execute project
  code, which is what makes a ReadOnly call safe. Other languages need a project-trust gate first.
- Navigation reuses the diagnostics server pool and its single live-server permit, so the idle
  timer, memory guard and failure backoff apply unchanged. A nav root without a project marker is
  the session workspace.
- A definition outside the workspace inlines up to 40 lines, gated by the session's `read_file` rules.
- Every request carries a deadline and is cancelled on expiry; an empty answer never clears failure
  state (the 386-restart lesson in `ServerEntry::clear_failure`).

## Consequences

- With the flag off nothing is registered and the pool is built only for `lsp.enabled`.
- Default-on needs LRU eviction of the single permit holder so one session's idle server cannot
  starve others in the daemon, and a measured win on navigation-heavy tasks.
- `rename_symbol`, `symbols`, other languages and prewarm are deferred.
