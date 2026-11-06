import { gapAckKey, legacyGapAckKey, GAP_ACK_PREFIX, LEGACY_GAP_ACK_PREFIX } from "./historyGap";
import { serverKeyId } from "./serverKeyId";
import { OFFLINE_QUEUE_PREFIX, offlineQueueKey, legacyOfflineQueueKey } from "./offlineQueue";

export interface KeyValueStorage {
  getItem(key: string): Promise<string | null>;
  setItem(key: string, value: string): Promise<unknown>;
  removeItem(key: string): Promise<unknown>;
  getAllKeys(): Promise<readonly string[]>;
  multiRemove(keys: readonly string[]): Promise<unknown>;
}

/** Move a pre-upgrade entry (keyed by the raw baseUrl) to its token-free key, once. */
export async function readMigrated(store: KeyValueStorage, key: string, legacyKey: string): Promise<string | null> {
  const current = await store.getItem(key);
  const legacy = await store.getItem(legacyKey);
  if (legacy == null) return current;
  if (current == null) await store.setItem(key, legacy);
  await store.removeItem(legacyKey);
  return current ?? legacy;
}

export function readOfflineQueue(store: KeyValueStorage, baseUrl: string | null, sessionId: string): Promise<string | null> {
  return readMigrated(store, offlineQueueKey(baseUrl, sessionId), legacyOfflineQueueKey(baseUrl, sessionId));
}

export function readGapAck(store: KeyValueStorage, baseUrl: string, sessionId: string): Promise<string | null> {
  return readMigrated(store, gapAckKey(baseUrl, sessionId), legacyGapAckKey(baseUrl, sessionId));
}

/** Drop every persisted per-session entry for an unpaired server, including pre-upgrade keys. */
export async function purgeServerStorage(store: KeyValueStorage, baseUrl: string): Promise<void> {
  const id = serverKeyId(baseUrl);
  const prefixes = [
    `${OFFLINE_QUEUE_PREFIX}:${id}:`,
    `${GAP_ACK_PREFIX}:${id}|`,
    `${OFFLINE_QUEUE_PREFIX}:${baseUrl}:`,
    `${LEGACY_GAP_ACK_PREFIX}:${baseUrl}|`,
  ];
  const doomed = (await store.getAllKeys()).filter((key) => prefixes.some((prefix) => key.startsWith(prefix)));
  if (doomed.length > 0) await store.multiRemove(doomed);
}
