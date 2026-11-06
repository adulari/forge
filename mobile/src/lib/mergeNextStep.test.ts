import { describe, expect, it } from "vitest";

import { mergeStagedMessage } from "./mergeNextStep";

describe("mergeStagedMessage", () => {
  it("names the repo the merge was staged in", () => {
    expect(mergeStagedMessage("/home/u/code/app", "forge/x")).toBe("Changes are staged in ~/code/app. Review & commit");
  });
  it("falls back to the branch when an older daemon omits the base", () => {
    expect(mergeStagedMessage(undefined, "forge/x")).toContain("forge/x");
  });
});
