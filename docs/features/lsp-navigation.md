# Feature: `lsp` navigation tool

> Status: **v0, opt-in** (`[harness] lsp_nav = true`, default off). Code:
> `crates/forge-lsp/src/{nav,position}.rs`, `crates/forge-tools/src/lsp_tool.rs`, wiring in
> `crates/forge-cli/src/cli/commands/run/lsp_nav.rs`.

## What it does

A deferred, read-only tool `lsp` with three ops:

| op | answers |
|---|---|
| `definition` | where the symbol is defined; the body is inlined for library code |
| `hover` | the symbol's type, signature and docstring |
| `references` | every use, grouped by file, at most 40 rows |

Arguments are `op`, `path`, `line` (1-based), `symbol` (the identifier exactly as written on that
line) and an optional `occurrence`. Never a column: Forge finds the whole-word match on that line
(or within three lines of it) and converts to the UTF-16 column the server wants. A dotted symbol
such as `json.dumps` lands on its last segment.

The first output line is a header, `[lsp:ty] definition of ...`; failures start with
`[lsp:unavailable]`, `[lsp:timeout]` or `[lsp:error]` and say what to use instead.

## Why library lookups matter

`read_file` cannot reach site-packages or the stdlib. A `definition` that resolves outside the
workspace returns up to 40 lines (2.5 KB) of the declaration's body, after the same path rules that
guard `read_file` have allowed the path. A non-file location (bundled typeshed) is printed and
nothing is inlined.

## Servers and roots

- v0 serves Python only (`NAV_LANGS`). ty and pyright analyse source without executing project
  code, so a read-only tool may start them. rust-analyzer and gopls run cargo/go tooling that
  honours repo config; they need a `project_trust::is_trusted` gate before they can join.
- Server choice: `[lsp.servers.python]` if set, else the first of `ty server`,
  `pyright-langserver --stdio`, `basedpyright-langserver --stdio` found on `PATH`.
  `forge doctor` prints the resolved choice.
- Root: the nearest project marker (`pyproject.toml`, `package.json`, `.git`, ...) when it lies
  inside the session workspace, otherwise the workspace itself. Benchmark task directories have no
  markers.
- ty resolves the interpreter's system packages (`/usr/lib/python3/dist-packages`,
  `/usr/local/lib/python3.x/site-packages`) from `PATH` with no venv; checked in Terminal-Bench
  images.

## Budgets and safety

- 10 s for `definition`/`hover`, 15 s for `references`, covering lock wait, spawn, initialize,
  re-sync and the request. On the deadline Forge sends `$/cancelRequest`.
- Total output is capped at 6 KB. Paths are workspace-relative.
- Files changed on disk by shell commands or scripts are re-sent to the server before each call.
- One analyzer process runs at a time (`MAX_LIVE_SERVERS = 1`) and is shared with post-edit
  diagnostics. Do not lower `lsp.idle_timeout_secs` expecting more capacity.
- Path rules: the built-in secrets deny covers `lsp`. A rule you wrote for `read_file` does not
  apply to the `lsp` tool's `path` argument unless its tool is `lsp` or `*`; inlined source is
  checked against your `read_file` rules regardless.
- Subagent children do not get the tool, and the CLI bridge does not register it.

## Not in v0

`rename_symbol`, `symbols`, other languages, a Lattice fallback, prewarm, a prompt line, `[lsp]`
nav knobs, and LRU eviction of the single live-server permit (needed before this can default on).
Promote the tool to the always-advertised set with `mesh.lean_core_extra = ["lsp"]`.
