import { describe, expect, it } from "vitest";

import { promptPreview, rowAtReach, stickyPromptFor } from "./stickyPrompt";

const prompts = [
  { index: 3, seq: 40, text: "newer prompt" },
  { index: 9, seq: 10, text: "older prompt" },
];

describe("stickyPromptFor", () => {
  it("names the prompt above the top visible row", () => {
    expect(stickyPromptFor(prompts, 1)?.seq).toBe(40);
    expect(stickyPromptFor(prompts, 5)?.seq).toBe(10);
  });

  it("shows nothing when the top row is the prompt itself", () => {
    expect(stickyPromptFor(prompts, 3)).toBeNull();
    expect(stickyPromptFor(prompts, 9)).toBeNull();
  });

  it("shows nothing when no prompt lies above the view", () => {
    expect(stickyPromptFor(prompts, 12)).toBeNull();
    expect(stickyPromptFor([], 0)).toBeNull();
  });
});

describe("promptPreview", () => {
  it("flattens whitespace and cuts long text", () => {
    expect(promptPreview("fix\n  the   bug")).toBe("fix the bug");
    const long = promptPreview("word ".repeat(80), 40);
    expect(long.length).toBe(40);
    expect(long.endsWith("…")).toBe(true);
  });
});

describe("rowAtReach", () => {
  it("finds the row holding a distance from the newest end", () => {
    const heights = [100, 50, 200, 80];
    expect(rowAtReach(heights, 0)).toBe(0);
    expect(rowAtReach(heights, 99)).toBe(0);
    expect(rowAtReach(heights, 100)).toBe(1);
    expect(rowAtReach(heights, 349)).toBe(2);
    expect(rowAtReach(heights, 5000)).toBe(3);
  });

  it("estimates unmeasured rows from the measured ones, and handles none", () => {
    expect(rowAtReach([100, undefined, undefined], 250)).toBe(2);
    expect(rowAtReach([undefined, undefined], 100, 60)).toBe(1);
    expect(rowAtReach([], 10)).toBeNull();
  });
});
