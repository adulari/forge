import AsyncStorage from "@react-native-async-storage/async-storage";
import { xchacha20poly1305 } from "@noble/ciphers/chacha.js";
import { hkdf } from "@noble/hashes/hkdf.js";
import { sha256 } from "@noble/hashes/sha2.js";

import { base64Url, fromBase64Url } from "./anywhereApi";
import { secureRandomBytes } from "./secureRandom";
import { decodeEnvelope, openEnvelope, bytesFromHex, bytesToHex } from "./transport/anywhereEnvelope";
import type { StoredAnywhereCredentials } from "./transport/anywhereCredentialTypes";

export interface AnywhereSyncChange {
  cursor: number;
  device_id: string;
  signing_public_key: string;
  record_kind: string;
  stable_id: string;
  revision: number;
  logical_clock: number;
  operation: "upsert" | "tombstone";
  base_hash: string | null;
  content_hash: string;
  ciphertext_bytes: number;
  ciphertext_sha256: string;
  download_url: string;
  created_at: number;
}

export interface AnywhereSyncFeed { version: 1; changes: AnywhereSyncChange[]; next_cursor: number }

export interface DecryptedSyncRecord {
  stable_id: string;
  kind: string;
  revision: number;
  logical_clock: number;
  device_id: number[];
  operation: "upsert" | "tombstone";
  base_hash: number[] | null;
  content_hash: number[];
  payload: number[];
}

export interface OfflineHistoryEntry {
  cursor: number;
  record: DecryptedSyncRecord;
  createdAt: number;
  conflict: boolean;
}

export interface CacheStorage { getItem(key: string): Promise<string | null>; setItem(key: string, value: string): Promise<void>; removeItem(key: string): Promise<void> }

const CACHE_PREFIX = "forge.anywhere.history.v1.";
const CACHE_CONTEXT = new TextEncoder().encode("forge-anywhere/v1/device-history-cache");

export async function fetchSyncFeed(serviceUrl: string, accessToken: string, cursor = 0, fetcher: typeof fetch = fetch): Promise<AnywhereSyncFeed> {
  const url = new URL("/v1/sync/changes", serviceUrl);
  url.searchParams.set("cursor", String(Math.max(0, cursor)));
  url.searchParams.set("limit", "500");
  const response = await fetcher(url.toString(), { headers: { accept: "application/json", authorization: `Bearer ${accessToken}` }, cache: "no-store" });
  if (!response.ok) throw new Error(`Encrypted history request failed (${response.status})`);
  const feed = await response.json() as AnywhereSyncFeed;
  if (feed.version !== 1 || !Array.isArray(feed.changes) || !Number.isSafeInteger(feed.next_cursor)) throw new Error("Invalid encrypted history response");
  return feed;
}

export async function decryptSyncChange(change: AnywhereSyncChange, credentials: StoredAnywhereCredentials, fetcher: typeof fetch = fetch): Promise<OfflineHistoryEntry> {
  const response = await fetcher(change.download_url, { headers: { accept: "application/vnd.forge-anywhere" }, cache: "no-store", referrerPolicy: "no-referrer" });
  if (!response.ok) throw new Error(`Encrypted history download failed (${response.status})`);
  const bytes = new Uint8Array(await response.arrayBuffer());
  if (bytes.length !== change.ciphertext_bytes || base64Url(sha256(bytes)) !== change.ciphertext_sha256) throw new Error("Encrypted history object hash does not match the feed");
  const envelope = decodeEnvelope(bytes);
  if (envelope.metadata.kind !== 4 || envelope.metadata.recipientKind !== 3
    || bytesToHex(envelope.metadata.accountId) !== credentials.accountIdHex
    || bytesToHex(envelope.metadata.recipientId) !== credentials.accountIdHex
    || bytesToHex(envelope.metadata.senderDeviceId) !== change.device_id) throw new Error("Encrypted history routing metadata does not match the feed");
  const epochKey = credentials.dataKeyEpochs?.[String(envelope.metadata.keyEpoch)]
    ?? (envelope.metadata.keyEpoch === credentials.keyEpoch ? credentials.accountDataKeyHex : undefined);
  if (epochKey == null) throw new Error("Encrypted history uses an unavailable key epoch");
  const opened = openEnvelope(bytes, bytesFromHex(epochKey), fromBase64Url(change.signing_public_key));
  const record = JSON.parse(new TextDecoder().decode(opened.plaintext)) as DecryptedSyncRecord;
  validateRecord(record, change);
  return { cursor: change.cursor, record, createdAt: change.created_at * 1000, conflict: false };
}

export function markSyncConflicts(entries: OfflineHistoryEntry[]): OfflineHistoryEntry[] {
  const fileBases = new Map<string, Set<string>>();
  for (const entry of entries) {
    if (entry.record.kind !== "file" || entry.record.operation !== "upsert") continue;
    const identity = `${entry.record.stable_id}\u0000${entry.record.revision}`;
    const ancestry = `${bytesKey(entry.record.base_hash)}:${bytesKey(entry.record.content_hash)}`;
    const set = fileBases.get(identity) ?? new Set<string>();
    set.add(ancestry);
    fileBases.set(identity, set);
  }
  return entries.map((entry) => ({ ...entry, conflict: entry.record.kind === "file"
    && (fileBases.get(`${entry.record.stable_id}\u0000${entry.record.revision}`)?.size ?? 0) > 1 }));
}

export function syncPayloadText(entry: OfflineHistoryEntry): string {
  if (entry.record.operation === "tombstone") return "Deleted";
  const text = new TextDecoder().decode(new Uint8Array(entry.record.payload));
  try { return JSON.stringify(JSON.parse(text), null, 2); } catch { return text; }
}

export async function writeOfflineHistory(credentials: StoredAnywhereCredentials, entries: OfflineHistoryEntry[], storage: CacheStorage = AsyncStorage): Promise<void> {
  const key = cacheKey(credentials);
  const nonce = secureRandomBytes(24);
  const plaintext = new TextEncoder().encode(JSON.stringify(entries.slice(-500)));
  const ciphertext = xchacha20poly1305(key, nonce).encrypt(plaintext);
  await storage.setItem(cacheName(credentials), JSON.stringify({ version: 1, nonce: base64Url(nonce), ciphertext: base64Url(ciphertext) }));
}

export async function readOfflineHistory(credentials: StoredAnywhereCredentials, storage: CacheStorage = AsyncStorage): Promise<OfflineHistoryEntry[]> {
  const stored = await storage.getItem(cacheName(credentials));
  if (stored == null) return [];
  try {
    const value = JSON.parse(stored) as { version: number; nonce: string; ciphertext: string };
    if (value.version !== 1) throw new Error("version");
    const plaintext = xchacha20poly1305(cacheKey(credentials), fromBase64Url(value.nonce)).decrypt(fromBase64Url(value.ciphertext));
    const entries = JSON.parse(new TextDecoder().decode(plaintext)) as OfflineHistoryEntry[];
    return Array.isArray(entries) ? entries : [];
  } catch { await storage.removeItem(cacheName(credentials)); return []; }
}

export async function clearOfflineHistory(credentials: StoredAnywhereCredentials, storage: CacheStorage = AsyncStorage): Promise<void> {
  await storage.removeItem(cacheName(credentials));
}

function cacheName(credentials: StoredAnywhereCredentials): string { return `${CACHE_PREFIX}${credentials.accountIdHex}.${credentials.deviceIdHex}`; }
function cacheKey(credentials: StoredAnywhereCredentials): Uint8Array { return hkdf(sha256, bytesFromHex(credentials.accountDataKeyHex), bytesFromHex(credentials.deviceIdHex), CACHE_CONTEXT, 32); }
function bytesKey(value: number[] | null): string { return value == null ? "root" : base64Url(new Uint8Array(value)); }

function validateRecord(record: DecryptedSyncRecord, change: AnywhereSyncChange): void {
  if (record.stable_id !== change.stable_id || record.kind !== change.record_kind || record.revision !== change.revision
    || record.logical_clock !== change.logical_clock || record.operation !== change.operation
    || bytesKey(record.base_hash) !== (change.base_hash ?? "root") || base64Url(new Uint8Array(record.content_hash)) !== change.content_hash
    || bytesToHex(new Uint8Array(record.device_id)) !== change.device_id || base64Url(sha256(new Uint8Array(record.payload))) !== change.content_hash
    || (record.operation === "tombstone" && record.payload.length !== 0)) throw new Error("Decrypted history does not match authenticated metadata");
}
