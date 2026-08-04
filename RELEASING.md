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

The release workflow derives the desktop bundle version from the workspace `Cargo.toml` with
`cargo metadata`, verifies that it matches the release tag, and stamps `mobile/src-tauri/tauri.conf.json`
just before bundling. Do not hand-edit the Tauri version for a release; a mismatch now fails the
workflow before build.


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
  `production` channel. It fires two ways: on any `mobile/**` push to `main`, and from `release.yml`'s
  `ota` job. Every run classifies the complete range from `IOS_OTA_COMPATIBLE_BASE_SHA`—the source
  commit of the installed native archive—to current `main`; neither a push base nor a manual
  dispatch can narrow or bypass that range. Every release therefore either ships the matching OTA
  or says out loud why it cannot, while a dropped push event is repaired by the reconciler. The
  separately recorded `IOS_OTA_RUNTIME_VERSION` gates delivery in every case.
  - If the guard refuses (`OTA not published — native build required` failure), the change touched
    native/dependency/build config. Run Xcode Cloud, then set `IOS_OTA_COMPATIBLE_BASE_SHA` to that
    archive's exact source commit and refresh `IOS_OTA_RUNTIME_VERSION` from its embedded fingerprint
    before another OTA can reach devices.
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
CLI/TUI targets + `checksums.txt`, attests them, and stages them into a **draft** release. It then
immediately dispatches the transactional five-platform desktop build and static web export from
protected `main`, both checking out the exact tag. A separate `manifests` job (`needs: release`)
opens the auto-merge PR that updates `Formula/forge.rb`, `packaging/aur/PKGBUILD`,
`packaging/aur/.SRCINFO`, and `bucket/forge.json` from those exact checksums. That split is
deliberate: the manifest chain is several independently flaky operations, and while it lived above
the dispatch in the same job any one of them failing meant the tag never got desktop or web
artifacts at all. A red `manifests` job is still a failed release, but it can no longer starve the
artifacts. Mobile source changes publish through the independent production OTA workflow.

The release stays a draft until `app-desktop.yml` has attached the five desktop bundles,
`desktop-checksums.txt`, and `latest.json`; that workflow flips it to published, verifies the
public CDN bytes of both manifests, and only then moves GitHub's Latest pointer. Nothing about the
new version is publicly resolvable before that — no release page, no `releases/latest`, no
`releases/download/vX.Y.Z/...`. This is what keeps `install-desktop.sh` and the Tauri updater from
resolving a version whose `desktop-checksums.txt`/`latest.json` have not been uploaded yet.

**If the desktop matrix fails, the release deliberately remains a draft.** Nothing ships, the
previous release stays Latest and installable, and the red `app-desktop` run is the signal. Fix the
cause and re-dispatch (see below) — do not publish the draft by hand; that reintroduces exactly the
partially-populated release the draft exists to prevent. Until the draft publishes, the
`dist/vX.Y.Z` manifest PR references download URLs that 404; it is safe to let it merge, but do not
announce Homebrew/Scoop availability before the release is published.

Because the manifest PR is created with `GITHUB_TOKEN`, `release.yml` explicitly dispatches every
branch-protection workflow on its branch before enabling auto-merge.
The x86-64 and ARM64 Linux legs run inside the same digest-pinned Debian Bullseye container and
enforce glibc 2.31, GLIBCXX 3.4.28, and no-ALSA ceilings before uploading either binary.
Wait for the CLI, desktop, web, and package-manifest runs to finish:

```bash
gh release view vX.Y.Z --json isDraft,assets   # isDraft must be false once app-desktop finished
gh pr list --state all --head dist/vX.Y.Z
```

After the GitHub tag/release exists, publish the matching `forge-agent*` crates in dependency order
using [`docs/RELEASING-crates.md`](docs/RELEASING-crates.md). Do not describe the Cargo channel as
current until crates.io has indexed the binary crate at X.Y.Z and a clean install succeeds.

If a compatible production OTA failed and the release commit is now on `main`, recover it with
`gh workflow run eas-update.yml --ref main`. The installed-archive baseline still applies; there is
no manual bypass. Never dispatch a production OTA from a topic branch.

If manifest automation needs manual recovery, run
`scripts/update-package-manifests.sh X.Y.Z`, then open one PR with its three changed manifests.
If the desktop matrix needs repair after a tag was created, merge the workflow fix and run
`gh workflow run app-desktop.yml --ref main -f release_tag=vX.Y.Z`; this checks out and rebuilds the
exact existing tag, then transactionally republishes the complete platform set.

## 7. Verify

- `gh release view vX.Y.Z` shows a published (non-draft) latest release with 5 CLI archives +
  `checksums.txt`, the desktop bundles, `desktop-checksums.txt`, and `latest.json`.
- The documented one-liner installs on a clean host:
  `curl -fsSL https://raw.githubusercontent.com/Adulari/forge/main/install-desktop.sh | sh`.
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
