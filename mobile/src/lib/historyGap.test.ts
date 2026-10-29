import { describe, expect, it } from "vitest";

import { gapAckKey, liveGapNotice, missingRowsNotice, needsAck } from "./historyGap";

describe("historyGap", () => {
  it("describes a missed reconnect with the count", () => {
    const notice = liveGapNotice({ count: 24, reason: "replay_window_exceeded" }, 31);
    expect(notice.title).toBe("History incomplete");
    expect(notice.detail).toContain("24 live updates");
    expect(liveGapNotice({ count: 1, reason: "x" }, 2).detail).toContain("1 live update passed");
    expect(notice.id).not.toBe(liveGapNotice({ count: 24, reason: "x" }, 40).id);
  });

  it("describes missing rows", () => {
    expect(missingRowsNotice({ count: 2, reason: "missing_rows" }).detail).toContain("2 messages are missing");
    expect(missingRowsNotice({ count: 1, reason: "missing_rows" }).detail).toContain("1 message is missing");
  });

  it("only a larger stored gap than the acknowledged one needs showing", () => {
    expect(needsAck(null, 0)).toBe(false);
    expect(needsAck({ count: 2, reason: "missing_rows" }, 0)).toBe(true);
    expect(needsAck({ count: 2, reason: "missing_rows" }, 2)).toBe(false);
    expect(needsAck({ count: 3, reason: "missing_rows" }, 2)).toBe(true);
  });

  it("keys the acknowledgement per host and session", () => {
    expect(gapAckKey("http://h", "s1")).not.toBe(gapAckKey("http://h", "s2"));
  });
});
