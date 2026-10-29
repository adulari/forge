import { describe, expect, it } from "vitest";

import { hasMoreDetail, threadErrorKey, threadErrorTitle } from "./threadError";
import type { ThreadError } from "./ws";

const error = (extra: Partial<ThreadError> = {}): ThreadError => ({
  category: "network",
  summary: "connection reset",
  details: "connection reset\nat provider",
  ...extra,
});

describe("threadError", () => {
  it("titles by category and falls back for an unknown one", () => {
    expect(threadErrorTitle(error())).toBe("Network error");
    expect(threadErrorTitle(error({ category: "rate_limit" }))).toBe("Rate limited");
    expect(threadErrorTitle(error({ category: "future" as ThreadError["category"] }))).toBe("Turn failed");
  });

  it("keys a failure by what it says", () => {
    expect(threadErrorKey(error())).toBe(threadErrorKey(error()));
    expect(threadErrorKey(error())).not.toBe(threadErrorKey(error({ summary: "timed out" })));
  });

  it("offers a fold only when the details add something", () => {
    expect(hasMoreDetail(error())).toBe(true);
    expect(hasMoreDetail(error({ details: "connection reset" }))).toBe(false);
  });
});
