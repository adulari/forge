# Feature: approvals learn

After you approve the same command shape a few times in a project and never refuse it, the next
prompt for that shape offers a standing rule: "You've allowed `cargo test *` 3x here. Always allow
in this project?" One tap writes it through the same plumbing as "always allow" and the prompt is
gone on every surface.

## What is recorded

Every answered permission prompt writes one row to `permission_decision` (migration 52): project
(a worktree counts as its checkout), tool, shape, decision (`allow`, `session`, `always`, `deny`)
and time. Offers add `suggestion_accepted` / `suggestion_declined` rows.

## Shapes (`forge-core/src/permission/learn.rs`)

- Shell: the program, plus the subcommand for `cargo`, `git`, `npm`/`pnpm`/`yarn`/`bun` (`run
  <script>`), `go`, `make`, `just`, `uv`, `rustup`, then ` *`. The rule written is the same pair
  "always allow" writes (`cargo test *` and `cargo test`).
- No shape (so no offer) for anything with `| ; & < > $ ( ) { }`, backticks, a backslash or a
  newline; a leading `VAR=value`; programs that escalate, delete, fetch, ship data out or run
  another interpreter (`sudo`, `rm`, `curl`, `ssh`, `bash`, `python`, `docker`, ...); `git` outside
  `status diff log show add commit fetch blame`; `cargo publish/install/login/...`; `npm
  publish/exec/...`; and anything the safety floor, a deny rule or the `auto` risk classifier flags.
- Other tools: the tool name, only for in-workspace `Write` tools. `python`, MCP tools and a public
  `shell_job expose` never get one.

## When it is offered

The shape has at least 3 allows in the project since its last deny or declined offer, and no
existing rule (allow or ask) matches the call. A deny resets the count, and so does letting an
offer pass: the shape has to earn it again.

## Surfaces

- TUI: a fifth choice under the prompt, key `r` (or `5`).
- Daemon / phone / desktop: `permission_suggestion: {rule, allows}` on the snapshot (additive); the
  permission card shows a secondary button with the rule text, which sends `allow` with
  `scope: "rule"`. A host that never offered the rule ignores that scope.
- `forge attach` (the plain line client), headless and print presenters do not show the offer and
  never count as having declined it.
