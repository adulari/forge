import { expect, it, vi } from "vitest";

import { parseTerminalPreferences, terminalLineHeight, TERMINAL_DEFAULTS } from "./terminalPreferences";

vi.mock("@react-native-async-storage/async-storage", () => ({
  default: {
    getItem: vi.fn(async () => null),
    setItem: vi.fn(async () => undefined),
  },
}));

it("falls back to the defaults for anything missing, malformed or off the offered scale", () => {
  expect(parseTerminalPreferences(null)).toEqual(TERMINAL_DEFAULTS);
  expect(parseTerminalPreferences("{bad json")).toEqual(TERMINAL_DEFAULTS);
  expect(parseTerminalPreferences('{"fontSize":99,"scrollSpeed":"fast"}')).toEqual(TERMINAL_DEFAULTS);
});

it("keeps stored choices that sit on the scale", () => {
  expect(parseTerminalPreferences('{"fontSize":14,"scrollSpeed":2,"confirmClose":false}')).toEqual({
    fontSize: 14,
    scrollSpeed: 2,
    confirmClose: false,
  });
});

it("grows the line height with the font", () => {
  expect(terminalLineHeight(11)).toBe(16);
  expect(terminalLineHeight(16)).toBeGreaterThan(terminalLineHeight(12));
});
