# Forge repository guidance

## Overview

Forge is a local-first, model-agnostic AI coding harness and CLI implemented as a Rust Cargo workspace. One session core serves terminal, headless, remote web, mobile, and desktop surfaces; Model Mesh routes provider work, Store persists the audit trail, and Lattice provides local code intelligence.

## Verified checks

Run these from the repository root:

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features
cargo test --locked --all --all-features
cargo build --release --locked --bin forge
```

The first command was verified during setup. The remaining commands are the repository's documented workspace checks in `CONTRIBUTING.md`; CI runs the same checks with `RUSTFLAGS=-D warnings`. The workspace uses the stable Rust toolchain with `rustfmt` and `clippy` components, and requires Rust 1.88 or newer.

## Architecture

- `crates/` is the Cargo workspace and modular monolith. `forge-cli` is the binary/composition root; `forge-core` owns the session/agent loop and permission broker.
- `forge-types` holds shared domain types; `forge-config` handles layered configuration and secrets; `forge-store` encapsulates SQLite persistence.
- `forge-provider` abstracts model providers; `forge-mesh` performs task routing and failover; `forge-tools` owns coding tools; `forge-lsp` supplies live diagnostics; `forge-mcp` integrates MCP.
- `forge-tui` renders terminal interactions through presenter adapters. Keep the session core's Interaction interface surface-independent.
- `forge-index` implements Lattice code intelligence. Architecture decisions are recorded in `docs/architecture/decisions/`; substantial design changes should add an ADR or RFC.

## Conventions

- Keep changes focused and explicit; comments should explain why rather than restate code.
- Add or update tests for new behavior and regressions where practical.
- Route side effects through the permission broker and keep SQLite access inside Store.
- Use branch names `feat/<slug>`, `fix/<slug>`, `refactor/<slug>`, `docs/<slug>`, `chore/<slug>`, `ci/<slug>`, or `perf/<slug>`.
- Use Conventional Commits (`feat:`, `fix:`, `refactor:`, `docs:`, `chore:`, `test:`, `perf:`, `ci:`).
- Do not edit generated or unrelated files. Before submitting, run the applicable formatting, lint, test, and build checks above.
