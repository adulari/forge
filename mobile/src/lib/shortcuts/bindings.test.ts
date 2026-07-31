import { describe, expect, it, vi } from "vitest";

import {
  findShortcutConflicts,
  formatShortcut,
  getDefaultShortcut,
  getResolvedShortcut,
  parseShortcutOverrides,
  setAppShortcut,
  shortcutBindingFromEvent,
  shortcutBindingsEqual,
} from "./bindings";

const storage = vi.hoisted(() => ({
  getItem: vi.fn(async () => null as string | null),
  setItem: vi.fn(async () => undefined),
}));

vi.mock("@react-native-async-storage/async-storage", () => ({
  default: storage,
}));
vi.mock("../platform", () => ({ isTauri: false }));

describe("app shortcut bindings", () => {
  it("normalizes shifted punctuation to a stable physical chord", () => {
    expect(shortcutBindingFromEvent({
      key: "?",
      metaKey: true,
      ctrlKey: false,
      altKey: false,
      shiftKey: true,
    })).toEqual({ key: "/", mod: true, alt: false, shift: true });
  });

  it("ignores modifier-only and unsupported key presses", () => {
    expect(shortcutBindingFromEvent({
      key: "Shift",
      metaKey: false,
      ctrlKey: false,
      altKey: false,
      shiftKey: true,
    })).toBeNull();
    expect(shortcutBindingFromEvent({
      key: "MediaPlayPause",
      metaKey: false,
      ctrlKey: false,
      altKey: false,
      shiftKey: false,
    })).toBeNull();
  });

  it("drops unknown or malformed persisted overrides", () => {
    expect(parseShortcutOverrides(JSON.stringify({
      "nav.fleet": { key: "9", mod: false, alt: true, shift: false },
      "nav.unknown": { key: "x", mod: true, alt: false, shift: false },
      "nav.inbox": { key: "2", mod: "yes", alt: true, shift: false },
    }))).toEqual({
      "nav.fleet": { key: "9", mod: false, alt: true, shift: false },
    });
    expect(parseShortcutOverrides("{bad json")).toEqual({});
  });

  it("detects app-action and reserved standard conflicts", () => {
    const overrides = {
      "nav.inbox": getDefaultShortcut("nav.fleet"),
    };
    expect(findShortcutConflicts("nav.fleet", getDefaultShortcut("nav.fleet"), overrides))
      .toEqual([{ id: "nav.inbox", label: "Open Inbox", kind: "app" }]);
    expect(findShortcutConflicts("command.palette", {
      key: "c",
      mod: true,
      alt: false,
      shift: false,
    })).toContainEqual({ id: "standard.copy", label: "standard Copy", kind: "reserved" });
  });

  it("formats platform labels without changing binding identity", () => {
    const value = { key: ".", mod: true, alt: true, shift: true };
    expect(formatShortcut(value, true)).toBe("⌘⌥⇧.");
    expect(formatShortcut(value, false)).toBe("Ctrl+Alt+Shift+.");
    expect(shortcutBindingsEqual(value, { ...value })).toBe(true);
  });

  it("rolls a visible edit back when device persistence fails", async () => {
    storage.setItem.mockRejectedValueOnce(new Error("storage unavailable"));
    await expect(setAppShortcut("nav.fleet", {
      key: "9",
      mod: false,
      alt: true,
      shift: false,
    })).rejects.toThrow("storage unavailable");
    expect(getResolvedShortcut("nav.fleet")).toEqual(getDefaultShortcut("nav.fleet"));
  });
});
