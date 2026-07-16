import { describe, expect, it } from "vitest";

import { parseOfflineQueue, queuedPromptInputs } from "./offlineQueue";

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
