import { expect, it, vi } from "vitest";

import { parseAppearancePreferences } from "./appearancePreferences";

vi.mock("@react-native-async-storage/async-storage", () => ({
  default: {
    getItem: vi.fn(async () => null),
    setItem: vi.fn(async () => undefined),
  },
}));

it("accepts only an explicit persisted code-wrap preference", () => {
  expect(parseAppearancePreferences('{"wrapCodeBlocks":true}')).toEqual({ wrapCodeBlocks: true });
  expect(parseAppearancePreferences('{"wrapCodeBlocks":"true"}')).toEqual({ wrapCodeBlocks: false });
  expect(parseAppearancePreferences("{bad json")).toEqual({ wrapCodeBlocks: false });
  expect(parseAppearancePreferences(null)).toEqual({ wrapCodeBlocks: false });
});
