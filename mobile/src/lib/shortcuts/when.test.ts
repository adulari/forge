import { describe, expect, it } from "vitest";

import { currentFocusTags, publishFocusTag } from "./focusContext";
import { describeWhen, evaluateWhen, whenDisjoint } from "./when";

describe("when expressions", () => {
  it("evaluates required and negated tags", () => {
    const tags = new Set(["hasSession"]);
    expect(evaluateWhen(undefined, tags)).toBe(true);
    expect(evaluateWhen("hasSession && !composerFocus", tags)).toBe(true);
    expect(evaluateWhen("hasSession && !composerFocus", new Set(["hasSession", "composerFocus"]))).toBe(false);
    expect(evaluateWhen("composerFocus", tags)).toBe(false);
  });

  it("treats the focus zones as mutually exclusive", () => {
    expect(whenDisjoint("composerFocus", "sidebarFocus")).toBe(true);
    expect(whenDisjoint("composerFocus", "!composerFocus")).toBe(true);
    expect(whenDisjoint("composerFocus", "hasSession")).toBe(false);
    expect(whenDisjoint(undefined, "composerFocus")).toBe(false);
  });

  it("describes a scope for the settings screen", () => {
    expect(describeWhen("composerFocus")).toBe("Only when the composer is focused");
    expect(describeWhen("hasSession && !composerFocus")).toBe("Only when a session is open and the composer is not focused");
    expect(describeWhen(undefined)).toBeNull();
  });

  it("publishes tags alongside the focused zone", () => {
    publishFocusTag("hasSession", true);
    expect([...currentFocusTags(null)]).toEqual(["hasSession"]);
    publishFocusTag("hasSession", false);
    expect(currentFocusTags(null).size).toBe(0);
  });
});
