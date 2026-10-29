import { describe, expect, it } from "vitest";

import type { HistoryRow } from "./api";
import { windowHolds } from "./historyWindow";

const row = (seq: number): HistoryRow => ({ seq, role: "user", content: "", model: null, created_at: 0, visibility: "llm" }) as HistoryRow;

describe("windowHolds", () => {
  it("is true when the newest-first window spans the row", () => {
    expect(windowHolds([row(40), row(35), row(30)], 35)).toBe(true);
    expect(windowHolds([row(40), row(35), row(30)], 30)).toBe(true);
  });

  it("is false for a page that does not reach the row, or an empty one", () => {
    expect(windowHolds([row(500), row(480)], 35)).toBe(false);
    expect(windowHolds([row(40), row(35)], 5)).toBe(false);
    expect(windowHolds([], 5)).toBe(false);
  });
});
