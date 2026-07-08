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
      "expo-splash-screen",
      {
        backgroundColor: "#16161c",
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
