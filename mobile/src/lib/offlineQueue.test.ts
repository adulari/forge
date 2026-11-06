import { describe, expect, it } from "vitest";

import { purgeServerStorage, readOfflineQueue, readGapAck, type KeyValueStorage } from "./serverStorageKeys";
import { gapAckKey, legacyGapAckKey } from "./historyGap";
import { buildInitialPromptQueue, newClientMsgId, offlineQueueKey, parseOfflineQueue, queuedPromptInputs } from "./offlineQueue";

describe("offline prompt queue", () => {
  it("migrates legacy strings and tolerates corrupt storage", () => {
    expect(parseOfflineQueue('["one",{"text":"two","attachments":[]} ]')).toEqual([
      { text: "one", attachments: [], clientMsgId: expect.any(String) },
      { text: "two", attachments: [], clientMsgId: expect.any(String) },
    ]);
    expect(parseOfflineQueue("not-json")).toEqual([]);
  });

  it("replays multiple prompts in FIFO order with their own attachments", () => {
    const inputs = queuedPromptInputs([
      { text: "first", attachments: [{ path: "a.png", image: true }] },
      { text: "second", attachments: [] },
      { text: "third", attachments: [{ path: "c.txt", image: false }] },
    ]);
    expect(inputs.map((input) => input.kind === "prompt" ? input.text : "")).toEqual(["first", "second", "third"]);
    expect(inputs[2]).toMatchObject({ kind: "prompt", attachments: [{ path: "c.txt", image: false }] });
  });
});

describe("client message ids", () => {
  it("replays a queued prompt with the id it was composed with, on every flush", () => {
    const queue = [{ text: "a", attachments: [], clientMsgId: "m-fixed" }];
    const first = queuedPromptInputs(queue);
    const second = queuedPromptInputs(queue);
    expect(first[0]).toMatchObject({ kind: "prompt", client_msg_id: "m-fixed" });
    expect(second).toEqual(first);
  });

  it("keeps the stored id across a storage round trip and gives legacy records a stable one", () => {
    const stored = JSON.stringify([{ text: "a", attachments: [], clientMsgId: "m-1" }, "legacy"]);
    const parsed = parseOfflineQueue(stored);
    expect(parsed[0].clientMsgId).toBe("m-1");
    expect(parsed[1].clientMsgId).toBeTruthy();
    const again = parseOfflineQueue(JSON.stringify(parsed));
    expect(queuedPromptInputs(again)).toEqual(queuedPromptInputs(parsed));
  });

  it("gives distinct prompts distinct ids", () => {
    expect(newClientMsgId()).not.toBe(newClientMsgId());
  });
});

describe("initial prompt queue (new-session -> chat handoff)", () => {
  it("queues trimmed task text as a single prompt with no attachments", () => {
    expect(buildInitialPromptQueue("  reply with pong  ")).toEqual([
      { text: "reply with pong", attachments: [], clientMsgId: expect.any(String) },
    ]);
  });

  it("queues nothing for empty or whitespace-only text", () => {
    expect(buildInitialPromptQueue("")).toEqual([]);
    expect(buildInitialPromptQueue("   \n\t ")).toEqual([]);
  });

  it("keys the queue by baseUrl+sessionId without embedding the baseUrl", () => {
    const url = "http://localhost:1234/SECRETTOKEN";
    const key = offlineQueueKey(url, "abc123");
    expect(key).not.toContain("SECRETTOKEN");
    expect(key).not.toContain("localhost");
    expect(key).toBe(offlineQueueKey(url, "abc123"));
    expect(key).not.toBe(offlineQueueKey("http://localhost:1234/OTHER", "abc123"));
    expect(gapAckKey(url, "s1")).not.toContain("SECRETTOKEN");
  });
});

function memoryStore(initial: Record<string, string>): KeyValueStorage & { data: Map<string, string> } {
  const data = new Map(Object.entries(initial));
  return {
    data,
    getItem: async (k) => data.get(k) ?? null,
    setItem: async (k, v) => void data.set(k, v),
    removeItem: async (k) => void data.delete(k),
    getAllKeys: async () => [...data.keys()],
    multiRemove: async (ks) => void ks.forEach((k) => data.delete(k)),
  };
}

describe("token-free storage keys", () => {
  const url = "http://h:1/SECRETTOKEN";

  it("moves a legacy queue and ack entry to the new key and deletes the old one", async () => {
    const legacyQueue = `forge.offlineQueue:${url}:s1`;
    const store = memoryStore({ [legacyQueue]: '["hi"]', [legacyGapAckKey(url, "s1")]: "3" });
    expect(await readOfflineQueue(store, url, "s1")).toBe('["hi"]');
    expect(await readGapAck(store, url, "s1")).toBe("3");
    expect(store.data.get(offlineQueueKey(url, "s1"))).toBe('["hi"]');
    expect(store.data.get(gapAckKey(url, "s1"))).toBe("3");
    expect([...store.data.keys()].some((k) => k.includes("SECRETTOKEN"))).toBe(false);
  });

  it("keeps the newer entry when both exist and still deletes the legacy one", async () => {
    const store = memoryStore({ [`forge.offlineQueue:${url}:s1`]: "old", [offlineQueueKey(url, "s1")]: "new" });
    expect(await readOfflineQueue(store, url, "s1")).toBe("new");
    expect(store.data.size).toBe(1);
  });

  it("purges only the removed server's entries, new and legacy", async () => {
    const other = "http://h:1/OTHER";
    const store = memoryStore({
      [offlineQueueKey(url, "s1")]: "a",
      [gapAckKey(url, "s2")]: "1",
      [`forge.offlineQueue:${url}:s3`]: "legacy",
      [legacyGapAckKey(url, "s3")]: "1",
      [offlineQueueKey(other, "s1")]: "keep",
      unrelated: "keep",
    });
    await purgeServerStorage(store, url);
    expect([...store.data.keys()].sort()).toEqual([offlineQueueKey(other, "s1"), "unrelated"].sort());
  });
});
