import { describe, expect, it, vi } from "vitest";

import {
  IDE_AGENT_WIDTH,
  IDE_RAIL_WIDTH,
  parseIdeModePreferences,
  readIdeModePreferences,
  resetIdeModePreferencesForTests,
  setIdeModeEnabled,
  setIdeRailWidth,
} from "./ideModePrefs";

const storage = vi.hoisted(() => ({
  getItem: vi.fn(async () => null as string | null),
  setItem: vi.fn(async () => undefined),
}));
vi.mock("@react-native-async-storage/async-storage", () => ({ default: storage }));

describe("ideModePrefs", () => {
  it("defaults to off with the plan's pane widths", () => {
    expect(parseIdeModePreferences(null)).toEqual({
      enabled: false,
      railWidth: 260,
      agentWidth: 340,
      bottomHeight: 240,
      coachSeen: false,
    });
  });

  it("reads a stored blob and clamps sizes that no longer fit", () => {
    const parsed = parseIdeModePreferences(
      JSON.stringify({ enabled: true, railWidth: 9000, agentWidth: 10, coachSeen: true }),
    );
    expect(parsed.enabled).toBe(true);
    expect(parsed.railWidth).toBe(IDE_RAIL_WIDTH.max);
    expect(parsed.agentWidth).toBe(IDE_AGENT_WIDTH.min);
    expect(parsed.coachSeen).toBe(true);
  });

  it("treats unparsable storage as the defaults", () => {
    expect(parseIdeModePreferences("{nope").enabled).toBe(false);
    expect(parseIdeModePreferences('"x"').railWidth).toBe(260);
  });

  it("persists a toggle under forge.ideMode.v1 and keeps it when storage throws", async () => {
    resetIdeModePreferencesForTests();
    setIdeModeEnabled(true);
    expect(readIdeModePreferences().enabled).toBe(true);
    expect(storage.setItem).toHaveBeenCalledWith("forge.ideMode.v1", expect.stringContaining('"enabled":true'));
    storage.setItem.mockRejectedValueOnce(new Error("quota"));
    setIdeRailWidth(300);
    await Promise.resolve();
    expect(readIdeModePreferences().railWidth).toBe(300);
  });
});
