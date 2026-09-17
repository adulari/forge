import { describe, expect, it } from "vitest";

import type { HistoryRow } from "./api";
import { historyHasRealContentSince, mergeNewestHistoryPage } from "./historyMerge";

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

describe("historyHasRealContentSince", () => {
  it("is false while nothing newer than the baseline has landed yet", () => {
    expect(historyHasRealContentSince(page(3, 2, 1), 3)).toBe(false);
  });

  it("is false when the only new row is blank — a known daemon gap for the turn's prose", () => {
    // Reproduces the reported bug: a turn's assistant row lands with only whitespace content
    // (its prose lived on tool rows this page didn't fetch) — that must not count as "arrived",
    // or the retained live-stream bridge clears and the reply disappears from view entirely.
    expect(historyHasRealContentSince([row(4, "\n\n\n"), row(3), row(2), row(1)], 3)).toBe(false);
  });

  it("is true once a real (non-blank) row lands newer than the baseline", () => {
    expect(historyHasRealContentSince([row(4, "Done."), row(3), row(2), row(1)], 3)).toBe(true);
  });

  it("keeps scanning past a blank row to find real content further ahead", () => {
    expect(
      historyHasRealContentSince([row(5, "All tasks complete."), row(4, "\n\n\n"), row(3)], 3),
    ).toBe(true);
  });
});
