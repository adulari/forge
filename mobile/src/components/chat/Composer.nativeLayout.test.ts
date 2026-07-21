/// <reference types="vite/client" />

import { describe, expect, it } from "vitest";

import composerSource from "./Composer.tsx?raw";

const normalizedSource = composerSource.replace(/\s+/g, " ");

function expectSource(pattern: RegExp, message: string): void {
  expect(normalizedSource, message).toMatch(pattern);
}

function sourceRegion(start: string, end: string, message: string): string {
  const startIndex = normalizedSource.indexOf(start);
  expect(startIndex, `${message}: missing ${start}`).toBeGreaterThanOrEqual(0);

  const endIndex = normalizedSource.indexOf(end, startIndex);
  expect(endIndex, `${message}: missing ${end}`).toBeGreaterThan(startIndex);
  return normalizedSource.slice(startIndex, endIndex);
}

describe("native composer layout contract", () => {
  it("platform-gates a named mirror fed by the local native draft", () => {
    expectSource(
      /const usesNativeMirror = composerUsesNativeMirror\(Platform\.OS\)/,
      "native mirror selection must remain platform-derived",
    );
    expectSource(
      /usesNativeMirror \? \( <NativeComposerMirror [^>]*text=\{nativeText\}[^>]*\/> \) : null/,
      "the normal-flow mirror must render only through the native platform gate",
    );

    const mirror = sourceRegion(
      "function NativeComposerMirror",
      "export interface ComposerProps",
      "named native mirror contract",
    );
    expect(mirror, "the mirror must size from the local native text and preserve trailing lines").toMatch(
      /nativeComposerMirrorText\(text\)/,
    );
  });

  it("keeps the mirror invisible, non-interactive, and out of accessibility trees", () => {
    const mirror = sourceRegion(
      "function NativeComposerMirror",
      "export interface ComposerProps",
      "named native mirror contract",
    );

    expect(mirror, "the mirror must not intercept pointer input").toMatch(/pointerEvents="none"/);
    expect(mirror, "the mirror must not become an accessibility element").toMatch(/accessible=\{false\}/);
    expect(mirror, "the mirror must be hidden from the iOS accessibility tree").toMatch(
      /accessibilityElementsHidden/,
    );
    expect(mirror, "the mirror subtree must be hidden from Android accessibility").toMatch(
      /importantForAccessibility="no-hide-descendants"/,
    );
    expectSource(/nativeMirror: \{ width: "100%", opacity: 0,? \}/, "the invisible mirror must retain full-width layout");
  });

  it("shares text metrics and lets the capped native wrapper own height", () => {
    const mirror = sourceRegion(
      "function NativeComposerMirror",
      "export interface ComposerProps",
      "named native mirror contract",
    );
    expect(mirror, "mirror typography and padding must match the input").toMatch(
      /style=\{\[type\.body, styles\.inputTextMetrics, styles\.nativeMirror, \{ color \}\]\}/,
    );
    expectSource(
      /<TextInput [\s\S]*?style=\{\[ type\.body, styles\.input, styles\.inputTextMetrics,/,
      "the visible input must share type.body and inputTextMetrics with the mirror",
    );
    expectSource(
      /nativeInputWrap: \{ minHeight: MIN_HEIGHT, maxHeight: MAX_HEIGHT, overflow: "hidden",? \}/,
      "the native wrapper must own min/max sizing and clip overflow",
    );

    const wrapperStyles = composerSource.match(/style=\{\[\s*styles\.inputWrap,([\s\S]*?)\]\}/)?.[1];
    expect(wrapperStyles, "input wrapper style contract must remain recognizable").toBeDefined();
    expect(wrapperStyles, "native wrapper selection must not reintroduce a JS-driven height").not.toMatch(
      /\bheight\b/,
    );
    expect(wrapperStyles, "native wrapper sizing must remain mirror-gated").toMatch(
      /usesNativeMirror\s*\?\s*styles\.nativeInputWrap\s*:\s*null/,
    );
  });

  it("keeps the native input overlaid and removes native measurement feedback", () => {
    expectSource(
      /Platform\.OS === "web" \? \{ height \} : StyleSheet\.absoluteFill/,
      "web must retain explicit height while native TextInput fills the mirror-owned wrapper",
    );
    expectSource(
      /scrollEnabled=\{composerScrollEnabled\(Platform\.OS, height\)\}/,
      "scroll behavior must stay wired through the platform strategy",
    );
    expect(normalizedSource, "native content-size feedback must remain absent").not.toMatch(
      /\bonContentSizeChange\s*=/,
    );
  });
});
