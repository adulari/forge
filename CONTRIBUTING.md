# Contributing to Forge

Thanks for your interest in Forge — a fast, model-agnostic AI coding harness and CLI written in
Rust. Forge is a post-2.0 project with actively shipped CLI, TUI, desktop, mobile, and web surfaces;
this document covers the workflow, the repo layout, and the quality bar for contributions.

## Prerequisites

- **Rust** matching `rust-toolchain.toml` (currently the stable channel; MSRV `1.88`).
- A C toolchain (for bundled native deps like `rusqlite` and the tree-sitter grammars).
- On Linux, `pkg-config` plus ALSA development headers are needed only for microphone-enabled or
  `--all-features` builds. The default CLI/TUI build deliberately has no ALSA runtime dependency.
- Git. No API keys are needed to build or run the test suite.

## Repository layout

Forge is a Cargo workspace under `crates/`:

| Crate | Responsibility |
| --- | --- |
| `forge-types` | Shared domain types (messages, usage, routing, permissions) |
| `forge-config` | Layered config + secret resolution (keyring / encrypted-file) |
| `forge-store` | SQLite persistence (sessions, messages, costs, decisions) |
| `forge-skills` | Slash-command + skill catalog (discovery, frontmatter, templates) |
| `forge-index` | Lattice — tree-sitter code-intelligence graph |
| `forge-mesh` | Model Mesh: rule-based task routing (cost × capability) |
| `forge-provider` | Provider-agnostic model interface (backed by genai) |
| `forge-tools` | Tool trait + core coding tools (read/write/shell/search) |
| `forge-lsp` | LSP client for live diagnostics after edits |
| `forge-mcp` | MCP client (connect Forge to external MCP servers) |
| `forge-tui` | Presenter abstraction + ratatui renderers |
| `forge-core` | Session orchestrator: the agent loop + permission broker |
| `forge-cli` | The `forge` binary (composition root + subcommands) |
| `xtasks` | Dev tasks (benchmarks, `gen-dist` for completions/man page); not published |
| `vendor/genai-0.6.5` | Independently publishable `forge-agent-genai` fork; excluded from the root workspace and checked with its own lockfile |

Architecture decisions live in `docs/architecture/` (ADRs under `decisions/`); designs and RFCs in
`docs/rfcs/` and `docs/features/`. Forge is design-first — substantial changes get an ADR or RFC.

## Development workflow

1. **Fork & branch.** Create a topic branch off `main`. Never commit directly to `main`.
2. **Branch naming:** `feat/<slug>`, `fix/<slug>`, `refactor/<slug>`, `docs/<slug>`,
   `chore/<slug>`, `ci/<slug>`, `perf/<slug>`. Example: `feat/model-mesh-router`.
3. **Conventional Commits.** `feat:`, `fix:`, `refactor:`, `docs:`, `chore:`, `test:`, `perf:`,
   `ci:` — see [Conventional Commits](https://www.conventionalcommits.org/).
4. **Keep it green.** Run the local checks below before pushing. CI must pass to merge.
5. **Open a PR** into `main`, filling out the PR template. One approving review + green CI are
   required. PRs are squash-merged to keep `main` linear.

## Branching & release model

- `main` — always releasable, branch-protected. Squash-merge only, linear history.
- topic branches — short-lived, one logical change each, deleted after merge.
- release tags — `vMAJOR.MINOR.PATCH` ([SemVer](https://semver.org/)) cut from `main`. A maintainer
  dispatches `.github/workflows/release.yml` from protected `main` with that existing tag; it builds Linux (x86_64 + aarch64),
  macOS (Apple Silicon + Intel), and Windows, then opens one checksummed manifest PR for Homebrew,
  AUR metadata, and Scoop (`Formula/forge.rb`, `packaging/aur/PKGBUILD`, and `bucket/forge.json`). It
  dispatches the five-target desktop and static-web release workflows against the exact tag. Matching `forge-agent*` crates are
  then published in dependency order using [`docs/RELEASING-crates.md`](docs/RELEASING-crates.md).
- Public-surface stability rules are in [`docs/STABILITY.md`](docs/STABILITY.md).

## Local checks (run before every push)

These are the root-workspace checks run by CI:

```bash
cargo fmt --all -- --check                                 # formatting
cargo clippy --locked --all-targets --all-features         # lints (CI runs with -D warnings)
cargo test --locked --all --all-features                   # tests (no API keys required)
cargo build --release --locked --bin forge                 # release-profile smoke
scripts/check-linux-runtime-deps.sh target/release/forge    # Linux: glibc/libstdc++ ceiling + no ALSA
```

The publishable `genai` fork is intentionally outside the root workspace, and the shared Expo/Tauri
app has independent gates. Run them when preparing a release (and whenever that surface changes):

```bash
cargo fmt --manifest-path vendor/genai-0.6.5/Cargo.toml -- --check
cargo clippy --locked --manifest-path vendor/genai-0.6.5/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path vendor/genai-0.6.5/Cargo.toml
(cd mobile && npm ci && npm run check && npx --no-install expo export -p web)
cargo test --locked --manifest-path mobile/src-tauri/Cargo.toml
cargo clippy --locked --manifest-path mobile/src-tauri/Cargo.toml --all-targets -- -D warnings
```

CI additionally runs supply-chain checks (`.github/workflows/security.yml`): `cargo audit`
(RUSTSEC advisories) and `cargo deny check` (licenses + bans + sources, configured in `deny.toml`).
To run them locally:

```bash
cargo install --locked cargo-audit --version 0.22.2
cargo install --locked cargo-deny --version 0.20.2
cargo audit --deny warnings \
  --ignore RUSTSEC-2024-0436 --ignore RUSTSEC-2024-0320 --ignore RUSTSEC-2025-0141
cargo deny check
cargo audit --file vendor/genai-0.6.5/Cargo.lock --deny warnings --ignore RUSTSEC-2024-0436
cargo deny --manifest-path vendor/genai-0.6.5/Cargo.toml check
cargo audit --file mobile/src-tauri/Cargo.lock --ignore RUSTSEC-2024-0429
```

Rust CodeQL, the mobile web export, Tauri checks, security checks, and the vendored fork all run on
every PR so branch protection can require stable check names even when a change is outside their
directory.

## Code standards

- Comments explain **why**, not what. No comments where the code is self-evident; no docstrings on
  trivial functions.
- Prefer explicit over clever.
- New behaviour ships with tests. Bug fixes ship with a regression test where practical.
- Architecture-affecting changes update `docs/architecture/` and add an ADR under
  `docs/architecture/decisions/`.
- New config keys, CLI flags, or output fields are additive (see `docs/STABILITY.md`) and update
  `docs/config-schema.json` where relevant.

## Building distribution assets locally

Shell completions and the man page are generated from the CLI's clap definition (no runtime
subcommand):

```bash
cargo run -p xtasks -- gen-dist dist/assets
# -> dist/assets/completions/{forge.bash,_forge,forge.fish,_forge.ps1}, dist/assets/forge.1
```

The release workflow does this and bundles the output into every archive.

## Reporting bugs / proposing features

Open an issue using the relevant template. For substantial design changes, write an ADR or open a
discussion before a large PR. Security issues follow [`SECURITY.md`](SECURITY.md) — do **not** open
a public issue.
