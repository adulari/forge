import AsyncStorage from "@react-native-async-storage/async-storage";

import type { AnywhereHost } from "./anywhereApi";

const HOST_CACHE_KEY_PREFIX = "forge.anywhere.hostMetadata.v2";

export interface HostMetadataStorage {
  getItem(key: string): Promise<string | null>;
  setItem(key: string, value: string): Promise<void>;
  removeItem(key: string): Promise<void>;
}

/**
 * Stores an explicit metadata-only projection bound to its routing account ID. Tokens, key
 * material, recovery phrases, and pairing state can never enter this cache through a host response.
 */
export async function writeAnywhereHostCache(
  accountIdHex: string,
  hosts: readonly AnywhereHost[],
  storage: HostMetadataStorage = AsyncStorage,
): Promise<void> {
  const metadata = hosts.map(({ id, device_id, name, created_at, last_heartbeat_at }) => ({
    id,
    device_id,
    name,
    created_at,
    last_heartbeat_at,
  }));
  const accountId = normalizedAccountId(accountIdHex);
  await storage.setItem(cacheKey(accountId), JSON.stringify({ version: 2, account_id: accountId, hosts: metadata }));
}

export async function readAnywhereHostCache(
  accountIdHex: string,
  storage: HostMetadataStorage = AsyncStorage,
): Promise<AnywhereHost[]> {
  const encoded = await storage.getItem(cacheKey(accountIdHex));
  if (!encoded) return [];
  try {
    const value: unknown = JSON.parse(encoded);
    if (
      !isRecord(value)
      || value.version !== 2
      || value.account_id !== normalizedAccountId(accountIdHex)
      || !Array.isArray(value.hosts)
    ) return [];
    return value.hosts.filter(isAnywhereHost);
  } catch {
    return [];
  }
}

export async function clearAnywhereHostCache(
  accountIdHex: string,
  storage: HostMetadataStorage = AsyncStorage,
): Promise<void> {
  await storage.removeItem(cacheKey(accountIdHex));
}

function cacheKey(accountIdHex: string): string {
  return `${HOST_CACHE_KEY_PREFIX}.${normalizedAccountId(accountIdHex)}`;
}

function normalizedAccountId(accountIdHex: string): string {
  if (!/^[0-9a-f]{32}$/i.test(accountIdHex)) throw new Error("invalid Anywhere account ID");
  return accountIdHex.toLowerCase();
}

function isAnywhereHost(value: unknown): value is AnywhereHost {
  return isRecord(value)
    && typeof value.id === "string"
    && typeof value.device_id === "string"
    && typeof value.name === "string"
    && typeof value.created_at === "string"
    && (value.last_heartbeat_at === null || typeof value.last_heartbeat_at === "string")
    && Object.keys(value).every((key) => ["id", "device_id", "name", "created_at", "last_heartbeat_at"].includes(key));
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
