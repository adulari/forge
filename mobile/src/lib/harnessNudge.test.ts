import { describe, expect, it } from "vitest";

import { isHarnessNudge } from "./harnessNudge";

describe("harness nudge detection", () => {
  it("recognizes the empty-diff continuation nudge sent as a synthetic user row", () => {
    expect(
      isHarnessNudge("user", "You have not modified any files. Implement the fix now — do not just describe it."),
    ).toBe(true);
  });

  it("does not flag a real user message, even a similar one", () => {
    expect(isHarnessNudge("user", "please implement the fix now")).toBe(false);
    expect(isHarnessNudge("user", "")).toBe(false);
  });

  it("only matches on user-role rows — an assistant/system row never counts", () => {
    const nudgeText = "You have not modified any files. Implement the fix now — do not just describe it.";
    expect(isHarnessNudge("assistant", nudgeText)).toBe(false);
    expect(isHarnessNudge("system", nudgeText)).toBe(false);
  });
});
