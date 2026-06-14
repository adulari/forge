# Contributing to Forge

Thanks for your interest in Forge. This document covers the workflow, branching model,
and quality bar for contributions.

## Development workflow

1. **Fork & branch.** Create a topic branch off `main`. Never commit directly to `main`.
2. **Branch naming:** `feat/<slug>`, `fix/<slug>`, `refactor/<slug>`, `docs/<slug>`,
   `chore/<slug>`. Example: `feat/model-mesh-router`.
3. **Conventional Commits.** Commit messages follow
   [Conventional Commits](https://www.conventionalcommits.org/):
   `feat:`, `fix:`, `refactor:`, `docs:`, `chore:`, `test:`, `perf:`, `ci:`.
4. **Keep it green.** Before pushing, run the local checks (below). CI must pass before
   a PR is mergeable.
5. **Open a PR** into `main`. Fill out the PR template. At least one approving review and
   green CI are required to merge. PRs are squash-merged to keep `main` history linear.

## Branching model

- `main` — always releasable, protected. Squash-merge only, linear history.
- topic branches — short-lived, one logical change each, deleted after merge.
- release tags — `vMAJOR.MINOR.PATCH` ([SemVer](https://semver.org/)) cut from `main`.

## Local checks (run before every push)

Once the Rust project is scaffolded (Phase 4):

```bash
cargo fmt --all -- --check     # formatting
cargo clippy --all-targets --all-features -- -D warnings   # lints (warnings = errors)
cargo test --all               # tests
cargo build --release          # build
```

These are exactly what CI runs. Match CI locally to avoid surprises.

## Code standards

- Comments explain **why**, not what. No comments where the code is self-evident.
- Prefer explicit over clever.
- New behaviour ships with tests.
- Architecture-affecting changes update `docs/architecture/` and add an ADR under
  `docs/architecture/decisions/`.

## Reporting bugs / proposing features

Open an issue using the relevant template. For substantial design changes, write an ADR
or open a discussion before large PRs — Forge is design-first.
