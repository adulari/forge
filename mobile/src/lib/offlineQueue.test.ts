import { describe, expect, it } from "vitest";

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

  it("keys the queue by baseUrl+sessionId so new-session and the chat screen agree", () => {
    expect(offlineQueueKey("http://localhost:1234", "abc123")).toBe(
      "forge.offlineQueue:http://localhost:1234:abc123",
    );
    expect(offlineQueueKey(null, "abc123")).toBe("forge.offlineQueue:unknown:abc123");
  });
});
