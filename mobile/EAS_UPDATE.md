# EAS Update (OTA)

This app uses [EAS Update](https://docs.expo.dev/eas-update/introduction/) to ship
JavaScript and asset changes to installed iOS binaries without a new App Store build.

## What OTA ships

An EAS Update contains **only** the JS bundle and static assets — no native code, no
native modules, no entitlements, no Info.plist changes. The installed app downloads the
new bundle on launch (and on returning to the foreground) and reloads into it. See
`mobile/src/lib/useOtaUpdates.ts` for the client-side check/apply logic.

## runtimeVersion: fingerprint policy

`mobile/app.config.ts` sets:

```ts
runtimeVersion: { policy: "fingerprint" }
```

The fingerprint is a hash of the app's **native** layer — native modules, config
plugins, Xcode project/target/entitlement settings, native dependencies, and the
`expo-updates` config. EAS only serves an OTA update to a binary whose `runtimeVersion`
matches the update's. This is the safety rail that prevents shipping a JS bundle that
references a native API the binary doesn't have.

## RULE: native changes require a new binary build

Any change to the **native** layer forces a **new binary build** (Xcode Cloud / EAS Build),
**not** an OTA update. This includes, but is not limited to:

- adding a **new native module** (anything with native Swift/ObjC/Kotlin/Java code),
- adding a **new Xcode target** or extension (e.g. a new widget / Live Activity),
- adding or changing an **entitlement** or capability,
- adding or changing a **config plugin** (including a plugin's native-side behavior),
- adding a **new native dependency** (a package with native code),
- changing `ios.*` / `android.*` native config in `app.config.ts`.

**An OTA update must NEVER be shipped to a binary whose `runtimeVersion` doesn't match.**
If an update is published with a different fingerprint, EAS does not offer it to the binary, so
the binary remains on its last compatible bundle. The new behavior is unavailable until a binary
with the matching native fingerprint is built. The fingerprint policy makes this a hard gate:
bump the native layer → fingerprint changes → OTA updates for the old fingerprint stop being
served → you must ship a new binary.

## Production channel and the one-time native bootstrap

The app config embeds both the EAS Updates URL and the production channel header:

```ts
updates: {
  requestHeaders: { "expo-channel-name": "production" },
  // ...
}
```

This header is required because Forge's production iOS binary is built by Xcode Cloud rather
than EAS Build. A binary built before `expo-updates` and this channel configuration was added
cannot receive OTA updates. Trigger one signed Xcode Cloud/TestFlight build manually after the
configuration is merged, and verify that the processed build includes the update URL, production
channel header, and fingerprint runtime version before publishing a production update.

After that bootstrap, pushes containing only `mobile/src/**` or `mobile/assets/**` use OTA by
default. The workflow checks the entire range since the last commit explicitly reviewed as
compatible with the installed native runtime. Changes to native dependencies, Expo/RN versions,
config plugins, entitlements, permissions, iOS native files, widgets, or Live Activities require a
new native build. Tests, docs, Android-only config, and the independent Tauri shell may be reviewed
as compatible without rebuilding iOS. The fingerprint policy prevents an update from being offered
to an incompatible binary, while the workflow guard prevents an unreviewed publication attempt.

To verify a TestFlight binary without publishing an update, inspect its embedded `Expo.plist` for
`EXUpdatesURL`, `EXUpdatesRequestHeaders` with `expo-channel-name=production`, and
`EXUpdatesRuntimeVersion`; then launch it twice after a known compatible update exists and confirm
the app reports the new bundle. Do not use this verification procedure to trigger a build or
publish an update automatically.

## CI workflow

`.github/workflows/eas-update.yml` runs on pushes to `main` that touch OTA-safe app source
(`mobile/src/**` or `mobile/assets/**`). It compares the complete range from the repository variable
`IOS_OTA_COMPATIBLE_BASE_SHA` to current `main`, not only the triggering commit. The baseline starts
at the installed archive's source commit and may advance only after every intervening mobile change
has been reviewed as compatible with that archive. This protects against a mixed or earlier commit
quietly changing native/config inputs alongside JavaScript.

When the guard passes, it publishes to the `production` channel with:

```
EXPO_RUNTIME_VERSION_OVERRIDE="<installed archive fingerprint>" ./node_modules/.bin/eas update --channel production --environment production --platform ios --message "<commit SHA> <commit subject>" --non-interactive
```

The workflow reads that value from the `IOS_OTA_RUNTIME_VERSION` GitHub repository variable.
It must be the fingerprint stored inside the installed Xcode Cloud `.xcarchive`, not a locally
recalculated fingerprint. Expo fingerprints include generated native state and can differ between
Apple's archive environment and another macOS runner even from the same source commit. Update this
runtime value after promoting a new native build to TestFlight. Separately advance
`IOS_OTA_COMPATIBLE_BASE_SHA` after a successful compatibility review; both variables fail closed
when missing or invalid.

### `--platform ios` is required

The app has **native-only modules** — the `mobile/modules/live-activity` Swift target and
`expo-audio` voice input. Their config plugin breaks the web bundle export, so letting
`eas-cli` default to exporting every platform would fail the entire publish. The workflow
therefore pins `--platform ios`.

### `EXPO_TOKEN` secret

The workflow authenticates to EAS with the `EXPO_TOKEN` [repo
secret](https://docs.expo.dev/accounts/programmatic-access) — an Expo access token with
permission to publish updates for this project. **This secret must be set** under the
repo's Settings → Secrets and variables → Actions → Repository secrets, or the workflow
will fail at the `eas update` step.

## Channels & branches

- `production` channel ← `main` branch (this workflow).
- Installed production binaries built with the bootstrap configuration are subscribed to the
  `production` channel and receive compatible updates automatically.

## Versioning across release surfaces

The mobile native marketing/build version is independent from Forge's CLI/desktop version. An
OTA can be labeled with the Forge release it carries (for example, `Forge v2.6.3`) while targeting
the currently installed mobile fingerprint. Bump the mobile native version and run Xcode Cloud
only when the native layer changes; do not force a native build for every CLI/desktop release.
