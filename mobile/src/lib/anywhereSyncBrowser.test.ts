import { describe, expect, it } from "vitest";
import { markSyncConflicts, readOfflineHistory, writeOfflineHistory, type CacheStorage, type OfflineHistoryEntry } from "./anywhereSyncBrowser";
import type { StoredAnywhereCredentials } from "./transport/anywhereCredentialTypes";

const credentials: StoredAnywhereCredentials = { version: 1, accountIdHex: "11".repeat(16), deviceIdHex: "22".repeat(16), signingPrivateKeyHex: "33".repeat(32), exchangePrivateKeyHex: "44".repeat(32), accountDataKeyHex: "55".repeat(32), keyEpoch: 1, accessToken: "secret-access", refreshToken: "secret-refresh", accessExpiresAtMs: 1, nextSequence: "1", acceptedSequences: {}, signingPublicKeys: {} };
const entry: OfflineHistoryEntry = { cursor: 1, createdAt: 10, conflict: false, record: { stable_id: "message-secret-id", kind: "message", revision: 1, logical_clock: 1, device_id: [...new Uint8Array(16).fill(0x22)], operation: "upsert", base_hash: null, content_hash: [...new Uint8Array(32)], payload: [...new TextEncoder().encode("private prompt text")] } };

it("encrypts offline history and never persists plaintext", async () => {
  const map = new Map<string, string>();
  const storage: CacheStorage = { getItem: async (key) => map.get(key) ?? null, setItem: async (key, value) => { map.set(key, value); }, removeItem: async (key) => { map.delete(key); } };
  await writeOfflineHistory(credentials, [entry], storage);
  const raw = [...map.values()].join("");
  expect(raw).not.toContain("private prompt text");
  expect(raw).not.toContain("message-secret-id");
  expect(raw).not.toContain(credentials.accessToken);
  expect(await readOfflineHistory(credentials, storage)).toEqual([entry]);
});

describe("cache failure handling", () => {
  it("fails closed and deletes ciphertext opened with the wrong device key", async () => {
    const map = new Map<string, string>();
    const storage: CacheStorage = { getItem: async () => [...map.values()][0] ?? null, setItem: async (key, value) => { map.set(key, value); }, removeItem: async () => { map.clear(); } };
    await writeOfflineHistory(credentials, [entry], storage);
    expect(await readOfflineHistory({ ...credentials, deviceIdHex: "99".repeat(16) }, storage)).toEqual([]);
    expect(map.size).toBe(0);
  });
});

it("marks only divergent copies at the same file revision as conflicts", () => {
  const file = { ...entry, record: { ...entry.record, kind: "file", base_hash: [1], content_hash: [2] } };
  const divergent = { ...file, cursor: 2, record: { ...file.record, base_hash: [3], content_hash: [4] } };
  const later = { ...file, cursor: 3, record: { ...file.record, revision: 2, base_hash: [2], content_hash: [5] } };
  expect(markSyncConflicts([file, divergent, later]).map((value) => value.conflict)).toEqual([true, true, false]);
});
