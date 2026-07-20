import * as SecureStore from "expo-secure-store";

const KEY = "forge.anywhere.pending-enrollment.v1";

export interface AnywhereEnrollmentStore {
  load(): Promise<string | null>;
  save(value: string): Promise<void>;
  clear(): Promise<void>;
}

export function anywhereEnrollmentStore(): AnywhereEnrollmentStore {
  return {
    load: () => SecureStore.getItemAsync(KEY),
    save: (value) => SecureStore.setItemAsync(KEY, value, { keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY }),
    clear: () => SecureStore.deleteItemAsync(KEY),
  };
}
