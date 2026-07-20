import type { AnywhereEnrollmentStore } from "./anywhereEnrollmentStore";

const DATABASE = "forge-anywhere-pending-enrollment";
const STORE = "protected-state";
const KEY_NAME = "wrapping-key";
const VALUE_NAME = "pending";

interface WrappedValue { iv: ArrayBuffer; ciphertext: ArrayBuffer }

export function anywhereEnrollmentStore(): AnywhereEnrollmentStore {
  return {
    async load() {
      const database = await openDatabase();
      const wrapped = await get<WrappedValue>(database, VALUE_NAME);
      if (!wrapped) return null;
      const key = await get<CryptoKey>(database, KEY_NAME);
      if (!key) throw new Error("Forge Anywhere enrollment wrapping key is missing");
      const plaintext = await crypto.subtle.decrypt({ name: "AES-GCM", iv: wrapped.iv }, key, wrapped.ciphertext);
      return new TextDecoder().decode(plaintext);
    },
    async save(value) {
      const database = await openDatabase();
      const key = await wrappingKey(database);
      const iv = crypto.getRandomValues(new Uint8Array(12));
      const ciphertext = await crypto.subtle.encrypt({ name: "AES-GCM", iv }, key, new TextEncoder().encode(value));
      await put(database, VALUE_NAME, { iv: iv.buffer.slice(iv.byteOffset, iv.byteOffset + iv.byteLength), ciphertext } satisfies WrappedValue);
    },
    async clear() {
      const database = await openDatabase();
      await remove(database, VALUE_NAME);
    },
  };
}

async function wrappingKey(database: IDBDatabase): Promise<CryptoKey> {
  const existing = await get<CryptoKey>(database, KEY_NAME);
  if (existing) return existing;
  const generated = await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"]);
  await put(database, KEY_NAME, generated);
  return generated;
}

function openDatabase(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DATABASE, 1);
    request.onupgradeneeded = () => { if (!request.result.objectStoreNames.contains(STORE)) request.result.createObjectStore(STORE); };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error("open protected enrollment store"));
  });
}

function get<T>(database: IDBDatabase, key: string): Promise<T | null> {
  return new Promise((resolve, reject) => {
    const request = database.transaction(STORE, "readonly").objectStore(STORE).get(key);
    request.onsuccess = () => resolve((request.result as T | undefined) ?? null);
    request.onerror = () => reject(request.error ?? new Error("read protected enrollment state"));
  });
}

function put(database: IDBDatabase, key: string, value: unknown): Promise<void> {
  return new Promise((resolve, reject) => {
    const transaction = database.transaction(STORE, "readwrite");
    transaction.objectStore(STORE).put(value, key);
    transaction.oncomplete = () => resolve();
    transaction.onerror = () => reject(transaction.error ?? new Error("write protected enrollment state"));
  });
}

function remove(database: IDBDatabase, key: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const transaction = database.transaction(STORE, "readwrite");
    transaction.objectStore(STORE).delete(key);
    transaction.oncomplete = () => resolve();
    transaction.onerror = () => reject(transaction.error ?? new Error("clear protected enrollment state"));
  });
}

export type { AnywhereEnrollmentStore } from "./anywhereEnrollmentStore";
