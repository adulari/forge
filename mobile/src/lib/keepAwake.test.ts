import { describe, expect, it } from "vitest";

import {
  KEEP_AWAKE_LABELS,
  KEEP_AWAKE_LOCK_NOTE,
  KEEP_AWAKE_MODES,
  isKeepAwakeMode,
  keepAwakeExplanation,
} from "./keepAwake";

describe("keep-awake copy", () => {
  it("has a short label and a sentence for every mode the daemon accepts", () => {
    for (const mode of KEEP_AWAKE_MODES) {
      expect(KEEP_AWAKE_LABELS[mode].length).toBeLessThanOrEqual(8);
      expect(keepAwakeExplanation(mode).endsWith(".")).toBe(true);
    }
  });

  it("recognises only the daemon's own values and explains nothing else", () => {
    expect(isKeepAwakeMode("while_working")).toBe(true);
    expect(isKeepAwakeMode("sometimes")).toBe(false);
    expect(keepAwakeExplanation("sometimes")).toBe("");
  });

  it("always reminds that the screen can still lock", () => {
    expect(KEEP_AWAKE_LOCK_NOTE).toMatch(/screen can still lock/);
  });
});
