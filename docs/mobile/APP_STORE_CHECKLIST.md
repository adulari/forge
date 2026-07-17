# App Store and mobile release checklist

This is the current operator checklist for Forge's mobile releases. The supported distribution
paths are deliberately separate:

- **Signed iOS/TestFlight/App Store:** Xcode Cloud. It regenerates the native project with
  [`ci_post_clone.sh`](../../mobile/ios/ci_scripts/ci_post_clone.sh), signs it, and uploads it to
  TestFlight. EAS Build and EAS Submit are not the iOS release path.
- **iOS JavaScript/assets:** EAS Update on the `production` channel, guarded by the installed
  archive's native fingerprint. An OTA never replaces a required native build.
- **Unsigned iOS:** the `mobile-v*` SideStore release and GitHub Pages source.
- **Android:** the manually dispatched `mobile-android.yml` workflow (APK by default, AAB for a
  tagged release).

As of 2026-07-17, the Apple Developer membership, App Store Connect app, Xcode Cloud workflow,
production OTA bootstrap, and TestFlight OTA delivery have all been exercised. TestFlight build
74 was the installed binary used while verifying the latest OTA fixes. Do not interpret that as
proof that every future native or OTA release is compatible; run the checks below for each one.

## Checked-in release configuration

- [x] Bundle ID `dev.adulari.forge` and Apple Team ID `95VXXPD28Y` are set in
      [`app.config.ts`](../../mobile/app.config.ts).
- [x] EAS project ID, production update URL/channel, and fingerprint runtime policy are set in
      `app.config.ts`.
- [x] Camera, photo-library, microphone, Face ID, document access, App Group, Live Activity, and
      export-compliance declarations are present.
- [x] The privacy manifest is declared through `ios.privacyManifests` and checked in as
      [`PrivacyInfo.xcprivacy`](../../mobile/PrivacyInfo.xcprivacy).
- [x] `expo-splash-screen` has light and dark launch backgrounds.
- [x] `assets/icon.png` is a 1024×1024 RGB PNG with no alpha channel.
- [x] The generated main iOS target currently requires iOS 16.4. The widget target requires 16.1;
      release manifests must advertise the stricter 16.4 minimum.
- [x] Anonymous usage counters have no installation/person identifier, disable GeoIP enrichment,
      and can be disabled in Settings → Privacy. See [Anonymous usage statistics](../telemetry.md).

Re-run `cd mobile && npm run check && npx expo config --type public` whenever the Expo config or
native dependencies change. A successful JavaScript check does not compile Swift or prove Apple
signing; Xcode Cloud is the native iOS gate.

## Privacy and security declarations

Keep App Store Connect's App Privacy answers synchronized with the implementation:

- Declare **Product Interaction → Analytics**, not linked to identity and not used for tracking,
  for the fixed anonymous counters. Forge has no advertising identifier or person profile.
- The daemon pairing URL is a bearer credential stored in the device keychain. Forge does not send
  it to Apple, Expo, PostHog, or the APNs relay. When the user chooses `--anywhere`, however, the
  configured tunnel provider terminates TLS and can technically observe the pairing token and
  relayed session traffic. Do not promise that a public tunnel is end-to-end encrypted to the
  daemon.
- Session data normally travels only between the app and the user's daemon. It can still pass
  through a user-selected HTTPS tunnel. Updated daemons send no session text to the stock public
  APNs relay. During the CLI upgrade window an older daemon can still transmit rich alert text, but
  the public relay does not log it and replaces it before forwarding to APNs.
- The default hosted APNs relay receives an opaque APNs device or Live Activity token plus either
  a static generic alert or the small Live Activity status object
  (`busy`/`waiting`/`cost_usd`/`context_tokens`/`context_limit`). It does not receive session titles,
  questions, permission prompts, response/error snippets, local session IDs, daemon tokens,
  transcripts, source files, paths, commands, or API credentials. An explicitly configured private
  relay may
  retain rich alert text, and bring-your-own Apple credentials bypass every relay. See
  [ADR-0012](../architecture/decisions/0012-hosted-apns-relay.md).

Before App Store submission:

- [ ] Complete/update the App Privacy questionnaire with those exact facts.
- [ ] Re-run the privacy-manifest/config review after adding any native package, permission,
      analytics property, relay payload, or stored data.
- [ ] Ensure the privacy policy and App Review notes disclose the hosted notification relay and
      user-selected tunnel rather than making a blanket “no third party sees session data” claim.

## Signed iOS and TestFlight (Xcode Cloud)

Xcode Cloud is the only supported signed-iOS build pipeline. The repository's `eas.json` is used
for Expo/Android configuration; its old EAS-managed iOS signing and `eas submit` workflow have
been removed.

- [x] Xcode Cloud is connected to `adulari/forge` and has produced processed TestFlight builds.
- [x] `mobile/ios/ci_scripts/ci_post_clone.sh` installs dependencies, regenerates the Expo iOS
      project, applies a monotonically increasing build number, and installs Pods.
- [x] A production-channel OTA bootstrap binary has been installed and an OTA has been observed on
      TestFlight build 74.
- [ ] For every native release, trigger Xcode Cloud with
      `scripts/trigger-ios-build.mjs`; set `TESTFLIGHT_GROUPS` so the processed build is assigned to
      testers automatically. If the build was triggered separately, run
      `scripts/testflight-assign-group.mjs` or dispatch `testflight-autogroup.yml`.
- [ ] Confirm the Xcode Cloud archive succeeded, Apple finished processing it, the intended beta
      group can install it, and its marketing/build versions exceed the previously uploaded pair.
- [ ] On a physical device, smoke-test cold launch, pairing, session creation/chat, permission and
      question prompts, background/foreground reconnect, attachments, voice input, Face ID,
      widget, Live Activity, and native push.
- [ ] Keep App Store screenshots, description, keywords, Developer Tools category, support URL,
      copyright, age rating, and reviewer contact current.

The App Store reviewer has no Forge daemon by default. Provide a short-lived, sandboxed demo:

1. Start `forge serve --anywhere` in a throwaway repository with no valuable credentials or
   network access.
2. Put its pairing URL/QR and a concise workflow in App Review Information.
3. Explain that the app is a client for the user's self-hosted Forge daemon, that the selected
   tunnel terminates TLS, and that the default APNs relay processes the limited payload documented
   above.
4. Rotate the daemon token before and after the review window with
   `forge serve --rotate-token`.

## OTA updates

EAS Update publishes iOS JavaScript and assets only. The workflow validates the complete diff from
the reviewed installed-runtime baseline, not merely the files in the triggering commit.

- [ ] Repository variable `IOS_OTA_COMPATIBLE_BASE_SHA` is not configured yet. Set it to the
      reviewed installed archive's source commit before the next production OTA; keep the workflow
      fail-closed until that baseline and `IOS_OTA_RUNTIME_VERSION` are both verified.
- [ ] Before publication, confirm the baseline is the newest commit whose complete mobile diff has
      been reviewed as compatible with the installed Xcode archive, and the runtime value is that
      archive's exact Expo fingerprint. Initially the baseline is the archive source commit; it may
      advance without a rebuild only after that compatibility review.
- [ ] Use OTA only when every iOS-relevant change since the baseline is JavaScript, assets, tests,
      docs, Android-only config, or independent Tauri-shell code accepted by
      `eas-update.yml`'s guard. Native dependencies, Expo/RN upgrades, config plugins,
      entitlements, permissions, `app.config.ts`, lockfile changes, or generated iOS code require a
      new Xcode Cloud binary and a new baseline.
- [ ] Verify the workflow's EAS update ID/channel/runtime, then launch the installed TestFlight app
      twice and verify the expected behavior. Keep the preceding embedded bundle usable in case the
      update cannot load.
- [ ] Use `gh workflow run eas-update.yml --ref main` only to recover a failed automatic
      publication from the current `main`; manual dispatch cannot bypass the compatibility guard.

## APNs, widgets, and Live Activities

Regular users do not receive the Apple `.p8` key. Native push works through the hosted relay by
default; operators can instead configure a direct APNs key, run a private relay, or set
`FORGE_APNS_DISABLE_RELAY=1`.

- [x] Push Notifications and App Groups are represented in the native configuration for
      `dev.adulari.forge` / `group.dev.adulari.forge`.
- [x] The production relay implementation and deployment runbook live in
      [`crates/forge-relay`](../../crates/forge-relay/README.md).
- [ ] For each rollout, check the relay health endpoint and server logs/metrics without exposing
      device tokens or notification text.
- [ ] Verify on a production-signed physical-device build: permission request, token registration,
      question/permission/final/error notifications, tap routing, Live Activity start/update/end,
      widget refresh, unsubscribe, malformed-token rejection, expired-token cleanup after Apple's
      `410 Unregistered` or invalid-token 400 response, and relay outage fallback.
- [ ] Rotate/revoke the APNs key immediately if it is ever copied into source control, logs,
      artifacts, chat, or an untrusted host. Keep the key only in the relay secret store or a
      tightly permissioned direct-mode host.

TestFlight and App Store builds use APNs production. Xcode Debug builds use APNs sandbox. A relay
health check proves reachability and configuration, but not delivery to a real device.

## SideStore and Android

- [ ] For a `mobile-v*` release, confirm the unsigned `Forge.ipa`, source JSON, and install page are
      all public and mutually consistent (version, size, SHA/source URL, icon, and minimum iOS
      16.4). Open the stable Pages source URL from a clean client before announcing it.
- [ ] Install that IPA through SideStore and complete the same core pairing/chat/reconnect smoke
      test. SideStore validates the unsigned distribution path, not App Store signing.
- [ ] Dispatch `mobile-android.yml` for the intended ref. Install and smoke-test the APK; for a
      tagged release, also verify the AAB artifact/release asset and Play Console upload if used.
- [ ] Do not call an OTA, a SideStore IPA, or a GitHub Android artifact a complete mobile release
      until its matching install path has been exercised on a device.

## Final App Store submission gate

- [ ] All automated checks and the Xcode Cloud archive are green at the exact submitted commit.
- [ ] A production-signed build passed the physical-device matrix above.
- [ ] Privacy answers, relay/tunnel disclosure, screenshots, metadata, age rating, support URL,
      reviewer notes, and demo daemon are current.
- [ ] TestFlight feedback has no unresolved release-blocking issue.
- [ ] Select the verified build in App Store Connect and submit it manually for review.

App Store metadata, reviewer access, physical-device behavior, Apple processing, and final review
cannot be certified from repository CI alone. Keep those boxes unchecked until a human verifies
the exact release candidate.
