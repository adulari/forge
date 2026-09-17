import { describe, expect, it } from "vitest";

import { buildInitialPromptQueue, offlineQueueKey, parseOfflineQueue, queuedPromptInputs } from "./offlineQueue";

describe("offline prompt queue", () => {
  it("migrates legacy strings and tolerates corrupt storage", () => {
    expect(parseOfflineQueue('["one",{"text":"two","attachments":[]} ]')).toEqual([
      { text: "one", attachments: [] },
      { text: "two", attachments: [] },
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

describe("initial prompt queue (new-session -> chat handoff)", () => {
  it("queues trimmed task text as a single prompt with no attachments", () => {
    expect(buildInitialPromptQueue("  reply with pong  ")).toEqual([
      { text: "reply with pong", attachments: [] },
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
