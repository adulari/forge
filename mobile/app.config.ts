import type { ExpoConfig } from "expo/config";

const BUNDLE_ID = "dev.adulari.forge";

const config: ExpoConfig = {
  name: "Forge",
  slug: "forge",
  scheme: "forge",
  version: "1.0.0",
  orientation: "portrait",
  icon: "./assets/icon.png",
  userInterfaceStyle: "automatic",
  backgroundColor: "#16161c",
  ios: {
    bundleIdentifier: BUNDLE_ID,
    supportsTablet: true,
    // TODO(team-id): Apple Developer account pending approval (~12-14h from 2026-07-06).
    // Set appleTeamId once approved. Does NOT block Expo web or SideStore sideload testing.
    infoPlist: {
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
