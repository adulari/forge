import {
  parseStoredCredentials,
  type AnywhereCredentialStore,
  type StoredAnywhereCredentials,
} from "./anywhereCredentialTypes";

const DATABASE = "forge-anywhere-credentials";
const STORE = "private-state";
const WRAPPING_KEY = "wrapping-key";
const CIPHERTEXT = "credentials";

interface WrappedCredentials {
  iv: ArrayBuffer;
  ciphertext: ArrayBuffer;
}

/**
 * Browser keys are encrypted under a non-extractable AES-GCM CryptoKey. IndexedDB can structured-
 * clone the CryptoKey but JavaScript cannot export its raw bytes, so copying storage records alone
 * does not expose the enrolled device keys.
 */
export function anywhereCredentialStore(): AnywhereCredentialStore {
  return {
    async load(): Promise<StoredAnywhereCredentials | null> {
      const database = await openDatabase();
      const wrapped = await get<WrappedCredentials>(database, CIPHERTEXT);
      if (wrapped == null) return null;
      const key = await get<CryptoKey>(database, WRAPPING_KEY);
      if (key == null) throw new Error("Forge Anywhere browser wrapping key is missing");
      const plaintext = await crypto.subtle.decrypt(
        { name: "AES-GCM", iv: wrapped.iv },
        key,
        wrapped.ciphertext,
      );
      return parseStoredCredentials(new TextDecoder().decode(plaintext));
    },
    async save(credentials): Promise<void> {
      const database = await openDatabase();
      const key = await wrappingKey(database);
      const iv = crypto.getRandomValues(new Uint8Array(12));
      const plaintext = new TextEncoder().encode(JSON.stringify(credentials));
      const ciphertext = await crypto.subtle.encrypt({ name: "AES-GCM", iv }, key, plaintext);
      await put(database, CIPHERTEXT, {
        iv: iv.buffer.slice(iv.byteOffset, iv.byteOffset + iv.byteLength),
        ciphertext,
      } satisfies WrappedCredentials);
    },
    async clear(): Promise<void> {
      const database = await openDatabase();
      await Promise.all([remove(database, CIPHERTEXT), remove(database, WRAPPING_KEY)]);
    },
  };
}

async function wrappingKey(database: IDBDatabase): Promise<CryptoKey> {
  const existing = await get<CryptoKey>(database, WRAPPING_KEY);
  if (existing != null) return existing;
  const generated = await crypto.subtle.generateKey(
    { name: "AES-GCM", length: 256 },
    false,
    ["encrypt", "decrypt"],
  );
  await put(database, WRAPPING_KEY, generated);
  return generated;
}

function openDatabase(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DATABASE, 1);
    request.onupgradeneeded = () => {
      if (!request.result.objectStoreNames.contains(STORE)) request.result.createObjectStore(STORE);
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error("open Anywhere credential database"));
  });
}

function get<T>(database: IDBDatabase, key: string): Promise<T | null> {
  return new Promise((resolve, reject) => {
    const request = database.transaction(STORE, "readonly").objectStore(STORE).get(key);
    request.onsuccess = () => resolve((request.result as T | undefined) ?? null);
    request.onerror = () => reject(request.error ?? new Error("read Anywhere credential state"));
  });
}

function put(database: IDBDatabase, key: string, value: unknown): Promise<void> {
  return new Promise((resolve, reject) => {
    const transaction = database.transaction(STORE, "readwrite");
    transaction.objectStore(STORE).put(value, key);
    transaction.oncomplete = () => resolve();
    transaction.onerror = () => reject(transaction.error ?? new Error("write Anywhere credential state"));
  });
}

function remove(database: IDBDatabase, key: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const transaction = database.transaction(STORE, "readwrite");
    transaction.objectStore(STORE).delete(key);
    transaction.oncomplete = () => resolve();
    transaction.onerror = () => reject(transaction.error ?? new Error("delete Anywhere credential state"));
  });
}

export type { AnywhereCredentialStore, PendingDeviceRevocation, StoredAnywhereCredentials } from "./anywhereCredentialTypes";
