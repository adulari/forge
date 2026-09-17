import { describe, expect, it } from "vitest";

import { shouldMountTranscript } from "./transcriptMountGate";

describe("shouldMountTranscript", () => {
  it("never gates non-Android platforms, regardless of focus/transition state", () => {
    expect(shouldMountTranscript({ isAndroid: false, isFocused: false, transitionSettled: false })).toBe(true);
    expect(shouldMountTranscript({ isAndroid: false, isFocused: true, transitionSettled: false })).toBe(true);
    expect(shouldMountTranscript({ isAndroid: false, isFocused: false, transitionSettled: true })).toBe(true);
  });

  it("withholds the transcript on Android until both focused and settled", () => {
    expect(shouldMountTranscript({ isAndroid: true, isFocused: false, transitionSettled: false })).toBe(false);
    expect(shouldMountTranscript({ isAndroid: true, isFocused: true, transitionSettled: false })).toBe(false);
    expect(shouldMountTranscript({ isAndroid: true, isFocused: false, transitionSettled: true })).toBe(false);
  });

  it("mounts on Android once focused and the transition has settled", () => {
    expect(shouldMountTranscript({ isAndroid: true, isFocused: true, transitionSettled: true })).toBe(true);
  });
});
