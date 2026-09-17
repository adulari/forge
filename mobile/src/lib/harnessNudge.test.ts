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

  it("trusts a v10+ daemon's kind='nudge' outright, regardless of wording", () => {
    expect(isHarnessNudge("user", "a brand-new nudge the client has never seen before", "nudge")).toBe(true);
  });

  it("falls back to the text heuristic when kind is present but not 'nudge'", () => {
    // A v9 daemon (kind-aware but not nudge-aware) sends kind='user' for everything, including
    // an old-wording nudge — the text match must still catch it.
    expect(
      isHarnessNudge(
        "user",
        "You have not modified any files. Implement the fix now — do not just describe it.",
        "user",
      ),
    ).toBe(true);
    // A real user message with kind='user' is still never flagged.
    expect(isHarnessNudge("user", "please implement the fix now", "user")).toBe(false);
  });
});
