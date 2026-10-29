import { describe, expect, it } from "vitest";

import { flushUnsubscribes, queueUnsubscribe, type OutboxStore, withPendingUnsubscribe } from "./pushOutbox";

const memoryStore = (initial: string | null = null): OutboxStore & { value: string | null } => {
  const store = {
    value: initial,
    read: async () => store.value,
    write: async (next: string) => {
      store.value = next;
    },
  };
  return store;
};

const a = { baseUrl: "http://a", body: { device_token: "t1" } };
const b = { baseUrl: "http://b", body: { endpoint: "https://push/x" } };

describe("push unsubscribe outbox", () => {
  it("does not queue the same request twice", () => {
    expect(withPendingUnsubscribe([a], { ...a })).toEqual([a]);
    expect(withPendingUnsubscribe([a], b)).toEqual([a, b]);
  });

  it("keeps only the newest requests", () => {
    let queue: ReturnType<typeof withPendingUnsubscribe> = [];
    for (let i = 0; i < 30; i++) queue = withPendingUnsubscribe(queue, { baseUrl: "http://a", body: { device_token: `t${i}` } });
    expect(queue).toHaveLength(20);
    expect(queue[19].body).toEqual({ device_token: "t29" });
  });

  it("replays queued requests and keeps the ones that fail", async () => {
    const store = memoryStore();
    await queueUnsubscribe(store, a);
    await queueUnsubscribe(store, b);
    const sent: string[] = [];
    const sentCount = await flushUnsubscribes(store, async (item) => {
      if (item.baseUrl === "http://b") throw new Error("offline");
      sent.push(item.baseUrl);
    });
    expect(sentCount).toBe(1);
    expect(sent).toEqual(["http://a"]);
    expect(JSON.parse(store.value ?? "[]")).toEqual([b]);

    expect(await flushUnsubscribes(store, async () => undefined)).toBe(1);
    expect(JSON.parse(store.value ?? "[]")).toEqual([]);
  });

  it("treats a corrupt store as empty", async () => {
    const store = memoryStore("{not json");
    expect(await flushUnsubscribes(store, async () => undefined)).toBe(0);
    await queueUnsubscribe(store, a);
    expect(JSON.parse(store.value ?? "[]")).toEqual([a]);
  });
});
