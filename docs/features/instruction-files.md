# Instruction files

Standing instructions (`AGENTS.md`, `CLAUDE.md` and friends) that Forge carries on every model
request, in the pinned session block (see [context-pipeline.md](context-pipeline.md)). Code:
`crates/forge-core/src/instructions.rs` and `instructions/`.

## What loads

Forge used to inject the first non-empty file of a fixed list, so a project `AGENTS.md` hid the
user's global `~/.claude/CLAUDE.md`, parent directories were never read, and `CLAUDE.local.md`,
`@path` imports, `.claude/rules/` and nested `CLAUDE.md` files did not exist. Every file now
applies, lowest precedence first:

1. **User-global**: `<config>/forge/AGENTS.md`, `~/.forge/AGENTS.md`, Claude Code's
   `CLAUDE.md` (`$CLAUDE_CONFIG_DIR`, else `~/.claude`), then the unconditional rules in
   `<config>/forge/rules/` and Claude's `rules/`.
2. **Project, root down**: for each directory from the filesystem root down to the workspace,
   `.forge/AGENTS.md`, `AGENTS.md`, `CLAUDE.md`, `.claude/CLAUDE.md`, `CLAUDE.local.md`, then the
   unconditional rules in its `.claude/rules/` and `.forge/rules/`. Parents are read like Claude
   Code reads them: every parent, not only up to the repository top. A parent anyone on the
   machine can write to (`/tmp`) is skipped, because a file any local user can create there is not
   an instruction file; the workspace itself is always read.
3. **On first use**: after a `read_file`, or a write to a file, below the workspace root, the same
   files in the directories between the root and that file, and every rule whose `paths:` globs
   match it. These are attached to that tool result (`[instructions] pkg/CLAUDE.md applies to the
   file just used …`) rather than added to the pinned block, so the block and the provider's
   cached prefix behind it do not change mid-session. Dependency and build directories
   (`node_modules`, `vendor`, `third_party`, `.venv`, `venv`, `site-packages`, `target`, `.git`)
   are skipped. A compaction or a rewind may drop that result, so both forget what was delivered
   and the next touch delivers it again.

`--setting-sources` (see [claude-code-parity.md](claude-code-parity.md)) applies here as in Claude
Code: without `user` the global files are skipped, without `project`/`local` the project ones.

When several files load, each is headed `Contents of <path> (<kind>):` and a one-line note says
the later, more specific file wins a disagreement. A single file reads exactly as before.

## Imports and comments

- `@path` anywhere in a file (start of a line or after a space) imports that file, resolved
  relative to the importing file; `@~/…` is under the home directory. The imported file is carried
  right after its importer, headed `imported by <importer>`, and may import further, five levels
  deep at most. A line made only of imports is dropped from the importer's text.
- `@` inside inline code or a fenced block is not an import, and a word like `me@x.md` is not one.
- A project file imports only from inside its repository (or its own directory outside one); the
  user's own global files import from anywhere. No file imports what the built-in secret rules keep
  `read_file` away from (`.env`, keys, `~/.ssh/`, cloud credentials).
- HTML comments (`<!-- … -->`, across lines too) are dropped outside fenced code.
- A file reached twice (symlink, import, identical copy) is carried once.

## Rules

Every `*.md` below `.claude/rules/` or `.forge/rules/` (subdirectories included) is a rule. Without
frontmatter it applies everywhere. With `paths:` it waits for a matching file:

```markdown
---
paths:
  - "src/api/**/*.ts"
---
Validate every request body.
```

`paths` takes a list, a single glob, or a comma-separated list (`src/**/*.{ts,tsx}, lib/**`).
Globs match the path relative to the workspace root, and relative to the directory holding the
rules folder; `*` does not cross `/`, `**` does.

## Freshness

The load records every path it looked at, present or not: candidate files, imported files, rule
directories and rule files. Each turn, one batch of `stat`s on a blocking thread compares their
(mtime, length) with the last load; any difference reloads the whole set, so a file written or
deleted mid-session (or `/init`) is in the next request.

## Claude bridge

A bridged `claude` reads the `CLAUDE.md` family, `.claude/rules/` and their imports itself. Each
file records whether it does (`InstructionFile::claude_loads`), and the claude bridge is sent only
the rest: `AGENTS.md`, `.forge/AGENTS.md`, the Forge-global files and `.forge/rules/`. An
`AGENTS.md` that a `CLAUDE.md` imports (`@AGENTS.md`) or copies (`lean_context::claude_md_covers`)
is left out too. First-use files are attached by Forge's own tool dispatch; bridged turns run
their tools through `forge mcp-serve` and do not get them.

## Tests

`instructions/tests.rs`: global plus project, parents root-down, world-writable parent skipped,
import depth, code and e-mail are not imports, import bounds and secrets, HTML comments,
de-duplication, rules with and without `paths:`, what the claude bridge is sent, setting sources,
the fingerprint, and first-use delivery in a session (again after a compaction, never for
`node_modules`). `tests/pinned_context.rs` covers mid-session edits and legacy sessions.
