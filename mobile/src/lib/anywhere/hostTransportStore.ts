// Per-host "transport for new sessions" preference. The managed service has no field for
// this, so it is a genuine device-local preference: AnywhereProvider.selectHost consults it
// when activating a host, which is what decides the transport a new session is created on.
// Metadata only — no tokens, no key material, never anything from a host response.
import AsyncStorage from "@react-native-async-storage/async-storage";

import type { TransportPreference } from "./types";

const KEY_PREFIX = "forge.anywhere.hostTransport.v1";

export type HostTransportPreferences = Record<string, TransportPreference>;

export interface HostTransportStorage {
  getItem(key: string): Promise<string | null>;
  setItem(key: string, value: string): Promise<void>;
}

export function isTransportPreference(value: unknown): value is TransportPreference {
  return value === "auto" || value === "direct" || value === "anywhere";
}

export async function readHostTransportPreferences(
  accountIdHex: string,
  storage: HostTransportStorage = AsyncStorage,
): Promise<HostTransportPreferences> {
  const encoded = await storage.getItem(storageKey(accountIdHex));
  if (!encoded) return {};
  try {
    const value: unknown = JSON.parse(encoded);
    if (typeof value !== "object" || value === null || Array.isArray(value)) return {};
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).filter(
        (entry): entry is [string, TransportPreference] => isTransportPreference(entry[1]),
      ),
    );
  } catch {
    return {};
  }
}

export async function writeHostTransportPreferences(
  accountIdHex: string,
  preferences: HostTransportPreferences,
  storage: HostTransportStorage = AsyncStorage,
): Promise<void> {
  await storage.setItem(storageKey(accountIdHex), JSON.stringify(preferences));
}

function storageKey(accountIdHex: string): string {
  if (!/^[0-9a-f]{32}$/i.test(accountIdHex)) throw new Error("invalid Anywhere account ID");
  return `${KEY_PREFIX}.${accountIdHex.toLowerCase()}`;
}
