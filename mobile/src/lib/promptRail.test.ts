import { describe, expect, it } from "vitest";

import type { HistoryRow } from "./api";
import { bucketTicks, mergePrompts, railTicks, railVisible } from "./promptRail";

const row = (seq: number, content: string, extra: Partial<HistoryRow> = {}): HistoryRow =>
  ({ seq, role: "user", content, model: null, created_at: 0, visibility: "llm", ...extra }) as HistoryRow;
const text = (r: HistoryRow) => r.content;

describe("mergePrompts", () => {
  it("keeps real prompts only, de-duplicated and oldest first", () => {
    const remote = [row(8, "second"), row(0, "first")];
    const loaded = [
      row(8, "second"),
      row(9, "assistant", { role: "assistant" }),
      row(10, "nudge", { kind: "nudge" }),
      row(12, "third"),
      row(13, "note", { visibility: "ui" }),
    ];
    expect(mergePrompts(remote, loaded, text).map((p) => p.seq)).toEqual([0, 8, 12]);
    expect(mergePrompts([row(3, "x", { created_at: 99 })], [], text)[0].at).toBe(99);
  });
});

describe("railTicks", () => {
  it("spreads prompts over the session by seq", () => {
    const ticks = railTicks([
      { seq: 0, text: "a", at: 1 },
      { seq: 10, text: "b", at: 2 },
      { seq: 40, text: "c", at: 3 },
    ]);
    expect(ticks.map((t) => t.top)).toEqual([0, 0.25, 1]);
  });

  it("centres a lone prompt and handles none", () => {
    expect(railTicks([{ seq: 4, text: "x", at: 1 }])[0].top).toBe(0.5);
    expect(railTicks([])).toEqual([]);
  });
});

describe("railVisible", () => {
  it("needs two prompts, an expanded layout and a fine pointer", () => {
    expect(railVisible({ promptCount: 2, expanded: true, finePointer: true })).toBe(true);
    expect(railVisible({ promptCount: 1, expanded: true, finePointer: true })).toBe(false);
    expect(railVisible({ promptCount: 5, expanded: false, finePointer: true })).toBe(false);
    expect(railVisible({ promptCount: 5, expanded: true, finePointer: false })).toBe(false);
  });
});

describe("bucketTicks", () => {
  it("leaves a short outline alone and thins a long one to the newest prompt per slot", () => {
    const few = railTicks([0, 5, 9].map((seq) => ({ seq, text: String(seq), at: seq })));
    expect(bucketTicks(few, 10)).toHaveLength(3);
    const many = railTicks(Array.from({ length: 200 }, (_, i) => ({ seq: i * 2, text: String(i), at: i })));
    const thinned = bucketTicks(many, 20);
    expect(thinned.length).toBeLessThanOrEqual(20);
    expect(thinned[thinned.length - 1].seq).toBe(398);
  });
});
