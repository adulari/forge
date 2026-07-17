// Metro uses this native implementation on iOS/Android and resolves the `.web.ts` sibling on web.
import * as SecureStore from "expo-secure-store";

import {
  parseStoredCredentials,
  type AnywhereCredentialStore,
  type StoredAnywhereCredentials,
} from "./anywhereCredentialTypes";

const CREDENTIAL_KEY = "forge.anywhere.credentials.v1";

export function anywhereCredentialStore(): AnywhereCredentialStore {
  return {
    async load(): Promise<StoredAnywhereCredentials | null> {
      const value = await SecureStore.getItemAsync(CREDENTIAL_KEY);
      return value == null ? null : parseStoredCredentials(value);
    },
    async save(credentials): Promise<void> {
      await SecureStore.setItemAsync(CREDENTIAL_KEY, JSON.stringify(credentials), {
        keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY,
      });
    },
    async clear(): Promise<void> {
      await SecureStore.deleteItemAsync(CREDENTIAL_KEY);
    },
  };
}

export type { AnywhereCredentialStore, StoredAnywhereCredentials } from "./anywhereCredentialTypes";
