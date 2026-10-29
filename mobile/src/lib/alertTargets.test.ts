import { describe, expect, it } from "vitest";

import { consumeComposerFocus, getVisibleSessionIds, requestComposerFocus, setVisibleSessionIds } from "./alertTargets";

describe("alert targets", () => {
  it("hands the composer focus request to the matching session exactly once", () => {
    requestComposerFocus("s1");
    expect(consumeComposerFocus("other")).toBe(false);
    expect(consumeComposerFocus("s1")).toBe(true);
    expect(consumeComposerFocus("s1")).toBe(false);
  });

  it("remembers the visible sessions", () => {
    setVisibleSessionIds(["a", "b"]);
    expect(getVisibleSessionIds()).toEqual(["a", "b"]);
  });
});
