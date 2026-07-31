import type { ExpoConfig } from "expo/config";

const BUNDLE_ID = "dev.adulari.forge";
// Shared container for the app <-> widget/Live Activity extension (mobile/targets/widget) — the
// convention @bacons/apple-targets' own docs recommend (`group.<bundle-id>`).
const APP_GROUP = `group.${BUNDLE_ID}`;

const config: ExpoConfig = {
  name: "Forge",
  slug: "forge",
  scheme: "forge",
  version: "1.0.1",
  // Native archives calculate their own fingerprint. OTA publishing supplies the exact
  // fingerprint extracted from the installed Xcode Cloud archive through this override.
  runtimeVersion: process.env.EXPO_RUNTIME_VERSION_OVERRIDE ?? { policy: "fingerprint" },
  updates: {
    url: "https://u.expo.dev/e1d145b5-344e-4147-ba35-5f0b993b4c8c",
    requestHeaders: {
      "expo-channel-name": "production",
    },
    enabled: true,
    checkAutomatically: "ON_LOAD",
    fallbackToCacheTimeout: 0,
  },
  orientation: "portrait",
  icon: "./assets/icon.png",
  userInterfaceStyle: "automatic",
  // Root native view background — the surface between iOS tearing down the launch storyboard and
  // React's first painted frame. Left unset, RN's root view keeps its own default (white), which
  // is the "default Expo screen" that flashed before the Face ID prompt on every cold start.
  //
  // This key ONLY takes effect if `expo-system-ui` is installed. Without it, prebuild's
  // withIosRootViewBackgroundColor takes its `else` branch — `warnSystemUIMissing`, a warning in
  // the build log — and writes nothing, so this value sat here inert for its whole life
  // (verified: `RCTRootViewBackgroundColor` was absent from the generated Info.plist, and is
  // 0xff09090b now that the package is a dependency). Removing expo-system-ui silently restores
  // the white flash.
  //
  // Single static value, no light/dark split, so it matches the dark splash and the app's primary
  // appearance. Value = theme/tokens.ts darkTokens.bg0 ("#09090B"), the same token Screen.tsx
  // paints every screen's root with; keep in sync by hand, since this file can't import from
  // src/theme without pulling RN into the (Node-executed) config context. RESIDUAL: a device in
  // light mode gets one dark frame here against the light "#F5F4F1" splash. Fixing that properly
  // needs an appearance-aware root colour, which is a custom native mod rather than a config key.
  backgroundColor: "#09090B",
  ios: {
    bundleIdentifier: BUNDLE_ID,
    supportsTablet: true,
    appleTeamId: "95VXXPD28Y",
    entitlements: {
      "com.apple.security.application-groups": [APP_GROUP],
      // APNs environment. expo-notifications' plugin writes `development` here and EAS Build
      // rewrites it to `production` from the signing credentials — but Forge builds iOS on Xcode
      // Cloud, so nothing performed that rewrite and every distribution archive shipped a
      // development entitlement. A distribution-signed app whose entitlement says `development`
      // cannot register with production APNs: `getDevicePushTokenAsync()` rejects, and the
      // in-app notification toggle silently refused to turn on while iOS Settings still showed
      // the permission as granted (permission and entitlement are unrelated).
      //
      // The previous fix was to hand-edit the generated pbxproj so Release pointed at a separate
      // `Forge.Release.entitlements`. prebuild deletes that file and repoints Release back at
      // this one on every single build, so it never survived to a real archive. Declaring it here
      // is the only form that does. `scripts/ci/verify-ios-entitlements.sh` fails the build if
      // prebuild ever produces anything other than `production`.
      //
      // Consequence, deliberately accepted: local Xcode debug builds would need `development` to
      // sign against a development profile. Nothing builds this app that way — Xcode Cloud always
      // produces distribution archives and mobile-sidestore.yml archives unsigned
      // (CODE_SIGNING_ALLOWED=NO), so a single production value is correct for every real build.
      "aps-environment": "production",
    },
    infoPlist: {
      NSSupportsLiveActivities: true,
      NSSupportsLiveActivitiesFrequentUpdates: true,
      // Standard HTTPS/TLS to the user's own daemon only — no proprietary/non-standard crypto,
      // so this is exempt from export compliance docs (Apple's own "false" = "no" branch).
      ITSAppUsesNonExemptEncryption: false,
      NSCameraUsageDescription:
        "Scan a Forge pairing QR code to connect to your server.",
      NSFaceIDUsageDescription: "Unlock Forge with Face ID.",
      NSPhotoLibraryUsageDescription:
        "Forge lets you attach photos from your library to a session.",
      NSDocumentsFolderUsageDescription:
        "Forge lets you attach documents to a session.",
      NSMicrophoneUsageDescription:
        "Forge lets you dictate messages by voice instead of typing.",
    },
    // SDK 57 privacy manifest mechanism: a `PrivacyInfo.xcprivacy` file at the project
    // root, wired in via `ios.privacyManifests` (expo-build-properties-free path — expo
    // itself merges this into the generated Xcode project during prebuild). See
    // PrivacyInfo.xcprivacy for the declared data use / required-reason APIs.
    privacyManifests: {
      NSPrivacyTracking: false,
      NSPrivacyTrackingDomains: [],
      NSPrivacyCollectedDataTypes: [
        {
          NSPrivacyCollectedDataType: "NSPrivacyCollectedDataTypeProductInteraction",
          NSPrivacyCollectedDataTypeLinked: false,
          NSPrivacyCollectedDataTypeTracking: false,
          NSPrivacyCollectedDataTypePurposes: ["NSPrivacyCollectedDataTypePurposeAnalytics"],
        },
      ],
      NSPrivacyAccessedAPITypes: [
        {
          NSPrivacyAccessedAPIType: "NSPrivacyAccessedAPICategoryUserDefaults",
          NSPrivacyAccessedAPITypeReasons: ["CA92.1"],
        },
      ],
    },
  },
  android: {
    package: BUNDLE_ID,
    // React Native's debug manifest contributes this special permission during manifest merging
    // even for the EAS release variant. Forge never draws over other apps, so keep it out of APKs
    // explicitly instead of shipping an unnecessary Play-policy-sensitive capability.
    blockedPermissions: ["android.permission.SYSTEM_ALERT_WINDOW"],
    adaptiveIcon: {
      // Same pre-Machined hex as the old root/splash backgroundColor above — kept in sync
      // with it (theme/tokens.ts darkTokens.bg0, "#09090B") rather than left to drift now
      // that those are fixed.
      backgroundColor: "#09090B",
      foregroundImage: "./assets/android-icon-foreground.png",
      monochromeImage: "./assets/android-icon-monochrome.png",
    },
    predictiveBackGestureEnabled: false,
  },
  web: {
    favicon: "./assets/favicon.png",
    bundler: "metro",
    output: "static",
  },
  plugins: [
    "expo-router",
    "expo-secure-store",
    "expo-status-bar",
    [
      "expo-notifications",
      {
        // Android status-bar icons must be an all-white transparent mask. The generated
        // monochrome Forge mark already has exactly that shape; without this config Android
        // falls back to an unsuitable launcher icon.
        icon: "./assets/android-notification-icon.png",
        color: "#C65A1E",
        defaultChannel: "forge-sessions",
        mode: "production",
      },
    ],
    "./plugins/with-share-extension-display-name.cjs",
    [
      "expo-sharing",
      {
        ios: {
          enabled: true,
          extensionBundleIdentifier: `${BUNDLE_ID}.sharing`,
          appGroupId: APP_GROUP,
          activationRule: {
            supportsText: true,
            supportsWebUrlWithMaxCount: 1,
          },
        },
        android: {
          enabled: true,
          singleShareMimeTypes: ["text/plain"],
          multipleShareMimeTypes: [],
        },
      },
    ],
    "@bacons/apple-targets",
    [
      "expo-font",
      {
        fonts: [
          "./assets/Geist-Regular.ttf",
          "./assets/Geist-Medium.ttf",
          "./assets/Geist-SemiBold.ttf",
          "./assets/Geist-Bold.ttf",
          "./assets/GeistMono-Regular.ttf",
          "./assets/GeistMono-Medium.ttf",
          "./assets/GeistMono-SemiBold.ttf",
        ],
      },
    ],
    [
      "expo-camera",
      {
        cameraPermission: "Scan a Forge pairing QR code to connect to your server.",
      },
    ],
    [
      "expo-image-picker",
      {
        photosPermission:
          "Forge lets you attach photos from your library to a session.",
      },
    ],
    [
      "expo-audio",
      {
        microphonePermission:
          "Forge lets you dictate messages by voice instead of typing.",
        // Voice input is a short one-shot recording the user stops explicitly — it never
        // continues once the app backgrounds, so the plugin's default background-audio
        // entitlements (UIBackgroundModes + an Android foreground media-playback service)
        // would only add unused permission surface.
        enableBackgroundRecording: false,
        enableBackgroundPlayback: false,
      },
    ],
    [
      "expo-splash-screen",
      {
        // Default (light) variant uses the light theme's bg0 (theme/tokens.ts
        // lightTokens.bg0, "#F5F4F1" — the same token Screen.tsx paints every screen's
        // root with) instead of the dark bg — this was hardcoded to the dark color for
        // both variants, so light-theme users got a dark flash on every cold start.
        //
        // Each variant gets its OWN mark, because one ink cannot serve both backgrounds.
        // splash-icon.png carries the mark's own ember ramp, which measures 7.13:1 against the
        // dark bg at its darkest stop and 10.55:1 at its lightest. That same ramp on the light
        // bg would be 2.54:1 — the mistake the previous pair shipped, where the dark mark
        // measured 1.41:1 on light and was invisible rather than merely dim. So
        // splash-icon-light.png re-inks the identical geometry in ember700 ("#964916") at
        // 5.85:1, which keeps the brand colour instead of falling back to near-black.
        //
        // Both are generated from scripts/brand/forge-mark.svg by scripts/gen-brand-assets.py;
        // do not edit them by hand. Both are baked in at prebuild, so an OTA can never update
        // them — a native build is required for either to reach a device.
        backgroundColor: "#F5F4F1",
        image: "./assets/splash-icon-light.png",
        imageWidth: 200,
        // theme/tokens.ts darkTokens.bg0 ("#09090B") — was a stale pre-Machined hex.
        dark: { backgroundColor: "#09090B", image: "./assets/splash-icon.png" },
      },
    ],
  ],
  extra: {
    eas: {
      projectId: "e1d145b5-344e-4147-ba35-5f0b993b4c8c",
    },
  },
};

export default config;
