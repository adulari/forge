import { describe, expect, it, vi } from "vitest";

vi.mock("expo-router", () => ({ router: { replace: vi.fn() } }));

const { segmentHref } = await import("./useSessionSegment");

describe("segmentHref", () => {
  it("addresses chat at the bare session route and every other segment by suffix", () => {
    expect(segmentHref("abc", "chat")).toBe("/session/abc");
    expect(segmentHref("abc", "tasks")).toBe("/session/abc/tasks");
    expect(segmentHref("abc", "replay")).toBe("/session/abc/replay");
  });
});
