// Native (iOS/Android) credential storage — thin wrapper over expo-secure-store.
// Metro resolves `secureStore.web.ts` instead of this file on the web platform
// (expo-secure-store itself is a no-op stub there).
import * as SecureStore from "expo-secure-store";

export async function getSecureItem(key: string): Promise<string | null> {
  return SecureStore.getItemAsync(key);
}

export async function setSecureItem(key: string, value: string): Promise<void> {
  await SecureStore.setItemAsync(key, value);
}

export async function deleteSecureItem(key: string): Promise<void> {
  await SecureStore.deleteItemAsync(key);
}
