# Forge repository guidance

## Overview

Forge is a local-first, model-agnostic AI coding harness and CLI implemented as a Rust Cargo workspace. One session core serves terminal, headless, remote web, mobile, and desktop surfaces; Model Mesh routes provider work, Store persists the audit trail, and Lattice provides local code intelligence.

## Verified checks

Run these from the repository root:

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features
FORGE_DB="$(mktemp -d)/forge.db" cargo test --locked --all --all-features
cargo build --release --locked --bin forge
```

`FORGE_DB` on the test command is not optional — see **Never let a dev build touch the real
store** below. A handful of tests open the default store when it is unset, and doing so has taken
the user's daemon down four times.

The first command was verified during setup. The remaining commands are the repository's documented workspace checks in `CONTRIBUTING.md`; CI runs the same checks with `RUSTFLAGS=-D warnings`. The workspace uses the stable Rust toolchain with `rustfmt` and `clippy` components, and requires Rust 1.88 or newer.

## Architecture

- `crates/` is the Cargo workspace and modular monolith. `forge-cli` is the binary/composition root; `forge-core` owns the session/agent loop and permission broker.
- `forge-types` holds shared domain types; `forge-config` handles layered configuration and secrets; `forge-store` encapsulates SQLite persistence.
- `forge-provider` abstracts model providers; `forge-mesh` performs task routing and failover; `forge-tools` owns coding tools; `forge-lsp` supplies live diagnostics; `forge-mcp` integrates MCP.
- `forge-tui` renders terminal interactions through presenter adapters. Keep the session core's Interaction interface surface-independent.
- `forge-index` implements Lattice code intelligence. Architecture decisions are recorded in `docs/architecture/decisions/`; substantial design changes should add an ADR or RFC.

## Never let a dev build touch the real store

The single most damaging recurring failure in this repository, four times so far
(2026-07-17, twice on 2026-08-06, 2026-08-07). A binary built from a working tree opens
`~/.local/share/forge/forge.db` and runs whatever migrations the checked-out branch carries. The
installed release binary then refuses that store — `SchemaTooNew`, which is correct — and
`forge serve` cannot start. The last occurrence left the daemon at 632 consecutive failed restarts
with Forge Anywhere dark throughout, and recovery needs a hand-written `PRAGMA user_version` write.

Rules:

- **Tests**: always pass `FORGE_DB` pointing somewhere disposable. Never run the suite bare.
- **MCP server entries** pointing at `target/debug/forge` (this repo's own `.mcp.json` and the
  untracked `.forge/mcp.toml`) must set `FORGE_DB` to a dev store. An agent session started on a
  feature branch is otherwise enough to migrate production data.
- **Anything that spawns a child** which may reach a `forge` binary must scrub `FORGE_DB` from the
  child's environment unless the child genuinely shares this store — `claude --print` loads the
  project's `.mcp.json` and can spawn a `forge` grandchild that inherits it.
- Deliberately debugging against real data is fine — set `FORGE_DB` explicitly and know why.

Prevention lives in the code too: a `debug_assertions` build resolves to `forge-dev.db` rather
than the release store, so `cargo run` and `target/debug/forge` are isolated by default. Treat that
as a backstop, not a reason to drop the rules above — a release-profile build ignores it.

## Every feature gets a UI on every surface

Any user-facing capability — a CLI subcommand, a slash command, a config switch, a status
readout — ships with a real UI on all three surfaces in the same change or a linked follow-up
landed before release:

- **TUI**: an interactive overlay or menu (navigable with arrows/Enter/Esc, searchable when the
  list is long, live status, confirmations for destructive actions), not only a slash command that
  prints text. A command may open the UI; it must not be the only way in.
- **Desktop and web**: a page or panel reachable from Settings or the relevant screen, plus the
  command palette.
- **Mobile**: the same page at phone width. A feature that is meaningless on a phone (for example
  storing secrets on a controller-only device) still gets a read-only view that says why.

A surface may be skipped only for a specific, written reason (in the PR body and, when lasting,
in the feature's doc), such as a capability the platform cannot offer. "Can be done from the CLI"
is not a reason. Match the existing design system on each surface (`crates/forge-tui/src/theme`,
`mobile/src/components/ds`) and verify each UI live before merging.

## Conventions

- Keep changes focused and explicit; comments should explain why rather than restate code.
- Add or update tests for new behavior and regressions where practical.
- Route side effects through the permission broker and keep SQLite access inside Store.
- Use branch names `feat/<slug>`, `fix/<slug>`, `refactor/<slug>`, `docs/<slug>`, `chore/<slug>`, `ci/<slug>`, or `perf/<slug>`.
- Use Conventional Commits (`feat:`, `fix:`, `refactor:`, `docs:`, `chore:`, `test:`, `perf:`, `ci:`).
- Do not edit generated or unrelated files. Before submitting, run the applicable formatting, lint, test, and build checks above.
