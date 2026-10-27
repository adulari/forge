import { describe, expect, it } from "vitest";

import { isSegmentPath, segmentFromPathname } from "./sessionSegments";

describe("segmentFromPathname", () => {
  it("maps the bare session route to chat", () => {
    expect(segmentFromPathname("/session/abc")).toBe("chat");
  });

  it.each(["tasks", "agents", "review", "files", "terminal", "replay"] as const)("maps /%s", (segment) => {
    expect(segmentFromPathname(`/session/abc/${segment}`)).toBe(segment);
  });

  it("treats routes pushed on top of a session as chat", () => {
    expect(segmentFromPathname("/session/abc/workflow")).toBe("chat");
  });
});

describe("isSegmentPath", () => {
  it("is true for the session route and every segment under it", () => {
    expect(isSegmentPath("/session/abc")).toBe(true);
    for (const segment of ["tasks", "agents", "review", "files", "terminal", "replay"]) {
      expect(isSegmentPath(`/session/abc/${segment}`)).toBe(true);
    }
  });

  it("is false for routes layered on top of the session", () => {
    expect(isSegmentPath("/session/abc/workflow")).toBe(false);
    expect(isSegmentPath("/session/abc/workflows")).toBe(false);
  });
});
