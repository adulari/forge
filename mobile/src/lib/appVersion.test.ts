import { describe, expect, it } from "vitest";

import { resolveAppVersion } from "./appVersionCore";

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
