# Sideloading Forge on iOS (Track A — testing now, no Apple approval needed)

This is the fast path to a real device: an **unsigned** IPA, installed and auto-updated through
[SideStore](https://sidestore.io). No Apple Developer Program membership, no App Store review, no
signing certificates. It mirrors the Helm app's distribution flow.

## How it works

1. Pushing a `mobile-v*` tag (e.g. `mobile-v1.0.0`) runs
   [`.github/workflows/mobile-sidestore.yml`](../../.github/workflows/mobile-sidestore.yml) on a
   macOS GitHub Actions runner:
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

Because the repo (`adulari/forge`) is public, the macOS runner minutes are free and unlimited on
GitHub Actions.

## The add-source URL

Once GitHub Pages is enabled for this repo (see "One-time setup" below), the stable add-source URL
is:

```
https://adulari.github.io/forge/forge-source.json
```

This is what you paste into SideStore. It is never versioned in the URL — every `mobile-v*`
release overwrites it in place, and the `versions[]` entry inside always reflects the tag that was
just built.

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

## Updating

SideStore polls added sources periodically (and on manual refresh). When a new `mobile-v*` tag
ships, `forge-source.json`'s `versions[0]` changes and SideStore surfaces an in-app update — no
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

## One-time setup required (not automatable from the workflow)

GitHub Pages must be pointed at Actions-based deployment once, by a human with repo admin access:

**Settings → Pages → Build and deployment → Source: "GitHub Actions"**

Until that's done, the `deploy-pages` job in `mobile-sidestore.yml` will fail (or the Pages URL
simply won't resolve) — use the release-asset manifest URL as a fallback in the meantime.

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
