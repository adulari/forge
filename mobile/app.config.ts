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
  // FLAGGED (w11-splash): SDK 57 moved splash config to the `expo-splash-screen` config
  // plugin (the classic top-level `splash` key no longer exists on ExpoConfig). That
  // package is NOT present in package.json / node_modules / package-lock.json in this
  // checkout — it is a separate npm package, not bundled inside `expo` itself. Per the W11
  // batch brief this worker must not install packages, so the plugin is intentionally left
  // unwired rather than referencing an unresolvable plugin name (which would break
  // `expo prebuild`/`expo-doctor` even though it wouldn't fail `tsc --noEmit`).
  //
  // To finish this once the package is installed (`npx expo install expo-splash-screen`),
  // add to the `plugins` array below:
  //   [
  //     "expo-splash-screen",
  //     {
  //       backgroundColor: "#16161c",
  //       image: "./assets/splash-icon.png",
  //       imageWidth: 200,
  //       dark: { backgroundColor: "#16161c", image: "./assets/splash-icon.png" },
  //     },
  //   ],
  // `assets/splash-icon.png` already exists in this checkout and is otherwise unused.
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
