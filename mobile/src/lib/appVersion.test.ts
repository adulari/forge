import { describe, expect, it } from "vitest";

import { formatVersionMeta, resolveAppVersion } from "./appVersionCore";

describe("resolveAppVersion", () => {
  it("uses the authoritative Tauri bundle version on desktop", () => {
    expect(resolveAppVersion(true, "2.8.1", "2.7.0")).toBe("2.8.1");
  });

  it("uses the Expo version outside Tauri", () => {
    expect(resolveAppVersion(false, "9.9.9", "2.8.0")).toBe("2.8.0");
  });

  it("falls back without mislabelling the platform", () => {
    expect(resolveAppVersion(true, null, "2.8.0")).toBe("2.8.0");
    expect(resolveAppVersion(false, null, null)).toBe("—");
  });
});

describe("formatVersionMeta", () => {
  it("labels the desktop bundle and the shared client separately", () => {
    expect(formatVersionMeta(true, "2.7.0", "1.0.1", 9)).toBe(
      "Desktop v2.7.0 · client v1.0.1 · protocol v9",
    );
  });

  it("keeps the single shared client version on every non-desktop surface", () => {
    expect(formatVersionMeta(false, "2.7.0", "1.0.1", 9)).toBe("v1.0.1 · protocol v9");
    expect(formatVersionMeta(false, null, "1.0.1", 7)).toBe("v1.0.1 · protocol v7");
  });

  it("does not claim a desktop release before the bundle lookup resolves", () => {
    expect(formatVersionMeta(true, null, "1.0.1", 9)).toBe("v1.0.1 · protocol v9");
    expect(formatVersionMeta(true, "  ", "1.0.1", 9)).toBe("v1.0.1 · protocol v9");
    expect(formatVersionMeta(true, "2.7.0", null, 9)).toBe("v2.7.0 · protocol v9");
    expect(formatVersionMeta(false, null, null, 9)).toBe("v— · protocol v9");
  });
});
