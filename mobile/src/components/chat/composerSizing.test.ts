import { describe, expect, it } from "vitest";

import {
  clampComposerHeight,
  composerInputVerticalPadding,
  composerScrollEnabled,
  composerUsesNativeMirror,
  COMPOSER_LINE_HEIGHT,
  COMPOSER_MAX_HEIGHT,
  COMPOSER_MAX_LINES,
  COMPOSER_MIN_HEIGHT,
  nativeComposerMirrorText,
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

  it("adds a zero-width suffix so empty and trailing-newline drafts occupy a mirror line", () => {
    expect(nativeComposerMirrorText("")).toBe("\u200b");
    expect(nativeComposerMirrorText("one\ntwo")).toBe("one\ntwo\u200b");
    expect(nativeComposerMirrorText("one\n")).toBe("one\n\u200b");
  });

  it("uses normal-flow mirror sizing only on native platforms", () => {
    expect(composerUsesNativeMirror("ios")).toBe(true);
    expect(composerUsesNativeMirror("android")).toBe(true);
    expect(composerUsesNativeMirror("web")).toBe(false);
  });

  it("keeps native scrolling enabled while web waits for the height cap", () => {
    expect(composerScrollEnabled("ios", COMPOSER_MIN_HEIGHT)).toBe(true);
    expect(composerScrollEnabled("ios", COMPOSER_MAX_HEIGHT)).toBe(true);
    expect(composerScrollEnabled("android", COMPOSER_MIN_HEIGHT)).toBe(true);
    expect(composerScrollEnabled("android", COMPOSER_MAX_HEIGHT)).toBe(true);
    expect(composerScrollEnabled("web", COMPOSER_MIN_HEIGHT)).toBe(false);
    expect(composerScrollEnabled("web", COMPOSER_MAX_HEIGHT)).toBe(true);
  });

  it("keeps web input padding out of scroll height while preserving the native inset", () => {
    const nativeInset = (COMPOSER_MIN_HEIGHT - COMPOSER_LINE_HEIGHT) / 2;

    expect(nativeInset).toBe(11);
    expect(composerInputVerticalPadding("web")).toBe(0);
    expect(composerInputVerticalPadding("ios")).toBe(nativeInset);
  });
});
