import * as Notifications from "expo-notifications";
import * as SecureStore from "expo-secure-store";
import { Platform } from "react-native";

import {
  anywherePushStatus,
  disableAnywherePush as disableWith,
  enableAnywherePush as enableWith,
  observeAnywherePush,
  type AnywherePushApi,
  type AnywherePushPlatform,
  type AnywherePushRegistration,
  type AnywherePushStatus,
  type AnywherePushStorage,
} from "./anywherePushCore";

export type { AnywherePushStatus } from "./anywherePushCore";

const STORAGE_KEY = "forge.anywhere.push.v1";

Notifications.setNotificationHandler({
  handleNotification: async () => ({
    shouldPlaySound: false,
    shouldSetBadge: false,
    shouldShowBanner: true,
    shouldShowList: true,
  }),
});

const storage: AnywherePushStorage = {
  async load() {
    const value = await SecureStore.getItemAsync(STORAGE_KEY);
    if (!value) return null;
    try {
      const parsed: unknown = JSON.parse(value);
      if (!isRegistration(parsed)) {
        await SecureStore.deleteItemAsync(STORAGE_KEY);
        return null;
      }
      return parsed;
    } catch {
      await SecureStore.deleteItemAsync(STORAGE_KEY);
      return null;
    }
  },
  async save(registration) {
    await SecureStore.setItemAsync(STORAGE_KEY, JSON.stringify(registration), {
      keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY,
    });
  },
  async clear() {
    await SecureStore.deleteItemAsync(STORAGE_KEY);
  },
};

const platform: AnywherePushPlatform = {
  supported: () => Platform.OS === "ios",
  async permission() {
    const permission = await Notifications.getPermissionsAsync();
    return permission.granted ? "granted" : permission.status === "denied" ? "denied" : "undetermined";
  },
  async requestPermission() {
    const permission = await Notifications.requestPermissionsAsync();
    return permission.granted ? "granted" : permission.status === "denied" ? "denied" : "undetermined";
  },
  async deviceToken() {
    const token = await Notifications.getDevicePushTokenAsync();
    if (typeof token.data !== "string") throw new Error("APNs did not return a device token");
    return token.data;
  },
  environment: () => (__DEV__ ? "sandbox" : "production"),
  observeRefresh(onRefresh) {
    const received = Notifications.addNotificationReceivedListener(() => onRefresh());
    const opened = Notifications.addNotificationResponseReceivedListener(() => onRefresh());
    return () => {
      received.remove();
      opened.remove();
    };
  },
};

export async function getAnywherePushStatus(): Promise<AnywherePushStatus> {
  return anywherePushStatus(platform, storage);
}

export async function enableAnywherePush(api: AnywherePushApi): Promise<AnywherePushStatus> {
  return enableWith(platform, storage, api);
}

export async function disableAnywherePush(api: AnywherePushApi): Promise<AnywherePushStatus> {
  return disableWith(platform, storage, api);
}

export function observeAnywherePushRefresh(onRefresh: () => void): () => void {
  return observeAnywherePush(platform, onRefresh);
}

export async function clearAnywherePushState(): Promise<void> {
  await storage.clear();
}

function isRegistration(value: unknown): value is AnywherePushRegistration {
  if (!value || typeof value !== "object") return false;
  const registration = value as Partial<AnywherePushRegistration>;
  return typeof registration.subscriptionId === "string"
    && /^[0-9a-f]{32}$/.test(registration.subscriptionId)
    && (registration.environment === "sandbox" || registration.environment === "production");
}
