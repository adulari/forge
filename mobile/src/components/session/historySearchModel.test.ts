import { describe, expect, it } from "vitest";

import { emptyMessage, roleLabel, splitSnippet } from "./historySearchModel";

describe("historySearchModel", () => {
  it("splits a snippet at its match markers", () => {
    expect(splitSnippet("we settled on [exponential] [backoff] for uploads")).toEqual([
      { text: "we settled on ", match: false },
      { text: "exponential", match: true },
      { text: " ", match: false },
      { text: "backoff", match: true },
      { text: " for uploads", match: false },
    ]);
  });

  it("leaves text without a closed marker alone", () => {
    expect(splitSnippet("array[0 and more")).toEqual([{ text: "array[0 and more", match: false }]);
    expect(splitSnippet("")).toEqual([]);
  });

  it("names roles for people", () => {
    expect(roleLabel("user")).toBe("You");
    expect(roleLabel("assistant")).toBe("Forge");
    expect(roleLabel("custom")).toBe("custom");
  });

  it("explains an empty result", () => {
    expect(emptyMessage("a")).toMatch(/at least two letters/);
    expect(emptyMessage(" retry ")).toBe('Nothing in this session matches "retry".');
  });
});
