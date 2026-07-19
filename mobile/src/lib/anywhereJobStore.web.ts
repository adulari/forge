import { parseStoredRemoteJobs, type AnywhereJobStore, type PendingRemoteJob } from "./anywhereJobs";

const DATABASE = "forge-anywhere-outgoing-jobs";
const STORE = "private-state";
const KEY = "wrapping-key";
const CIPHERTEXT = "ciphertext";

interface WrappedJobs { iv: ArrayBuffer; ciphertext: ArrayBuffer }

/** Web pending jobs are wrapped by a non-extractable WebCrypto key held in IndexedDB. */
export function anywhereJobStore(): AnywhereJobStore {
  return {
    async load(): Promise<PendingRemoteJob[]> {
      const database = await openDatabase();
      const wrapped = await get<WrappedJobs>(database, CIPHERTEXT);
      if (!wrapped) return [];
      const key = await get<CryptoKey>(database, KEY);
      if (!key) throw new Error("Anywhere remote job wrapping key is missing");
      const plaintext = await crypto.subtle.decrypt({ name: "AES-GCM", iv: wrapped.iv }, key, wrapped.ciphertext);
      return parseStoredRemoteJobs(new TextDecoder().decode(plaintext));
    },
    async save(jobs): Promise<void> {
      const database = await openDatabase();
      const key = await wrappingKey(database);
      const iv = crypto.getRandomValues(new Uint8Array(12));
      const plaintext = new TextEncoder().encode(JSON.stringify(jobs));
      const ciphertext = await crypto.subtle.encrypt({ name: "AES-GCM", iv }, key, plaintext);
      await put(database, CIPHERTEXT, { iv: iv.buffer.slice(iv.byteOffset, iv.byteOffset + iv.byteLength), ciphertext } satisfies WrappedJobs);
    },
  };
}

async function wrappingKey(database: IDBDatabase): Promise<CryptoKey> {
  const existing = await get<CryptoKey>(database, KEY);
  if (existing) return existing;
  const generated = await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"]);
  await put(database, KEY, generated);
  return generated;
}

function openDatabase(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DATABASE, 1);
    request.onupgradeneeded = () => { if (!request.result.objectStoreNames.contains(STORE)) request.result.createObjectStore(STORE); };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error("Open Anywhere remote job store"));
  });
}

function get<T>(database: IDBDatabase, key: string): Promise<T | null> {
  return new Promise((resolve, reject) => {
    const request = database.transaction(STORE, "readonly").objectStore(STORE).get(key);
    request.onsuccess = () => resolve((request.result as T | undefined) ?? null);
    request.onerror = () => reject(request.error ?? new Error("Read Anywhere remote job store"));
  });
}

function put(database: IDBDatabase, key: string, value: unknown): Promise<void> {
  return new Promise((resolve, reject) => {
    const transaction = database.transaction(STORE, "readwrite");
    transaction.objectStore(STORE).put(value, key);
    transaction.oncomplete = () => resolve();
    transaction.onerror = () => reject(transaction.error ?? new Error("Write Anywhere remote job store"));
  });
}
