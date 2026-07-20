import { describe, expect, it } from "vitest";

import {
  clampComposerHeight,
  composerInputVerticalPadding,
  COMPOSER_LINE_HEIGHT,
  COMPOSER_MAX_HEIGHT,
  COMPOSER_MAX_LINES,
  COMPOSER_MIN_HEIGHT,
  nativeComposerHeightFromContent,
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

  it("does not compound native vertical inset across repeated layout measurements", () => {
    for (const measuredHeight of [44, 66, 88, 110, 132, 154]) {
      expect(nativeComposerHeightFromContent(measuredHeight)).toBe(measuredHeight);
    }
    expect(nativeComposerHeightFromContent(22)).toBe(COMPOSER_MIN_HEIGHT);
  });

  it("keeps web input padding out of scroll height while preserving the native inset", () => {
    const nativeInset = (COMPOSER_MIN_HEIGHT - COMPOSER_LINE_HEIGHT) / 2;

    expect(nativeInset).toBe(11);
    expect(composerInputVerticalPadding("web")).toBe(0);
    expect(composerInputVerticalPadding("ios")).toBe(nativeInset);
  });
});
