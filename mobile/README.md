# Forge app

Unified companion app for the `forge` coding-agent daemon (`forge serve`) — one TypeScript/React
Native codebase, six targets: **iOS, Android, Web, Windows, macOS, Linux**. See
[`redesign/ARCHITECTURE.md`](redesign/ARCHITECTURE.md) for the binding design (this doc is
usage-only, not a spec).

The app never talks to anything except a real `forge serve` daemon over its documented REST/WS
API — no mocks, no bundled backend.

## Pairing

1. Start (or already have) a daemon: `forge serve --local` (or `--anywhere` for a real-TLS
   tunnel; see the TLS note below).
2. The daemon prints a `connect:` URL and a QR code.
3. In the app's Connect screen: scan the QR (native, camera) or paste the URL (web/desktop).
   Deep links also work: `forge://connect?url=<connect-url>` (native) or
   `https://<app-host>/connect?url=<connect-url>` (web).

**TLS note:** `--local` uses a self-signed cert that native fetch/WS, browsers, and the Tauri
WebView all reject. Use `--anywhere` (tunnel, real TLS) or `--local` + Tailscale/VPN for a
plain-`http://` daemon — the Tauri desktop build's transport seam is the one target that can
still reach a plain-http daemon directly (see §6.3 of ARCHITECTURE.md).

## Dev loops

```bash
npm ci

npm start          # native dev client (iOS/Android), Expo Go where possible
npm run web        # primary inner loop — point it at a --local/--anywhere daemon
npm run tauri:dev  # desktop shell (Tauri v2), wraps the web build with a native window
```

Gate before any PR: `npx tsc --noEmit` clean, `npm run lint` clean.

## Build matrix

| Target | Build | CI |
|---|---|---|
| iOS (sideload) | unsigned IPA, `xcodebuild archive` with `CODE_SIGNING_ALLOWED=NO` | [`mobile-sidestore.yml`](../.github/workflows/mobile-sidestore.yml) — tag `mobile-v*` |
| iOS (App Store) | `eas build`/`eas submit`, profile `production` | [`mobile-eas.yml`](../.github/workflows/mobile-eas.yml) — tag `mobile-release-v*`, requires `EXPO_TOKEN`; skips cleanly until Apple Developer approval + EAS credentials land |
| Android | `eas build -p android` (APK for sideload, AAB for Play later) | not yet wired in CI — build locally with `eas build -p android --profile preview` |
| Web | `npx expo export -p web` → `dist/` (static, deployable anywhere over HTTPS) | [`app-web.yml`](../.github/workflows/app-web.yml) — tag `app-v*` or manual dispatch |
| Windows / macOS / Linux | `npm run tauri:build` (Tauri v2, unsigned in v1) → `.exe`/`.msi`, `.dmg`, `.AppImage`/`.deb` | [`app-desktop.yml`](../.github/workflows/app-desktop.yml) — 3-OS matrix, tag `app-v*` or manual dispatch |

`eas.json` has `development` / `preview` / `production` build profiles; `ios.appleTeamId` is a
tracked TODO until the Apple Developer account is approved (does not block Android, web, or
desktop).

## iOS sideloading (SideStore)

No Apple Developer account needed. Add this source in SideStore (**Sources → + → Add Source**):

```
https://adulari.github.io/forge/forge-source.json
```

Full details, including the versioned release-asset fallback URL and update behavior: see
[`docs/mobile/SIDELOAD.md`](../docs/mobile/SIDELOAD.md).

## iOS simulator build (no Apple account)

For a native iOS build with zero Apple Developer credentials, use the `simulator` EAS profile
(unsigned, simulator-only):

```bash
eas build -p ios --profile simulator
```

This produces a simulator `.app` (tarball) in EAS's cloud — no credentials prompted. Run it by
either dragging the `.app` onto a local iOS Simulator (macOS + Xcode), or uploading it to
[Appetize.io](https://appetize.io) to drive it from a browser (works from a phone too). Then
`npx expo start --dev-client` to hot-reload into it.

## Distribution posture (v1)

Unsigned desktop builds + unsigned SideStore iOS + static web hosting. No store-submission
automation — the `mobile-eas.yml` App Store path is gated on `EXPO_TOKEN` and Apple approval,
and there is no auto-publish step anywhere in `app-desktop.yml`/`app-web.yml`; both only produce
build artifacts (plus an opt-in GitHub Pages deploy for the web export, off by default — see the
comments in `app-web.yml`). See [`docs/mobile/APP_STORE_CHECKLIST.md`](../docs/mobile/APP_STORE_CHECKLIST.md)
for what's still needed to flip the App Store path on.
