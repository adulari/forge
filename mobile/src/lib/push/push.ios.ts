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
import * as SecureStore from "expo-secure-store";
import { Platform } from "react-native";

import { PushRegistrationError } from "./pushErrors";
import {
  PUSH_KEY_ACCESS_GROUP,
  PUSH_KEY_NAME,
  PUSH_KEY_SERVICE,
  loadOrCreateDeviceKey,
  publicKeys,
  type PushKeys,
} from "./pushE2e";
import { subscribePush as apiSubscribePush, unsubscribePush as apiUnsubscribePush } from "../api";
import { pushRegistrationDue, withRegistration, withoutRegistration, type RegistrationLedger } from "./pushRegistrations";

export type PushSubscriptionState = "unsupported" | "subscribed" | "unsubscribed";

const SUBSCRIBED_TOKEN_KEY = "forge.apnsDeviceToken";
const REGISTRATIONS_KEY = "forge.apnsRegistrations.v1";
const APP_LOCK_KEY = "forge.appLock";

// The private half lives in a keychain access group shared with the Notification Service
// Extension, which is the only other process that can read it. AFTER_FIRST_UNLOCK because the
// extension runs while the phone is locked; THIS_DEVICE_ONLY so it never reaches a backup.
const KEY_OPTIONS: SecureStore.SecureStoreOptions = {
  keychainService: PUSH_KEY_SERVICE,
  accessGroup: PUSH_KEY_ACCESS_GROUP,
  keychainAccessible: SecureStore.AFTER_FIRST_UNLOCK_THIS_DEVICE_ONLY,
};

/** The device's public notification key, or null where the keychain group is unavailable (an
 * unsigned sideload has no entitlements). Registration then proceeds without a key and the lock
 * screen keeps the generic alert, which is the pre-extension behaviour. */
async function devicePushKeys(): Promise<PushKeys | null> {
  try {
    // With the app lock on, OS surfaces stay generic (docs/features/push-notification-tool.md), so
    // no key is registered and the daemon sends the generic alert. Toggling the lock re-registers.
    if ((await AsyncStorage.getItem(APP_LOCK_KEY)) === "true") return null;
    const key = await loadOrCreateDeviceKey({
      load: () => SecureStore.getItemAsync(PUSH_KEY_NAME, KEY_OPTIONS),
      save: (value) => SecureStore.setItemAsync(PUSH_KEY_NAME, value, KEY_OPTIONS),
    });
    return publicKeys(key);
  } catch {
    return null;
  }
}

async function readLedger(): Promise<RegistrationLedger> {
  try {
    const raw = await AsyncStorage.getItem(REGISTRATIONS_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : {};
    return typeof parsed === "object" && parsed !== null && !Array.isArray(parsed) ? (parsed as RegistrationLedger) : {};
  } catch {
    return {};
  }
}

async function writeLedger(ledger: RegistrationLedger): Promise<void> {
  await AsyncStorage.setItem(REGISTRATIONS_KEY, JSON.stringify(ledger)).catch(() => undefined);
}

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
  } catch (err) {
    // This rejects when iOS itself refuses to register the app for remote notifications, which is
    // NOT the same as the user declining — permission is already granted by the time we get here.
    // In practice it means the build lacks a usable `aps-environment` entitlement. Returning
    // "unsubscribed" here (as this did) made the toggle silently snap back while the UI advised
    // checking a permission that was fine.
    throw new PushRegistrationError(
      `iOS refused to register for push (${err instanceof Error ? err.message : String(err)}). This build may be missing its push entitlement.`,
    );
  }

  // Unlike the permission/device-token failures above (genuinely "no subscription possible"),
  // a subscribe-call failure is usually transient (daemon unreachable) — let it propagate
  // instead of collapsing into "unsubscribed" so the caller can tell the two apart and show
  // an accurate message (mirrors push.web.ts, which never catches this call either).
  const keys = await devicePushKeys();
  await apiSubscribePush(baseUrl, {
    device_token: deviceToken,
    environment: currentEnvironment(),
    ...(keys ? { keys } : {}),
  });

  await AsyncStorage.setItem(SUBSCRIBED_TOKEN_KEY, deviceToken);
  await writeLedger(withRegistration(await readLedger(), baseUrl, deviceToken, keys));
  return "subscribed";
}

export async function disablePush(baseUrl: string): Promise<PushSubscriptionState> {
  if (!isPushSupported()) return "unsupported";

  const storedToken = await AsyncStorage.getItem(SUBSCRIBED_TOKEN_KEY);
  await AsyncStorage.removeItem(SUBSCRIBED_TOKEN_KEY);
  await writeLedger(withoutRegistration(await readLedger(), baseUrl));
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

/**
 * Bring an existing subscription up to date: a device that enabled push before sealed alerts
 * existed has no key on its daemon, and an APNs token can rotate. Runs when the app opens and
 * only for a host this device already subscribed to (or, for a subscription that predates the
 * ledger, the active host, where the toggle was used). A failure is silent; the next open retries.
 */
export async function refreshPushRegistration(activeBaseUrl: string | null): Promise<void> {
  if (!isPushSupported() || !activeBaseUrl) return;
  try {
    const stored = await AsyncStorage.getItem(SUBSCRIBED_TOKEN_KEY);
    if (!stored) return;
    const permissions = await Notifications.getPermissionsAsync();
    if (!permissions.granted) return;
    const token = (await Notifications.getDevicePushTokenAsync()).data;
    if (typeof token !== "string") return;
    const keys = await devicePushKeys();
    let ledger = await readLedger();
    const hosts = Object.keys(ledger).length > 0 ? Object.keys(ledger) : [activeBaseUrl];
    for (const baseUrl of hosts) {
      if (!pushRegistrationDue(ledger, baseUrl, token, keys)) continue;
      await apiSubscribePush(baseUrl, {
        device_token: token,
        environment: currentEnvironment(),
        ...(keys ? { keys } : {}),
      });
      ledger = withRegistration(ledger, baseUrl, token, keys);
      await writeLedger(ledger);
    }
    if (token !== stored) await AsyncStorage.setItem(SUBSCRIBED_TOKEN_KEY, token);
  } catch {
    // Offline or the daemon predates sealed alerts; try again on the next open.
  }
}
