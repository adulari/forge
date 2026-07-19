import * as SecureStore from "expo-secure-store";

import { parseStoredRemoteJobs, type AnywhereJobStore, type PendingRemoteJob } from "./anywhereJobs";

const KEY = "forge.anywhere.outgoing-jobs.v1";

/** Native pending ciphertext lives in SecureStore, never config or ordinary async storage. */
export function anywhereJobStore(): AnywhereJobStore {
  return {
    async load(): Promise<PendingRemoteJob[]> {
      const encoded = await SecureStore.getItemAsync(KEY);
      return encoded == null ? [] : parseStoredRemoteJobs(encoded);
    },
    async save(jobs): Promise<void> {
      await SecureStore.setItemAsync(KEY, JSON.stringify(jobs), {
        keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY,
      });
    },
  };
}
