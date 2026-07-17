// Config for the "widget" target (@bacons/apple-targets) — covers BOTH the Home Screen widget
// and the Live Activity/Dynamic Island, since Apple ships them in the same WidgetKit extension
// bundle (see targets/widget/ForgeWidgetBundle.swift). Mirrors the App Group entitlement
// declared on the main app in app.config.ts so the two processes can share data via
// NSUserDefaults(suiteName:) — see ForgeSharedData.swift.
/** @type {import('@bacons/apple-targets/app.plugin').ConfigFunction} */
module.exports = (config) => ({
  type: "widget",
  name: "ForgeWidget",
  displayName: "Forge",
  colors: {
    // Emberline theme's ember500 (mobile/src/theme/tokens.ts) — the app's accent color.
    $accent: "#F5761A",
  },
  frameworks: ["SwiftUI", "WidgetKit", "ActivityKit", "AppIntents"],
  entitlements: {
    "com.apple.security.application-groups":
      config.ios.entitlements["com.apple.security.application-groups"],
  },
  // Live Activities require iOS 16.1+; this is also this project's confirmed SDK 57 minimum
  // (see docs/mobile/APP_STORE_CHECKLIST.md §3).
  deploymentTarget: "16.1",
});
