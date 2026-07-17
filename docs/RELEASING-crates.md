# Releasing to crates.io

Forge is a Cargo workspace published under the **`forge-agent`** brand (the `forge-*` package names are
already taken on crates.io by unrelated projects). Each crate's **package** name is `forge-agent-X`, but
its **lib** name is preserved as `forge_X`, so every `use forge_X::` import and `forge-X.workspace =
true` dependency key still works with zero source changes. The binary crate publishes as the bare
**`forge-agent`** and still builds a binary named `forge`. Installing:

```bash
cargo install forge-agent      # builds + installs the `forge` binary from crates.io
```

On Linux this default install deliberately omits CPAL/ALSA so Forge starts without an audio
runtime. File/upload transcription remains available. Users who want TUI microphone capture can
install ALSA development headers and run `cargo install forge-agent --features microphone`.

> **Naming note.** The package/lib split is intentional. Dependency KEYS in
> `[workspace.dependencies]` stay `forge-X` (with `package = "forge-agent-X"`) so dependents and source
> imports are untouched; only the published crate names change. The binary crate keeps
> `[[bin]] name = "forge"`, so `cargo install forge-agent` yields the `forge` command.

## Prerequisites

- A crates.io API token with publish rights (`cargo login`).
- A clean tree on a release tag; `Cargo.lock` committed and in sync (`cargo build --locked`).
- All internal crates share one version (`workspace.package.version`) and the
  `[workspace.dependencies]` `version` fields **match it** (see the comment in the root
  `Cargo.toml`). A mismatch makes `cargo publish` fail to select sibling crates.

## Publish order

Crates must be published leaf-first: a crate can only be published once every crate it depends on is
already on crates.io at the matching version. The valid topological order for this workspace (package
names):

Publish the fixed-version dependency fork first when it has not yet been indexed:

0. `forge-agent-genai` (`0.6.5-forge.1`; published once, not once per Forge release)

Then publish the versioned Forge graph:

1. `forge-agent-types`
2. `forge-agent-workflow`
3. `forge-agent-voice`
4. `forge-agent-skills`
5. `forge-agent-store`
6. `forge-agent-config`
7. `forge-agent-lsp`
8. `forge-agent-mcp`
9. `forge-agent-mesh`
10. `forge-agent-index`
11. `forge-agent-tui`
12. `forge-agent-provider`
13. `forge-agent-tools`
14. `forge-agent-core`
15. `forge-agent` (the binary crate, published last)

(`forge-relay` and `xtasks` are `publish = false` and are never released.)

## Dry run first

Verify packaging for each crate without publishing:

```bash
cargo publish -p forge-agent-types  --dry-run
cargo publish -p forge-agent-config --dry-run
cargo publish -p forge-agent        --dry-run
# ...etc
```

`--dry-run` packages the crate and type-checks the packaged copy. Leaf crates can dry-run before the
rest of the graph is indexed; dependent crates may report a missing matching `forge-agent-*` package
until their prerequisites are published. Resume in order once each dependency reaches the index.

## Publish

Run in the order above, waiting for each to be live (crates.io indexes within seconds) before the
next:

```bash
cargo info --registry crates-io forge-agent-genai@0.6.5-forge.1 >/dev/null 2>&1 || \
  cargo publish --locked --manifest-path vendor/genai-0.6.5/Cargo.toml

for crate in forge-agent-types forge-agent-workflow forge-agent-voice forge-agent-skills forge-agent-store \
             forge-agent-config forge-agent-lsp forge-agent-mcp forge-agent-mesh forge-agent-index \
             forge-agent-tui forge-agent-provider forge-agent-tools forge-agent-core forge-agent; do
  cargo publish -p "$crate" --locked
  # give the index a moment so the next crate can resolve this one
  sleep 20
done
```

If a publish fails midway, fix it and resume from the failed crate — already-published crates can't
be re-published at the same version (bump the patch and retry the whole set if needed).

## After publishing

- `cargo install forge-agent` should now work on a clean machine (installs the `forge` binary).
- The already-created tag + GitHub release (handled by `.github/workflows/release.yml`) provide the
  prebuilt binaries plus repository-backed Homebrew and Scoop manifests. AUR publication remains a
  separate push to the AUR Git repository after the maintainer key is configured.
