// Native push for iOS (APNs), via expo-notifications' raw device-token API — NOT the Expo
// push token relay (Forge's own daemon talks directly to Apple, same self-hosted-no-relay
// principle as push.web.ts's Web Push). Metro resolves this file only when bundling for iOS
// (Android/Tauri desktop still get push.ts's no-op); see index.ts's barrel comment.
//
// Subscribe flow: request notification permission → Notifications.getDevicePushTokenAsync()
// (the raw APNs device token, hex string) → POST /api/push/subscribe {device_token,
// environment}. Unsubscribe: POST /api/push/unsubscribe {device_token} (best-effort, mirrors
// push.web.ts). AsyncStorage remembers the last-subscribed token locally, since — unlike the
// browser's PushManager.getSubscription() — there is no OS API to ask "is there already a
// server-side subscription for this device token."
import AsyncStorage from "@react-native-async-storage/async-storage";
import * as Notifications from "expo-notifications";
import { Platform } from "react-native";

import { subscribePush as apiSubscribePush, unsubscribePush as apiUnsubscribePush } from "../api";

export type PushSubscriptionState = "unsupported" | "subscribed" | "unsubscribed";

const SUBSCRIBED_TOKEN_KEY = "forge.apnsDeviceToken";

// TestFlight and App Store builds are both signed for distribution and use APNs'
// PRODUCTION environment — only a local Xcode debug/development build uses sandbox. `__DEV__`
// is exactly that distinction, so it's a reliable (not a guess) way to pick the environment.
function currentEnvironment(): "sandbox" | "production" {
  return __DEV__ ? "sandbox" : "production";
}

// This module only resolves when Metro bundles for iOS (see the file-header comment) — the
// `Platform.OS` check is a defensive belt-and-suspenders, not the real gate.
export function isPushSupported(): boolean {
  return Platform.OS === "ios";
}

export async function initPush(): Promise<void> {
  // No service worker to register — nothing to do until enablePush() is called.
}

export async function getPushStatus(): Promise<PushSubscriptionState> {
  if (!isPushSupported()) return "unsupported";
  const permissions = await Notifications.getPermissionsAsync();
  if (!permissions.granted) return "unsubscribed";
  const storedToken = await AsyncStorage.getItem(SUBSCRIBED_TOKEN_KEY);
  return storedToken ? "subscribed" : "unsubscribed";
}

export async function enablePush(baseUrl: string): Promise<PushSubscriptionState> {
  if (!isPushSupported()) return "unsupported";

  const existing = await Notifications.getPermissionsAsync();
  const granted = existing.granted || (await Notifications.requestPermissionsAsync()).granted;
  if (!granted) return "unsubscribed";

  let deviceToken: string;
  try {
    const token = await Notifications.getDevicePushTokenAsync();
    deviceToken = token.data;
  } catch {
    return "unsubscribed";
  }

  // Unlike the permission/device-token failures above (genuinely "no subscription possible"),
  // a subscribe-call failure is usually transient (daemon unreachable) — let it propagate
  // instead of collapsing into "unsubscribed" so the caller can tell the two apart and show
  // an accurate message (mirrors push.web.ts, which never catches this call either).
  await apiSubscribePush(baseUrl, {
    device_token: deviceToken,
    environment: currentEnvironment(),
  });

  await AsyncStorage.setItem(SUBSCRIBED_TOKEN_KEY, deviceToken);
  return "subscribed";
}

export async function disablePush(baseUrl: string): Promise<PushSubscriptionState> {
  if (!isPushSupported()) return "unsupported";

  const storedToken = await AsyncStorage.getItem(SUBSCRIBED_TOKEN_KEY);
  await AsyncStorage.removeItem(SUBSCRIBED_TOKEN_KEY);
  if (storedToken) {
    try {
      await apiUnsubscribePush(baseUrl, { device_token: storedToken });
    } catch {
      // local unsubscribe already took effect; the daemon prunes a stale token on its next
      // failed send regardless (410 BadDeviceToken/Unregistered), same as Web Push.
    }
  }

  return "unsubscribed";
}
