import { describe, expect, it } from "vitest";

import {
  clampComposerHeight,
  COMPOSER_MAX_HEIGHT,
  COMPOSER_MAX_LINES,
  COMPOSER_MIN_HEIGHT,
} from "./composerSizing";

describe("mobile composer sizing", () => {
  it("starts at the 44 point touch target", () => {
    expect(clampComposerHeight(22)).toBe(COMPOSER_MIN_HEIGHT);
  });

  it("grows with multiline content", () => {
    expect(clampComposerHeight(66)).toBe(66);
    expect(clampComposerHeight(110)).toBe(110);
  });

  it("caps at six lines and then scrolls", () => {
    expect(COMPOSER_MAX_LINES).toBe(6);
    expect(COMPOSER_MAX_HEIGHT).toBe(154);
    expect(clampComposerHeight(220)).toBe(COMPOSER_MAX_HEIGHT);
  });
});
