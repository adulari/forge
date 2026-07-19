# Sideloading Forge on iOS (Track A — testing now, no Apple approval needed)

This is the fast path to a real device: an **unsigned** IPA, installed and auto-updated through
[SideStore](https://sidestore.io). No Apple Developer Program membership, no App Store review, no
signing certificates. It mirrors the Helm app's distribution flow.

## How it works

1. After creating a `mobile-v*` tag (e.g. `mobile-v1.0.1`) on `main`, a maintainer dispatches
   [`.github/workflows/mobile-sidestore.yml`](../../.github/workflows/mobile-sidestore.yml) from
   protected `main` with that existing tag. The workflow checks out and validates the exact tag,
   then runs on a macOS GitHub Actions runner:
   - `npx expo prebuild -p ios` generates the native Xcode project from `mobile/app.config.ts`.
   - `xcodebuild archive` builds it with `CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO` — no
     certificate or provisioning profile is involved.
   - The resulting `Forge.app` is zipped into a standard IPA (`Payload/Forge.app` → `Forge.ipa`).
2. The workflow publishes a **GitHub Release** for the tag with `Forge.ipa` attached, and generates
   a **SideStore/AltStore source manifest** (`forge-source.json`) describing that release.
3. The manifest is published two places:
   - As a release asset, next to the IPA.
   - To **GitHub Pages**, so there's one URL that always resolves to the *latest* release without
     needing to know a tag name.

The unsigned archive still requires Xcode, so this is the one release leg that cannot run on the
dedicated Arch runner. It uses GitHub's `macos-15` runner; hosted-runner availability and GitHub's
current public-repository policy limits still apply.

## The add-source URL

The production repository is configured and the stable add-source URL is live:

```
https://adulari.github.io/forge/forge-source.json
```

Verify that URL returns the source JSON before pasting it into SideStore. It is never versioned in
the URL. A protected-`main` deployment replaces it only when the requested tag is the newest
published `mobile-v*` version, and then verifies that the live manifest points to that exact IPA.
Repairing an older tag can refresh that release's IPA/manifest assets but cannot roll the stable
source back from a newer mobile release.

If Pages isn't enabled yet, use the release-asset copy instead (versioned, so it won't silently
update):

```
https://github.com/adulari/forge/releases/download/<tag>/forge-source.json
```

## Installing on-device

1. Install [SideStore](https://sidestore.io) (requires pairing with a desktop companion the first
   time, per SideStore's own setup — outside the scope of this app).
2. In SideStore: **Sources → + → Add Source**, paste the add-source URL above.
3. Find "Forge" under that source and tap **Install** (or **Get**). SideStore handles re-signing
   the unsigned IPA with your personal Apple ID's free provisioning during install — this is what
   avoids needing a paid Apple Developer account for Track A.
4. Trust the resulting profile if prompted (Settings → General → VPN & Device Management).

The generated app currently requires iOS 16.4; the source manifest must advertise the same minimum
before the release is announced.

## Updating

SideStore polls added sources periodically (and on manual refresh). When a new `mobile-v*` tag is
published through the workflow, `forge-source.json`'s `versions[0]` changes and SideStore surfaces an in-app update — no
re-adding the source, no re-pairing.

Because SideStore's free-Apple-ID signing is tied to a 7-day certificate refresh cycle, SideStore
itself (not this pipeline) needs to periodically re-sign already-installed apps in the background —
that's inherent to free sideloading and not something this CI can influence.

## Limitations of the current manifest

The generated `forge-source.json` carries only the **latest** release in `versions[]`, not full
version history — SideStore only needs the latest entry to detect and offer an update, so this
keeps the CI script simple. If historical versions are ever wanted (e.g. to let users roll back),
extend the `jq` step in `mobile-sidestore.yml` to fetch and merge the previous manifest's
`versions[]` array before writing the new one.

## One-time setup for forks

Forge's production repository is already configured. A fork must point GitHub Pages at
Actions-based deployment once, by a human with repo admin access:

**Settings → Pages → Build and deployment → Source: "GitHub Actions"**

The `github-pages` environment must allow deployments from the protected `main` branch:

**Settings → Environments → github-pages → Deployment branches and tags → Selected branches
→ `main`**

Without both settings, GitHub can reject `deploy-pages` before a runner starts (or the Pages URL
simply won't resolve) even after the IPA and release manifest succeed. Use the release-asset
manifest URL as a fallback in the meantime.

## Alternative considered: serving the manifest from `forge serve`

Rather than GitHub Pages, the AltSource manifest could instead be served directly by the user's own
running `forge serve` daemon, at a path like `/sideload/source.json` and `/sideload/Forge.ipa`
(mirroring how `forge serve` already serves the PWA assets under `/<token>/...` — see
`crates/forge-cli/src/serve.rs`, which registers routes like `/api/sessions`, `/app.js`, etc. on an
`axum::Router`).

This was **not implemented** (this worker owns only `mobile/eas.json`, `.github/workflows/mobile-*`,
and `docs/mobile/*` — no Rust changes). Sketch of what it would take, for whoever picks it up:

- A new pair of routes in `serve.rs`'s router-building function, e.g.
  `GET /sideload/source.json` and `GET /sideload/Forge.ipa`, added alongside the existing
  `.route(&format!("{base}/..."), get(...))` chain.
- The IPA and manifest would need to live somewhere the daemon can read them (bundled into the
  `forge` binary via `include_bytes!`, similar to how `remote_assets/` is embedded, or fetched from
  the latest GitHub Release at daemon startup).
- Auth question to resolve: should `/sideload/*` require the daemon token like every other route,
  or be deliberately unauthenticated (since installing the app is a prerequisite to ever having a
  token)? Precedent in this codebase is that **everything** lives under `/<token>/...` (§1.1 of
  `mobile/BUILD_PLAN.md`) — an unauthenticated `/sideload/*` would be the first exception.
- Upside over GitHub Pages: works fully offline / on a LAN-only daemon, no GitHub Pages setup step,
  ties distribution to the same host serving the app's actual API. Downside: couples app
  distribution to a single running daemon's uptime, and needs someone to keep the bundled IPA in
  sync with the CI-built one (vs. GitHub Releases being the natural single source of truth today).

If pursued, this is a small, self-contained addition — not a blocker for Track A as shipped here.
