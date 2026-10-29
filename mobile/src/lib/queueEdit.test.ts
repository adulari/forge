import { describe, expect, it } from "vitest";

import { mergeIntoDraft, moveInList, moveInput, moveTarget, queueRows, removeInput, sendNowInput } from "./queueEdit";

describe("queue rows", () => {
  it("lists server prompts before offline ones, each indexed in its own list", () => {
    const rows = queueRows(["a", "b"], [{ text: "c" }]);
    expect(rows.map((r) => [r.source, r.index, r.text])).toEqual([
      ["server", 0, "a"],
      ["server", 1, "b"],
      ["offline", 0, "c"],
    ]);
  });

  it("echoes index and text on every server action", () => {
    const row = queueRows(["a", "b"], [])[1];
    expect(moveInput(row, 0)).toEqual({ kind: "queue_move", index: 1, text: "b", to: 0 });
    expect(sendNowInput(row)).toEqual({ kind: "queue_send_now", index: 1, text: "b" });
    expect(removeInput(row)).toEqual({ kind: "dequeue", index: 1, text: "b" });
  });

  it("refuses to move past either end", () => {
    expect(moveTarget(0, -1, 3)).toBeNull();
    expect(moveTarget(2, 1, 3)).toBeNull();
    expect(moveTarget(1, 1, 3)).toBe(2);
  });
});

describe("local reordering and edit", () => {
  it("moves an item within a list", () => {
    expect(moveInList(["a", "b", "c"], 2, 0)).toEqual(["c", "a", "b"]);
  });

  it("keeps an existing draft above the prompt pulled back", () => {
    expect(mergeIntoDraft("", "queued")).toBe("queued");
    expect(mergeIntoDraft("  ", "queued")).toBe("queued");
    expect(mergeIntoDraft("typing\n", "queued")).toBe("typing\n\nqueued");
  });
});
