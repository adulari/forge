import { describe, expect, it } from "vitest";

import { shipHint, shipLabel, shipSteps } from "./shipModel";

describe("shipSteps", () => {
  it("does nothing on a clean tree", () => {
    expect(shipSteps({ staged: 0, unstaged: 0, hasMessage: false, canPush: true })).toEqual([]);
  });
  it("stages first when nothing is staged, and writes a message when the box is empty", () => {
    expect(shipSteps({ staged: 0, unstaged: 3, hasMessage: false, canPush: true })).toEqual(["stage", "message", "commit", "push"]);
  });
  it("respects what is already staged and typed", () => {
    expect(shipSteps({ staged: 2, unstaged: 1, hasMessage: true, canPush: true })).toEqual(["commit", "push"]);
  });
  it("skips push without a remote", () => {
    expect(shipSteps({ staged: 1, unstaged: 0, hasMessage: true, canPush: false })).toEqual(["commit"]);
  });
});

describe("copy", () => {
  it("reads as a sentence", () => {
    expect(shipHint(["stage", "message", "commit", "push"])).toBe("Will stage, write a message, commit and push.");
    expect(shipHint(["commit"])).toBe("Will commit.");
    expect(shipHint([])).toBe("Nothing to ship. The working tree is clean.");
  });
  it("labels by whether it pushes", () => {
    expect(shipLabel(["commit", "push"])).toBe("Ship it");
    expect(shipLabel(["commit"])).toBe("Commit");
  });
});
