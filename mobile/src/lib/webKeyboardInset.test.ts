import { describe, expect, it } from "vitest";

import { keyboardInset } from "./webKeyboardInset";

const viewport = (patch: Partial<Parameters<typeof keyboardInset>[0]>) => ({ innerHeight: 800, height: 800, offsetTop: 0, scale: 1, ...patch });

describe("keyboardInset (mobile Safari)", () => {
  it("is zero while the keyboard is closed", () => {
    expect(keyboardInset(viewport({}))).toBe(0);
  });

  it("is the height the visual viewport lost to the keyboard", () => {
    expect(keyboardInset(viewport({ height: 480 }))).toBe(320);
  });

  it("does not count the part Safari scrolled the visual viewport down by", () => {
    expect(keyboardInset(viewport({ height: 480, offsetTop: 100 }))).toBe(220);
  });

  it("ignores address-bar movement", () => {
    expect(keyboardInset(viewport({ height: 760 }))).toBe(0);
  });

  it("ignores a pinch zoom, which also shrinks the visual viewport", () => {
    expect(keyboardInset(viewport({ height: 400, scale: 2 }))).toBe(0);
  });
});
