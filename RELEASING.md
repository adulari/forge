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
2. `Cargo.lock` — run `cargo build` once; it rewrites every `forge-*` crate to `X.Y.Z`. Stage it.

Do **not** pre-bump Homebrew, AUR, or Scoop to assets that do not exist. `release.yml` updates all
three together from the published `checksums.txt` in step 6.

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
- **GitHub Release** (`v*` tag): `release.yml` composes the body from an all-platform header + this
  CHANGELOG section, then appends GitHub's auto PR list (hybrid). TUI binaries + desktop bundles +
  `latest.json` all attach to this same release.
- **Mobile OTA** (iOS): `.github/workflows/eas-update.yml` publishes JavaScript/assets to the
  `production` channel only for mobile-source changes on `main`.
- **TestFlight** (iOS): `scripts/testflight-assign-group.mjs` reads the same section and sets the
  build's "What to Test" note via the ASC API (best-effort). Trigger Xcode Cloud manually only
  when native changes require a new binary; the IPA is not a GitHub Release asset.

## 4. Pre-flight — all must be green (CI runs these too; do not rely on a hook)

```bash
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test --workspace
```

## 5. PR and merge

Open a PR (`chore: prepare vX.Y.Z release`), let CI pass, merge to `main`. Do **not** tag the branch
— the tag goes on `main` after merge.

## 6. Tag and release

```bash
git switch main && git pull --ff-only origin main
git tag vX.Y.Z && git push origin vX.Y.Z          # triggers .github/workflows/release.yml
```

`release.yml` builds all five CLI/TUI targets + `checksums.txt`, publishes the GitHub Release, and
opens an auto-merge PR that updates Homebrew, AUR, and Scoop from those exact checksums. The same tag
triggers the transactional five-platform desktop build; mobile source changes on `main` publish
through the independent production OTA workflow. Wait for both release workflows and the
package-manifest PR to finish:

```bash
gh release view vX.Y.Z --json assets
gh pr list --state all --head dist/vX.Y.Z
```

If manifest automation needs manual recovery, run
`scripts/update-package-manifests.sh X.Y.Z`, then open one PR with its three changed manifests.

## 7. Verify

- `gh release view vX.Y.Z` shows latest with 5 CLI archives + checksums and desktop assets.
- A pre-X.Y.Z binary's `forge update` self-replaces to X.Y.Z.
- Homebrew, AUR, and Scoop all resolve X.Y.Z with non-placeholder hashes.
- `latest.json` contains all five signed desktop updater platforms.
