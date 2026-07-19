# Releasing Forge

A fixed checklist for cutting a release. Follow it top to bottom — most release breakage has been
a *skipped step* (a stale Homebrew version that shipped the previous binary, an empty changelog on
a minor bump), not a hard problem. Do not improvise the order.

Replace `X.Y.Z` with the version below. Pick the bump per SemVer: **patch** for fixes only,
**minor** for new features or behaviour changes, **major** for breaking changes. A version with no
user-facing change should not be released at all.

## 1. Branch

```bash
git fetch origin
git switch -c release/vX.Y.Z origin/main   # always branch from origin/main, never a stale local
```

## 2. Bump the workspace version

1. `Cargo.toml` — workspace `version = "X.Y.Z"`.
2. `Cargo.lock` — run `cargo build --locked` after refreshing the lock; every versioned
   `forge-agent-*` entry must read `X.Y.Z`. The fixed `forge-agent-genai` fork has its own version.

Do **not** pre-bump `Formula/forge.rb`, `packaging/aur/PKGBUILD`, or `bucket/forge.json` to assets
that do not exist. `release.yml` updates all three together from the published `checksums.txt` in
step 6.

Verify the workspace version is consistent:

```bash
grep -n "X.Y.Z" Cargo.toml
grep -n "<old version>" Cargo.toml # empty
```

## 3. Changelog

Add a `## [X.Y.Z] - YYYY-MM-DD` section to `CHANGELOG.md` with REAL entries (what changed and why,
with the touched file). A minor/major bump with only a "prepared the workspace" line is wrong —
either there is real content or it should not be a release. Update the compare links at the bottom:
add `[X.Y.Z]` and repoint `[Unreleased]` to `vX.Y.Z...HEAD`.

This section is the source of truth for the **CLI/TUI and desktop release note**. Mobile uses the
same human-readable changelog content for OTA/TestFlight “What to Test” notes, but its native
version is independent and a new binary is only built manually when native changes require it:
- **GitHub Release** (`v*` tag): dispatching `release.yml` from protected `main` composes the body from this
  CHANGELOG section, then appends GitHub's auto PR list (hybrid). TUI binaries + desktop bundles +
  `latest.json` all attach to this same release.
- **Mobile OTA** (iOS): `.github/workflows/eas-update.yml` publishes JavaScript/assets to the
  `production` channel for OTA-safe mobile-source changes on `main`. A main-only manual dispatch is
  the recovery path when an automatic publish failed; the runtime fingerprint still gates delivery.
- **TestFlight** (iOS): `scripts/testflight-assign-group.mjs` reads the same section and sets the
  build's "What to Test" note via the ASC API (best-effort). Trigger Xcode Cloud manually only
  when native changes require a new binary; the IPA is not a GitHub Release asset.

## 4. Pre-flight — all must be green (CI runs these too; do not rely on a hook)

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features
cargo test --locked --workspace --all-features
cargo build --release --locked --bin forge
scripts/check-linux-runtime-deps.sh target/release/forge

cargo fmt --manifest-path vendor/genai-0.6.5/Cargo.toml -- --check
cargo clippy --locked --manifest-path vendor/genai-0.6.5/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path vendor/genai-0.6.5/Cargo.toml

(cd mobile && npm ci && npm run check && npx --no-install expo export -p web)
cargo test --locked --manifest-path mobile/src-tauri/Cargo.toml
cargo clippy --locked --manifest-path mobile/src-tauri/Cargo.toml --all-targets -- -D warnings
actionlint .github/workflows/*.yml
```

Also run the root, vendored-fork, and Tauri `cargo audit`/`cargo deny` commands documented in
[`CONTRIBUTING.md`](CONTRIBUTING.md). The known exceptions are narrow, documented unmaintained or
upstream-Tauri advisories; never add an ignore to make a real vulnerability green.

## 5. PR and merge

Open a PR (`chore: prepare vX.Y.Z release`), let every required check pass, and merge to `main`.
Branch protection must require the aggregate `CI` check. Security checks still run on every PR,
while mobile/Tauri checks run when their source paths change. Do **not** tag the branch — the tag
goes on `main` after merge.

## 6. Tag and release

```bash
git switch main && git pull --ff-only origin main
git tag vX.Y.Z && git push origin vX.Y.Z
gh workflow run release.yml --ref main -f release_tag=vX.Y.Z
```

Dispatch immediately after tagging: `release.yml` requires the tag commit to equal the protected
`main` workflow-dispatch SHA, not merely be an ancestor of it. This keeps the source named by the
build-provenance attestation identical to the source that is checked out and published. If `main`
advances first, do not move the release tag; prepare a new version from the new head instead.

`release.yml` also validates that the existing tag matches the workspace version, builds all five
CLI/TUI targets + `checksums.txt`, attests and publishes the release, and
opens an auto-merge PR that updates `Formula/forge.rb`, `packaging/aur/PKGBUILD`, and
`packaging/aur/.SRCINFO`, and `bucket/forge.json` from those exact checksums. It then dispatches the
transactional five-platform desktop build and static web export from protected `main`, both checking
out the exact tag. Mobile source changes publish through the independent production OTA workflow.
Because the manifest PR is created with `GITHUB_TOKEN`, `release.yml` explicitly dispatches every
branch-protection workflow on its branch before enabling auto-merge.
The x86-64 and ARM64 Linux legs run inside the same digest-pinned Debian Bullseye container and
enforce glibc 2.31, GLIBCXX 3.4.28, and no-ALSA ceilings before uploading either binary.
Wait for the CLI, desktop, web, and package-manifest runs to finish:

```bash
gh release view vX.Y.Z --json assets
gh pr list --state all --head dist/vX.Y.Z
```

After the GitHub tag/release exists, publish the matching `forge-agent*` crates in dependency order
using [`docs/RELEASING-crates.md`](docs/RELEASING-crates.md). Do not describe the Cargo channel as
current until crates.io has indexed the binary crate at X.Y.Z and a clean install succeeds.

If a compatible production OTA failed and the release commit is now on `main`, recover it with
`gh workflow run eas-update.yml --ref main`. Never dispatch a production OTA from a topic branch.

If manifest automation needs manual recovery, run
`scripts/update-package-manifests.sh X.Y.Z`, then open one PR with its three changed manifests.
If the desktop matrix needs repair after a tag was created, merge the workflow fix and run
`gh workflow run app-desktop.yml --ref main -f release_tag=vX.Y.Z`; this checks out and rebuilds the
exact existing tag, then transactionally republishes the complete platform set.

## 7. Verify

- `gh release view vX.Y.Z` shows latest with 5 CLI archives + checksums and desktop assets.
- A pre-X.Y.Z binary's `forge update` self-replaces to X.Y.Z.
- `brew install Adulari/forge/forge` and `scoop install forge/forge` resolve X.Y.Z with
  non-placeholder hashes. Publish and verify AUR separately after its maintainer SSH key is set.
- `cargo install forge-agent --version X.Y.Z` succeeds from a clean Cargo home after crates.io has
  indexed every publishable package.
- `latest.json` contains all five signed desktop updater platforms.
- `gh attestation verify` succeeds for CLI/TUI, desktop, web, IPA, and Android release assets.
- The released Linux CLI/TUI starts in the distro battery without ALSA installed; the runtime gate
  reports glibc ≤2.31, GLIBCXX ≤3.4.28, and no `libasound.so` dependency for both architectures.
- If the release includes OTA-safe mobile changes, the production EAS update group points at the
  current `main` commit and the installed TestFlight/App Store runtime fingerprint.
