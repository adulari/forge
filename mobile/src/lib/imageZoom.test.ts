import { describe, expect, it } from "vitest";

import { fitScale, nextZoom } from "./imageZoom";

describe("fitScale", () => {
  it("shrinks a large image to the stage and keeps a small one at its own size", () => {
    expect(fitScale({ width: 2000, height: 1000 }, { width: 1024, height: 800 })).toBeCloseTo(1000 / 2000);
    expect(fitScale({ width: 100, height: 50 }, { width: 1024, height: 800 })).toBe(1);
  });

  it("does not divide by zero before layout", () => {
    expect(fitScale({ width: 100, height: 100 }, { width: 0, height: 0 })).toBe(1);
  });
});

describe("nextZoom", () => {
  it("steps through the stops and stops at the ends", () => {
    expect(nextZoom(1, 1)).toBe(1.5);
    expect(nextZoom(1, -1)).toBe(0.75);
    expect(nextZoom(8, 1)).toBe(8);
    expect(nextZoom(0.25, -1)).toBe(0.25);
    expect(nextZoom(1.2, 1)).toBe(1.5);
  });
});
