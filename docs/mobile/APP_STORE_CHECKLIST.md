# App Store readiness checklist (Track B)

Human-facing checklist for shipping Forge to TestFlight/App Store via Xcode Cloud (`mobile/eas.json`,
`mobile/ios/ci_scripts/ci_post_clone.sh`) and for publishing compatible EAS Updates
(`.github/workflows/eas-update.yml`). Everything here is either a manual Apple/App Store Connect
step that cannot be scripted, or a piece of app content owned by another workstream — this doc
just tells you what still needs doing and where.

Apple Developer Program enrollment approved 2026-07-08. Team ID `95VXXPD28Y` is now wired into
`mobile/eas.json` (`build.preview`/`build.production`/`submit.production`) and
`mobile/app.config.ts` (`ios.appleTeamId`) — confirmed via `npx expo config --type public`.

## 0. Prerequisites

- [x] Apple Developer Program membership approved.
- [x] Team ID `95VXXPD28Y` set in `mobile/eas.json` and `mobile/app.config.ts`.
- [ ] Create an **App Store Connect** app record for `dev.adulari.forge` (or let
      `eas submit` create it on first run) and fill in `submit.production.ios.ascAppId` in
      `mobile/eas.json` with the resulting numeric App ID.
- [ ] Set `submit.production.ios.appleId` in `mobile/eas.json` to the Apple ID email used to manage
      this app in App Store Connect.

## 1. EAS-managed credentials

EAS holds all signing material server-side, keyed to the project's Expo account — this is simpler
and safer than managing `.p12`/`.mobileprovision` files by hand, and it's what `eas.json` here is
set up for.

- [ ] Run `eas credentials` (interactively, once, from `mobile/`) and let EAS generate/upload:
  - iOS **Distribution Certificate**.
  - iOS **Provisioning Profile** (App Store distribution type) for `dev.adulari.forge`.
  - An **APNs key** — see §7 below, native push/widgets/Live Activities now build against it.
- [ ] Create an **App Store Connect API Key** (App Store Connect → Users and Access → Integrations
      → App Store Connect API → "+") and register it with EAS
      (`eas credentials` → iOS → App Store Connect API Key) so `eas submit` in CI can authenticate
      non-interactively. This key is stored by EAS, not as a GitHub secret.
- [x] Add the **`EXPO_TOKEN`** GitHub Actions secret (Expo access token — Expo dashboard → Account
      settings → Access tokens) so `.github/workflows/eas-update.yml` can publish compatible
      production OTA updates non-interactively. This secret does not build or submit a native
      binary; native iOS builds remain manual Xcode Cloud work.

## 2. Privacy manifest + App Privacy "nutrition label"

Apple requires both a bundled `PrivacyInfo.xcprivacy` manifest **and** matching answers in App
Store Connect's App Privacy questionnaire. The checked-in manifest and `mobile/app.config.ts` are
the synchronized source of truth. What this app actually does:

- **Data stored locally:** the daemon pairing token (`{scheme}://{host}:{port}/{token}`), stored in
  `expo-secure-store` on-device. It is a bearer credential for the user's *own* self-hosted `forge
  serve` daemon — it is never transmitted to Apple, Expo, or any third party.
- **Anonymous analytics:** fixed installation/activity counters go to PostHog EU. They contain no
  stable identifier, content, or location, are not linked to a user, are not used for tracking,
  and can be disabled under Settings → Privacy. There is no analytics SDK or advertising ID.
- **Data NOT collected:** no tracking or third-party data sharing. Session transcripts/tasks/diffs are fetched live from the user's own
  daemon over the connection they configured and are not persisted by Apple/Expo infrastructure
  beyond the on-device react-query cache.
- [ ] In App Store Connect → App Privacy, answer **no tracking** and declare **Product Interaction
      → Analytics**, not linked to identity. The pairing credential remains on-device and is used
      solely for app functionality.
- [ ] Confirm `PrivacyInfo.xcprivacy` (wherever the owning worker lands it) declares required
      reason API usage matching what's actually called — camera (QR scan), photo library
      (attachments), and secure-store/keychain access (token storage) are the ones this app
      actually uses per `mobile/app.config.ts`'s `infoPlist` strings and `expo-camera`/
      `expo-image-picker`/`expo-secure-store` plugins.

## 3. Required device capabilities

- [ ] Confirm `mobile/app.config.ts` `ios.infoPlist` usage strings are present for every permission
      actually invoked (camera, photo library, documents) — they're already there as of this
      writing; re-check if screens change.
- [ ] No unusual `UIRequiredDeviceCapabilities` needed — this is a standard networked app (HTTP +
      WebSocket client), no ARKit/NFC/etc. Leave Expo's defaults.
- [ ] Decide/confirm minimum iOS version. `eas.json`'s generated manifest and this app's use of
      current Expo SDK 57 / RN 0.86 imply **iOS 16+** — confirm against Expo SDK 57's actual
      minimum supported iOS version at build time (`npx expo config --type public` or Expo's SDK
      57 release notes) and keep it consistent with the `minOSVersion` used in the SideStore
      manifest (`.github/workflows/mobile-sidestore.yml`, currently `16.0` as a placeholder — align
      it once confirmed).

## 4. App icons, launch screen, screenshots

- [ ] App icon set (all required iOS sizes) — Expo/EAS generates the full icon set from a single
      1024×1024 source (`assets/icon.png`, already referenced in `app.config.ts`); verify it's a
      real 1024×1024 PNG with no alpha channel (App Store rejects icons with transparency).
- [ ] Launch screen: `mobile/app.config.ts` currently has **no working splash config** — see the
      `FLAGGED (w11-splash)` comment block in that file. SDK 57 needs the `expo-splash-screen`
      config plugin wired in (owned by the worker(s) touching `app.config.ts`, not this one) before
      a production build will have a real launch screen instead of a blank/default one. **This
      blocks a polished production submission** even though it doesn't block Track A sideloading.
- [ ] App Store screenshots: required sizes are per device class Apple currently mandates for the
      Store listing (6.7"/6.9" iPhone at minimum; iPad sizes only if `supportsTablet` stays `true`
      in `app.config.ts`, which it currently is). Capture from real screens once Batch 1–4 UI lands
      (`mobile/BUILD_PLAN.md` §6/§7) — a Simulator or a device via `--anywhere` tunnel both work
      since this app is a thin client over HTTP/WS.
- [ ] App preview video: optional, skip for v1.

## 5. Age rating & App Store listing metadata

- [ ] Age rating questionnaire in App Store Connect — this app has no user-generated content
      shared with other users, no gambling, no mature content; it's a personal dev-tool client.
      Expect the lowest tier (4+), but the questionnaire itself must be filled in App Store Connect
      by a human — it isn't derivable from the codebase.
- [ ] App name, subtitle, description, keywords, category (likely **Developer Tools**), support URL,
      marketing URL (optional).
- [ ] **Support URL**: point at the `adulari/forge` GitHub repo (issues) or a docs page — decide and
      set in App Store Connect; not encoded anywhere in this app's code.
- [ ] Copyright / legal entity name for the listing.

## 6. Reviewer access (App Review sign-in notes)

This app has no login of its own — it pairs with a self-hosted `forge serve` daemon via a token
URL (QR or paste). Apple's reviewer will not have a daemon to pair with unless one is provided.

- [ ] Provide reviewer notes in App Store Connect's "App Review Information" with either:
  - **A demo daemon**: run `forge serve --anywhere` against a throwaway/sandboxed repo, and paste
    the resulting `connect:` URL (or a QR screenshot) into the reviewer notes, valid for the
    review window. `--anywhere` gives real TLS via a tunnel (cloudflared/ngrok), which is required
    since App Review devices won't be on the same LAN or VPN as `--local`.
  - **Or** a short written walkthrough + screen-recording showing pairing and core flows, if
    standing up a live reviewer-accessible daemon for the review window isn't practical.
- [ ] Explicitly note in reviewer comments: "this app requires a running instance of the
  open-source `forge` CLI (`forge serve`) that the user runs on their own machine; there is no
  user-account system. Native iOS push notifications are, by default, relayed through a small
  operator-run forwarding service (source: `crates/forge-relay`, see ADR-0012) that sees only
  an opaque device token and the notification's title/body/status payload — never session
  content, source code, or credentials — and any user may point their own daemon at their own
  Apple Developer key (or their own relay instance) to bypass it entirely." This heads off a
  common rejection reason (apps that appear to require an unreachable backend) while staying
  accurate about the one small forwarding service that does exist.
  - Note if the reviewer-demo daemon (above) is configured with `FORGE_APNS_TEAM_ID`/`_KEY_ID`/
    `_KEY_PATH` (Direct mode) rather than the default relay, so reviewer notes describe
    whichever path is actually being exercised.
- [ ] Re-generate/rotate the demo token before and after the review window
  (`forge serve --rotate-token`) so a stale reviewer credential doesn't linger.

## 7. APNs key, App Group, Widgets/Live Activities, Xcode Cloud

Added once Team ID `95VXXPD28Y` unblocked these entitlement-gated capabilities: native push
(`crates/forge-cli/src/apns.rs`), a Home Screen widget + Live Activity (`mobile/targets/widget/`,
`mobile/modules/live-activity/`), and Xcode Cloud as the CI that actually compiles the Swift
(this repo's dev environment has no macOS/Xcode, so this is the only real build verification for
that code until a device/TestFlight test).

- [ ] **APNs key**: **developer.apple.com/account** (not App Store Connect — different site) →
      Certificates, Identifiers & Profiles → Keys → "+", enable "Apple Push Notifications service
      (APNs)". Download the `.p8` **once** (Apple won't let you re-download it) and record its Key
      ID and this account's Team ID (`95VXXPD28Y`).
- [ ] Configure the `forge serve` host with `FORGE_APNS_TEAM_ID=95VXXPD28Y`,
      `FORGE_APNS_KEY_ID=<key id>`, `FORGE_APNS_KEY_PATH=/path/to/AuthKey_<key id>.p8` (see
      `ApnsConfig::from_env` in `crates/forge-cli/src/apns.rs`). Never commit the `.p8` file.
- [ ] **App ID capabilities**: in the Developer Portal, edit the `dev.adulari.forge` App ID and
      enable **Push Notifications** and **App Groups** (both require the paid membership, both
      were unavailable before enrollment). Register the App Group
      `group.dev.adulari.forge` (must match `mobile/app.config.ts`'s `APP_GROUP` constant exactly
      — the widget/Live Activity extension and the main app share data through it).
- [x] **Xcode Cloud workflow**: App Store Connect → your app → Xcode Cloud → Get Started, connect
      the `adulari/forge` GitHub repo, and create a workflow scoped to `mobile/` changes on
      `main` (Xcode Cloud can filter by path). `mobile/ios/ci_scripts/ci_post_clone.sh` (already in
      the repo) runs `npm ci && npx expo prebuild` automatically post-clone — no other config
      needed for it to materialize the widget/Live-Activity extension target and build. Xcode
      Cloud handles signing itself (no `eas credentials`/EAS provisioning profile needed — that
      pipeline is gone, this is the only build/distribution path now). Workflow's Archive action
      has `buildDistributionAudience: INTERNAL_ONLY` set, so it auto-uploads to TestFlight on
      success.
- [ ] **One-time OTA bootstrap**: after the OTA channel configuration is merged, manually run one
      signed Xcode Cloud build and install its processed TestFlight build. Confirm the generated
      iOS `Expo.plist` contains `EXUpdatesURL`, `EXUpdatesRequestHeaders` with
      `expo-channel-name=production`, and `EXUpdatesRuntimeVersion`. This is required before
      installed binaries can receive production OTA updates; ordinary `main` pushes do not create
      a native binary.
- [ ] **OTA compatibility check**: publish only pushes whose complete changed-path set is limited
      to `mobile/src/**` or `mobile/assets/**` through `.github/workflows/eas-update.yml`. Native
      dependencies, Expo/RN upgrades, config plugins, permissions, entitlements, iOS/Android
      files, widgets, Live Activities, or any other path in the same push require a new Xcode Cloud
      binary or a follow-up OTA-safe push. The workflow skips mixed pushes automatically.
- [ ] **TestFlight OTA proof**: after the bootstrap build is processed, install it, launch it twice
      after a known compatible update, and confirm the update is downloaded/applied. Record the
      binary's fingerprint and production channel before publishing a user-facing OTA.
- [ ] **Release versioning**: mobile's native marketing/build version may differ from the Forge
      CLI/desktop version. Label a compatible OTA with the Forge release it carries (for example,
      `Forge v2.6.3`), and bump the mobile native version only when the native layer changes.
- [ ] **Beta-group assignment.** Xcode Cloud uploads the build but never assigns it to a beta
      group, so testers see nothing until `scripts/testflight-assign-group.mjs` runs. Because iOS
      builds are opt-in (Xcode Cloud is triggered by hand via the ASC API, not on push), the assign
      step is **manual-only** — `.github/workflows/testflight-autogroup.yml` is `workflow_dispatch`
      (auto-firing on push would just hang ~45m polling for a build that a plain push never
      produces). **Use the one-step trigger** so this can't be forgotten again (it stranded builds
      #68–#70): `scripts/trigger-ios-build.mjs` triggers the Xcode Cloud build AND, when
      `TESTFLIGHT_GROUPS` is set, waits for processing and assigns the group in the same run:
  - `ASC_KEY_ID=… ASC_ISSUER_ID=… ASC_API_PRIVATE_KEY="$(cat AuthKey_*.p8)" TESTFLIGHT_GROUPS=Testers node scripts/trigger-ios-build.mjs`
  - Or, if you triggered the build another way, assign after the fact with
    `gh workflow run testflight-autogroup.yml` or `scripts/testflight-assign-group.mjs` directly.
  - CI config for the workflow path:
  - repo **variable** `TESTFLIGHT_GROUPS` = the internal beta group name(s), comma-separated
    (e.g. `Internal`). The group must already exist in App Store Connect → TestFlight.
  - repo **secrets** `ASC_KEY_ID`, `ASC_ISSUER_ID`, `ASC_API_PRIVATE_KEY` — an App Store Connect
    API key (Users and Access → Integrations → App Store Connect API) with the Developer/App
    Manager role and access to the app; `ASC_API_PRIVATE_KEY` is the whole `AuthKey_*.p8` file.
    Until these are set the workflow skips cleanly (never fails).
- [ ] Also set the marketing version (`mobile/app.config.ts`'s `version`) higher than any build
      number Xcode Cloud's own automatic build-number management has already used for the current
      version string, if builds start failing "bundle version must be higher than previously
      uploaded" again — that setting isn't exposed via Xcode Cloud's UI/API, only inferable from
      failures.
- [ ] TestFlight builds are **production-signed**, not sandbox — a common misconception. Both
      TestFlight and App Store builds talk to APNs' production host; only Xcode
      Debug-run-on-device builds are sandbox (`ApnsNotifier`/`push.ios.ts` both derive this from
      `__DEV__`, matching that split).
- [ ] Once a device/TestFlight build exists, manually verify: the widget renders on the Home
      Screen and updates after a session's state changes; starting a turn shows a Live Activity
      on the Lock Screen and in the Dynamic Island; ending a turn dismisses/updates it correctly.
      **This cannot be confirmed from this environment** — nothing here has run on a real device
      or Simulator.

## 8. What a human must do that cannot be automated

Summary of the manual, Apple-side/App-Store-Connect-side actions from the sections above:

1. Wait for and accept Apple Developer Program approval; record the Team ID.
2. Create/confirm the App Store Connect app record and API key; wire IDs into `mobile/eas.json`.
3. Run `eas credentials` once (interactive) to provision the Distribution cert + profile.
4. Add the `EXPO_TOKEN` repo secret.
5. Fill in the App Privacy questionnaire in App Store Connect.
6. Supply App Store screenshots (needs real UI, i.e. after Batches 1–4 land).
7. Fill in age rating questionnaire, listing metadata, support URL.
8. Write App Review reviewer notes and stand up (or record) a reviewer-accessible demo daemon.
9. Coordinate with whoever owns `mobile/app.config.ts` to finish the splash-screen plugin wiring
   before the first production submission (not required for Track A/TestFlight-internal testing,
   but expected for a real App Store listing).
10. Generate the APNs `.p8` key, register the App Group, enable Push Notifications/App Groups on
    the App ID, and set up the Xcode Cloud workflow (§7) — then verify the widget/Live Activity
    on a real device or TestFlight build, since none of that can be confirmed headlessly.

None of steps 1–9 are things `mobile-eas.yml` or `eas.json` can do on their own — the workflow's
`guard` job simply keeps CI quiet (skips, doesn't fail) until step 4 is done, and `eas build`/
`eas submit` will still fail loudly if steps 1–3 aren't finished, with EAS's own error messages
pointing at what's missing.
