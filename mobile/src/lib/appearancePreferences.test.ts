import { expect, it, vi } from "vitest";

import { parseAppearancePreferences } from "./appearancePreferences";

vi.mock("@react-native-async-storage/async-storage", () => ({
  default: {
    getItem: vi.fn(async () => null),
    setItem: vi.fn(async () => undefined),
  },
}));

it("accepts only an explicit persisted code-wrap preference", () => {
  expect(parseAppearancePreferences('{"wrapCodeBlocks":true}').wrapCodeBlocks).toBe(true);
  expect(parseAppearancePreferences('{"wrapCodeBlocks":"true"}').wrapCodeBlocks).toBe(false);
  expect(parseAppearancePreferences("{bad json").wrapCodeBlocks).toBe(false);
  expect(parseAppearancePreferences(null).wrapCodeBlocks).toBe(false);
});

it("folds tool groups unless explicitly turned off", () => {
  expect(parseAppearancePreferences(null).foldToolGroups).toBe(true);
  expect(parseAppearancePreferences('{"wrapCodeBlocks":true}')).toEqual({ wrapCodeBlocks: true, foldToolGroups: true });
  expect(parseAppearancePreferences('{"foldToolGroups":false}').foldToolGroups).toBe(false);
  expect(parseAppearancePreferences('{"foldToolGroups":"false"}').foldToolGroups).toBe(true);
  expect(parseAppearancePreferences("{bad json").foldToolGroups).toBe(true);
});
