import { describe, expect, it } from "vitest";

import type { HistoryRow } from "./api";
import { answerAlreadyShown, fillHistoryGap, historyHasRealContentSince, mergeNewestHistoryPage, shouldRetryEmptyHistory } from "./historyMerge";

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

describe("fillHistoryGap", () => {
  const store = Array.from({ length: 40 }, (_, i) => row(40 - i)); // 40..1, newest first
  const before = async (seq: number) => store.filter((r) => r.seq < seq).slice(0, 5);

  it("pages back until the live page joins the loaded history, so nothing is dropped", async () => {
    const fresh = store.slice(0, 5); // 40..36
    const rows = await fillHistoryGap(fresh, 22, before);
    expect(rows.at(-1)!.seq).toBeLessThanOrEqual(22);
    const data = { pages: [page(22, 21, 20), page(19, 18)], pageParams: [undefined, 20] };
    const merged = mergeNewestHistoryPage(data, rows)!;
    expect(merged.pages).toHaveLength(2);
    expect(merged.pages[0][0].seq).toBe(40);
    expect(merged.pages[1]).toBe(data.pages[1]);
  });

  it("stops after its page budget and when nothing is loaded yet", async () => {
    const fresh = store.slice(0, 5);
    expect((await fillHistoryGap(fresh, 1, before, 2)).length).toBe(15);
    expect(await fillHistoryGap(fresh, undefined, before)).toBe(fresh);
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

describe("shouldRetryEmptyHistory", () => {
  const base = { historySettled: true, rowsEmpty: true, snapshotHasContent: false, blindAttempts: 0 };

  it("never retries before the first fetch has settled", () => {
    expect(shouldRetryEmptyHistory({ ...base, historySettled: false })).toBe(false);
  });

  it("never retries once real rows have landed", () => {
    expect(shouldRetryEmptyHistory({ ...base, rowsEmpty: false, blindAttempts: 99 })).toBe(false);
  });

  it("retries without limit once the live snapshot disagrees — a slow turn can take a while", () => {
    expect(shouldRetryEmptyHistory({ ...base, snapshotHasContent: true, blindAttempts: 1000 })).toBe(true);
  });

  it("retries a bounded number of times with no snapshot evidence either way", () => {
    // Reproduces the reported bug: a freshly created session's chat screen fetches history
    // before the daemon has started (or this device's WS has even reached) the turn, so there
    // is no live snapshot to disagree with `useHistory` coming back empty.
    expect(shouldRetryEmptyHistory({ ...base, blindAttempts: 0 })).toBe(true);
    expect(shouldRetryEmptyHistory({ ...base, blindAttempts: 3 })).toBe(true);
  });

  it("stops blind retries at the cap — a genuinely empty session must not poll forever", () => {
    expect(shouldRetryEmptyHistory({ ...base, blindAttempts: 4 })).toBe(false);
    expect(shouldRetryEmptyHistory({ ...base, blindAttempts: 4 }, 2)).toBe(false);
    expect(shouldRetryEmptyHistory({ ...base, blindAttempts: 1 }, 2)).toBe(true);
  });
});

describe("answerAlreadyShown", () => {
  const msg = (seq: number, role: HistoryRow["role"], content: string) => ({ kind: "message", row: { ...row(seq, content), role } });
  const tool = { kind: "toolGroup" };

  it("finds a segment that already landed as a row of the running turn", () => {
    const entries = [tool, msg(3, "assistant", "I'll read the file first."), msg(2, "user", "fix it")];
    expect(answerAlreadyShown(entries, "I'll read the file first.\n")).toBe(true);
  });

  it("accepts a stream tail of a longer row, and a row without the stream's reasoning", () => {
    expect(answerAlreadyShown([msg(3, "assistant", "a long answer ending here")], "ending here")).toBe(true);
    expect(answerAlreadyShown([msg(3, "assistant", "answer")], "<think>hm</think>answer")).toBe(true);
  });

  it("looks past a follow-up the running turn picked up", () => {
    const entries = [tool, msg(4, "user", "also this"), tool, msg(3, "assistant", "Test file plus run."), msg(2, "user", "go")];
    expect(answerAlreadyShown(entries, "Test file plus run.")).toBe(true);
  });

  it("compares only the newest assistant row", () => {
    const entries = [msg(4, "user", "again"), msg(3, "assistant", "Done."), msg(1, "assistant", "Earlier text.")];
    expect(answerAlreadyShown(entries, "Earlier text.")).toBe(false);
  });

  it("is false for text that has not landed or is blank", () => {
    expect(answerAlreadyShown([msg(3, "assistant", "other")], "new text")).toBe(false);
    expect(answerAlreadyShown([msg(3, "assistant", "other")], "  ")).toBe(false);
  });
});
