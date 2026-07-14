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
If it were, the mismatched update simply wouldn't be offered to that binary — but more
importantly, any JS that calls a newly-added native API would **silently no-op** (the
native module isn't present), so the feature would appear broken with no error. The
fingerprint policy makes this a hard gate: bump the native layer → fingerprint changes →
OTA updates for the old fingerprint stop being served → you must ship a new binary.

## CI workflow

`.github/workflows/eas-update.yml` runs on every push to `main` that touches app source
(`mobile/src/**`, `mobile/app.config.ts`, `mobile/package.json`, `mobile/assets/**`).
It publishes to the `production` channel with:

```
npx eas-cli@latest update --channel production --environment production --platform ios --message "<commit subject>" --non-interactive
```

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
- Installed production binaries are subscribed to the `production` channel and receive
  these updates automatically.
