// Config for the Notification Service Extension (@bacons/apple-targets): iOS runs it for every
// push carrying `mutable-content: 1`, before the alert is drawn, so it can replace the generic
// "Open Forge to view an update." with the real text the daemon sealed end-to-end to this device
// (ADR-0012, "End-to-end sealed alerts"). On any failure it leaves the generic alert as it is.
//
// The private key lives in the keychain access group below, written by the main app through
// expo-secure-store (mobile/src/lib/push/push.ios.ts) and read here by ForgePushKey.swift. The
// same group is declared on the main app in app.config.ts. Keychain sharing is an entitlement
// only; unlike App Groups it needs no capability registered on the Apple Developer portal.
/** @type {import('@bacons/apple-targets/app.plugin').ConfigFunction} */
module.exports = () => ({
  type: "notification-service",
  name: "ForgeNotificationService",
  displayName: "Forge Notifications",
  frameworks: ["UserNotifications"],
  entitlements: {
    "keychain-access-groups": ["$(AppIdentifierPrefix)dev.adulari.forge.pushkeys"],
  },
  deploymentTarget: "16.1",
});
