# Releasing without a prep PR

A release is the tag plus one dispatch. There is no `chore: prepare vX.Y.Z` PR and no PR CI + main
CI wait before it: the version comes from the tag.

```bash
git switch main && git pull --ff-only origin main
scripts/release.sh vX.Y.Z          # checks, tags main's tip, pushes the tag, dispatches release.yml
scripts/release.sh --dry-run       # rehearsal from the current branch: builds everything, publishes nothing
```

`scripts/release.sh` refuses an empty `[Unreleased]` changelog, a stale checkout and an existing tag.
Pushing the tag stays a manual step on purpose: the `Protected release tags` ruleset only lets a
repository admin create `v*` tags, and the workflow `GITHUB_TOKEN` cannot.

## What the workflow does with the version

`scripts/ci/stamp-release-version.sh X.Y.Z` runs at the start of every job that builds or publishes
(`release.yml` assets, build, release; `app-desktop.yml` build). It rewrites:

| File | Change |
| --- | --- |
| `Cargo.toml` | `[workspace.package] version` |
| `Cargo.lock` | the workspace members' own entries (offline, no resolver run, so `--locked` holds) |
| `mobile/src-tauri/tauri.conf.json` | `version` |
| `CHANGELOG.md`, `crates/forge-cli/CHANGELOG.md` | `## [Unreleased]` becomes `## [X.Y.Z] - date` under a fresh empty `[Unreleased]`; compare links repointed |

The binary embeds `CHANGELOG.md`, and `forge --version` reads `CARGO_PKG_VERSION`, so both come out
right. The GitHub release body is composed from the same rolled section, and `latest.json`, the
manifests and the upgrade gate all read the stamped version. The script is idempotent and refuses to
move a version backwards.

`require-release-version.sh` is the preflight gate: the tag must equal the committed workspace
version (old flow) or be greater and the source must carry the stamp script. An older or mislabelled
tag still fails in seconds.

## Getting main back in step

The tag's commit still says the previous version. After publishing, the `manifests` job runs
`scripts/ci/release-catchup.sh X.Y.Z` on a fresh clone of main and puts the result in the same
auto-merge `dist/vX.Y.Z` PR as the Homebrew/AUR/Scoop manifests: the version files and the changelog
roll. Entries merged after the tag stay under `[Unreleased]`; only the lines that were in the
tag's `[Unreleased]` move into the new section. If main already carries the version (a release
prepared by hand) it is a no-op.

## Things that are different from before, and why that is acceptable

- **The tagged commit carries the old version in `Cargo.toml`.** Binaries, desktop bundles, the
  release notes and the attestation are built from the tag plus the deterministic stamp, so they are
  correct. `cargo install --git ... --tag vX.Y.Z` or building the bare tag by hand reports the
  previous version. The provenance attestation names the tag commit; the stamp is a pure function of
  (tag, `[Unreleased]` text at that commit), so it is reproducible by running the script.
- **crates.io**: publish from a checkout of main after the `dist/vX.Y.Z` PR merges, or from the tag
  after `scripts/ci/stamp-release-version.sh X.Y.Z`. See `docs/RELEASING-crates.md`.
- **Between the release and the dist PR merging**, main's dev builds report the previous version.
- **Hand-prepared releases still work.** Bump the version yourself, tag, dispatch; the stamp sees
  nothing to do.

## Rehearsal

`--dry-run` dispatches `release.yml` with `dry_run=true` from the current branch. It stamps a fake
`v2.99.0`, builds all five targets, assembles the archives and runs the upgrade/reconnect/rollback
gate; it attests, publishes and dispatches nothing.

## Where the time goes (dry run, hosted runners)

See the PR description for measured numbers; the critical path is the Windows leg.
