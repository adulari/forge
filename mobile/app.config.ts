import type { ExpoConfig } from "expo/config";

const BUNDLE_ID = "dev.adulari.forge";

const config: ExpoConfig = {
  name: "Forge",
  slug: "forge",
  scheme: "forge",
  version: "1.0.0",
  orientation: "portrait",
  icon: "./assets/icon.png",
  userInterfaceStyle: "dark",
  backgroundColor: "#16161c",
  // TODO(w11-splash): SDK 57 moved splash screen config to the expo-splash-screen plugin
  // (classic top-level `splash` key is gone from ExpoConfig). Wire the real splash
  // (bg #16161c, accent mark) via that plugin in Batch 4 (BUILD_PLAN §7, W11).
  ios: {
    bundleIdentifier: BUNDLE_ID,
    supportsTablet: true,
    // TODO(team-id): Apple Developer account pending approval (~12-14h from 2026-07-06).
    // Set appleTeamId once approved. Does NOT block Expo web or SideStore sideload testing.
    infoPlist: {
      NSCameraUsageDescription:
        "Forge uses the camera to scan the QR code printed by `forge serve` to pair with your daemon.",
      NSPhotoLibraryUsageDescription:
        "Forge lets you attach photos from your library to a session.",
      NSDocumentsFolderUsageDescription:
        "Forge lets you attach documents to a session.",
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
  },
  plugins: [
    "expo-router",
    "expo-status-bar",
    "expo-secure-store",
    "expo-font",
    [
      "expo-camera",
      {
        cameraPermission:
          "Forge uses the camera to scan the QR code printed by `forge serve` to pair with your daemon.",
      },
    ],
    [
      "expo-image-picker",
      {
        photosPermission:
          "Forge lets you attach photos from your library to a session.",
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
