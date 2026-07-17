import { describe, expect, it } from "vitest";

import { readAnywhereHostCache, writeAnywhereHostCache, type HostMetadataStorage } from "./anywhereHostCache";

function memoryStorage(): HostMetadataStorage & { value: string | null } {
  return {
    value: null,
    async getItem() { return this.value; },
    async setItem(_key, value) { this.value = value; },
    async removeItem() { this.value = null; },
  };
}

describe("Anywhere offline fleet metadata", () => {
  it("round-trips the non-secret host projection for offline display", async () => {
    const storage = memoryStorage();
    const host = { id: "11".repeat(16), device_id: "22".repeat(16), name: "Laptop", created_at: "10", last_heartbeat_at: "20" };
    await writeAnywhereHostCache("aa".repeat(16), [{ ...host, token: "must-not-persist", recovery_words: "must-not-persist" } as typeof host], storage);
    expect(storage.value).not.toContain("must-not-persist");
    await expect(readAnywhereHostCache("aa".repeat(16), storage)).resolves.toEqual([host]);
  });

  it("rejects cache rows with unexpected fields", async () => {
    const storage = memoryStorage();
    storage.value = JSON.stringify({ version: 2, account_id: "aa".repeat(16), hosts: [{ id: "a", device_id: "b", name: "Host", created_at: "1", last_heartbeat_at: null, token: "leak" }] });
    await expect(readAnywhereHostCache("aa".repeat(16), storage)).resolves.toEqual([]);
  });

  it("never returns stale hosts after a failed clear and account switch", async () => {
    const storage = memoryStorage();
    const host = { id: "11".repeat(16), device_id: "22".repeat(16), name: "Old laptop", created_at: "10", last_heartbeat_at: null };
    await writeAnywhereHostCache("aa".repeat(16), [host], storage);
    storage.removeItem = async () => { throw new Error("disk unavailable"); };
    await expect(readAnywhereHostCache("bb".repeat(16), storage)).resolves.toEqual([]);
  });
});
