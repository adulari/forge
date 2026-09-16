import { describe, expect, it } from "vitest";

import type { HistoryRow } from "./api";
import { mergeNewestHistoryPage } from "./historyMerge";

const row = (seq: number, content = `r${seq}`): HistoryRow => ({
  seq,
  role: "assistant",
  content,
  model: null,
  created_at: 0,
  visibility: "llm",
});
const page = (...seqs: number[]) => seqs.map((s) => row(s));

describe("mergeNewestHistoryPage", () => {
  it("adds new rows on top and keeps what was already loaded", () => {
    const data = { pages: [page(10, 9, 8), page(7, 6, 5)], pageParams: [undefined, 8] };
    const merged = mergeNewestHistoryPage(data, [row(12), row(11), row(10, "updated")])!;
    expect(merged.pages[0].map((r) => r.seq)).toEqual([12, 11, 10, 9, 8]);
    expect(merged.pages[0][2].content).toBe("updated");
    expect(merged.pages[1]).toBe(data.pages[1]);
    expect(merged.pageParams).toEqual([undefined, 8]);
  });

  it("restarts from the fresh page when it no longer joins up with the loaded ones", () => {
    const data = { pages: [page(10, 9), page(8, 7)], pageParams: [undefined, 9] };
    const merged = mergeNewestHistoryPage(data, page(30, 29))!;
    expect(merged.pages).toEqual([page(30, 29)]);
    expect(merged.pageParams).toEqual([undefined]);
  });

  it("leaves an unloaded or empty result alone", () => {
    expect(mergeNewestHistoryPage(undefined, page(1))).toBeUndefined();
    const data = { pages: [page(3)], pageParams: [undefined] };
    expect(mergeNewestHistoryPage(data, [])).toBe(data);
  });
});
