import { describe, expect, it } from "vitest";

import {
  USAGE_REFRESH_DEFAULT_MINUTES,
  clampUsageRefreshMinutes,
  parseDesktopShellPrefs,
  usageRefetchInterval,
} from "./desktopShellPrefs";

describe("desktop shell prefs", () => {
  it("defaults to notifying in the background and a five minute usage refresh", () => {
    expect(parseDesktopShellPrefs(null)).toEqual({ notifyWhenBackground: true, usageRefreshMinutes: 5 });
    expect(parseDesktopShellPrefs("not json")).toEqual({ notifyWhenBackground: true, usageRefreshMinutes: 5 });
  });

  it("only an explicit false turns notifications off", () => {
    expect(parseDesktopShellPrefs('{"notifyWhenBackground":false}').notifyWhenBackground).toBe(false);
    expect(parseDesktopShellPrefs('{"notifyWhenBackground":"no"}').notifyWhenBackground).toBe(true);
  });

  it("clamps the refresh interval to 2..60 minutes and falls back on junk", () => {
    expect(clampUsageRefreshMinutes(1)).toBe(2);
    expect(clampUsageRefreshMinutes(0)).toBe(2);
    expect(clampUsageRefreshMinutes(7.6)).toBe(8);
    expect(clampUsageRefreshMinutes(500)).toBe(60);
    expect(clampUsageRefreshMinutes("3")).toBe(USAGE_REFRESH_DEFAULT_MINUTES);
    expect(clampUsageRefreshMinutes(Number.NaN)).toBe(USAGE_REFRESH_DEFAULT_MINUTES);
    expect(parseDesktopShellPrefs('{"usageRefreshMinutes":1}').usageRefreshMinutes).toBe(2);
  });

  it("keeps per-session usage live and applies the setting to the account-wide view", () => {
    expect(usageRefetchInterval("s1", 10)).toBe(30_000);
    expect(usageRefetchInterval(null, 10)).toBe(600_000);
    expect(usageRefetchInterval(undefined, 0)).toBe(120_000);
  });
});
