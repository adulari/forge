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
  runtimeVersion: { policy: "fingerprint" },
  updates: {
    url: "https://u.expo.dev/e1d145b5-344e-4147-ba35-5f0b993b4c8c",
    enabled: true,
    checkAutomatically: "ON_LOAD",
    fallbackToCacheTimeout: 0,
  },
  orientation: "portrait",
  icon: "./assets/icon.png",
  userInterfaceStyle: "automatic",
  backgroundColor: "#16161c",
  ios: {
    bundleIdentifier: BUNDLE_ID,
    supportsTablet: true,
    appleTeamId: "95VXXPD28Y",
    entitlements: {
      "com.apple.security.application-groups": [APP_GROUP],
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
    adaptiveIcon: {
      backgroundColor: "#16161c",
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
    "expo-notifications",
    "@bacons/apple-targets",
    [
      "expo-font",
      {
        fonts: [
          "./assets/JetBrainsMono-Regular.ttf",
          "./assets/JetBrainsMono-Bold.ttf",
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
        // Default (light) variant uses the light theme's bg1 (theme/tokens.ts
        // lightTokens.bg1) instead of the dark bg — this was hardcoded to the dark
        // color for both variants, so light-theme users got a dark flash on every cold
        // start. NOTE: splash-icon.png is a light-gray mark drawn for the dark bg; on
        // this light bg it's low-contrast (near-invisible) rather than wrong-colored —
        // a barely-visible glyph for ~1 frame beats an incongruous dark flash, but a
        // proper light-variant asset (dark-on-transparent) would fix this fully.
        backgroundColor: "#FAF8F4",
        image: "./assets/splash-icon.png",
        imageWidth: 200,
        dark: { backgroundColor: "#16161c", image: "./assets/splash-icon.png" },
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
