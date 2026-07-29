# Code archaeology: marketplace persistence boundary

## Decision

The marketplace registry and installed-skill lockfile are a cohesive persistence boundary. Their TOML models, path policy, serialization, and mutation helpers live in `marketplace/registry.rs`; git resolution, fetching, installation, update orchestration, and output remain in `marketplace.rs`.

## Invariants

- A missing registry or lockfile represents an empty state.
- Any other read or TOML parse failure is returned before a mutation can write replacement state.
- Directory-creation and write failures are contextual errors.
- Plugin removal updates its lockfile before its caller removes skill content.

## Verification

Focused registry round-trip, malformed-state preservation, removal, and resolution tests plus warnings-denied Clippy and the architecture-size guard characterize the split.
