# Forge app

Unified companion app for the `forge` coding-agent daemon (`forge serve`) — one TypeScript/React
Native codebase, six targets: **iOS, Android, Web, Windows, macOS, Linux**. See
[`redesign/ARCHITECTURE.md`](redesign/ARCHITECTURE.md) for the binding design (this doc is
usage-only, not a spec).

Session/control traffic goes only to a real `forge serve` daemon over its documented REST/WS API —
no mocks or bundled backend. Separately, production builds can fetch runtime-compatible OTA bundles
from Expo and send the fixed anonymous counters documented in [`docs/telemetry.md`](../docs/telemetry.md)
to PostHog EU; both are independent of session content, and analytics is user-disableable.

## Pairing

1. Start (or already have) a daemon. Use `forge serve --local` for the desktop/web app on the same
   machine, or `forge serve --anywhere` for a phone or another computer.
2. The daemon prints a `connect:` URL and a QR code.
3. In the app's Connect screen: scan the QR (native, camera) or paste the URL (web/desktop).
   Deep links also work: `forge://connect?url=<connect-url>` (native) or
   `https://<app-host>/connect?url=<connect-url>` (web).

**TLS note:** `--local` is plain HTTP on `127.0.0.1` and is reachable only from the daemon's own
machine. The default/`--lan` exposure uses a self-signed HTTPS certificate that native fetch/WS,
browsers, and the Tauri WebView do not trust automatically. Use `--anywhere` for another device, or
put the local daemon behind an explicitly configured trusted HTTPS reverse proxy such as Tailscale
Serve. The Tauri desktop transport can reach a same-machine plain-HTTP daemon directly (see §6.3
of ARCHITECTURE.md).

## Sessions and projects

After pairing, the app exposes the active Fleet plus persisted History; the Fleet keeps
decision-waiting work at the top. Creating a session defaults to the daemon's current project,
remembers the last project used on that server, and offers recent projects plus the daemon's
allowlisted folder browser.
Manual path entry remains under the advanced fallback; the Tauri desktop app can use a native
folder picker. A session can optionally start in an isolated git worktree, and the model picker
prioritizes discovered healthy models while retaining automatic mesh routing and an offline manual
ID fallback.

On desktop and web viewports at least 1024 px wide, the Fleet rail remains visible beside the active
chat. Phones use focused full-screen navigation. Offline prompts queue locally in FIFO order, and a
reconnect replays missed server frames before queued input is reconciled.

## Dev loops

```bash
npm ci

npm start          # native dev client (iOS/Android), Expo Go where possible
npm run web        # primary inner loop — point it at a --local/--anywhere daemon
npm run tauri:dev  # desktop shell (Tauri v2), wraps the web build with a native window
```

Gate before any PR: `npm run check` (lint, TypeScript, and Vitest) clean.

## Build matrix

| Target | Build | CI |
|---|---|---|
| iOS (sideload) | unsigned IPA, `xcodebuild archive` with `CODE_SIGNING_ALLOWED=NO` | [`mobile-sidestore.yml`](../.github/workflows/mobile-sidestore.yml) — protected-`main` dispatch with an existing `mobile-v*` tag |
| iOS (App Store) | Xcode Cloud (`mobile/ios/ci_scripts/ci_post_clone.sh` runs `expo prebuild`, Xcode Cloud archives/signs/uploads) | App Store Connect → Xcode Cloud, workflow on `main` scoped to `mobile/`; see the [`App Store/mobile checklist`](../docs/mobile/APP_STORE_CHECKLIST.md) |
| Android | `eas build -p android` (APK for sideload, AAB for Play) | [`mobile-android.yml`](../.github/workflows/mobile-android.yml) — protected-`main` preview/production dispatch, optional existing `android-v*` artifact tag and Play submit |
| Web | `npx expo export -p web` → `dist/` (static, deployable anywhere over HTTPS) | [`app-web.yml`](../.github/workflows/app-web.yml) — protected-`main` dispatch; optional exact-tag release asset |
| Windows / macOS / Linux | `npm run tauri:build` (Tauri v2; OS installers are unsigned, updater artifacts are signed in release CI) → NSIS `.exe`, `.dmg`, `.AppImage`/`.deb` | [`app-desktop.yml`](../.github/workflows/app-desktop.yml) — protected-`main` dispatch, five exact-tag targets across 3 OSes |

`eas.json` has `development` / `preview` / `production` build profiles, used for Android builds
and the iOS simulator build below — the iOS App Store submit path no longer uses EAS (Xcode
Cloud handles that end to end now, see the table above).

## iOS sideloading (SideStore)

No Apple Developer account is needed. When the Pages deployment is live, add this source in
SideStore (**Sources → + → Add Source**):

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

Unsigned desktop installers + unsigned SideStore iOS + static web export, plus a working signed iOS
App Store path via Xcode Cloud (see the build matrix above). `app-desktop.yml` transactionally
publishes the complete five-platform bundle and updater manifest to a versioned GitHub release;
`app-web.yml` always produces a build artifact, while its GitHub Pages deploy remains opt-in and off
by default because this repository's Pages site hosts the SideStore source. See
[`docs/mobile/APP_STORE_CHECKLIST.md`](../docs/mobile/APP_STORE_CHECKLIST.md) for the remaining
manual App Store Connect steps (beta group assignment, App Store listing, etc).
