import { describe, expect, it } from "vitest";

import { resolvePlanDecision } from "./planDecision";

const options = [
  { label: "Build it", description: "implement now" },
  { label: "Cancel", description: "discard" },
];

describe("resolvePlanDecision", () => {
  it("binds explicit Build and Cancel options to the matching live plan question", () => {
    expect(
      resolvePlanDecision(
        "Ship the release",
        'Build this plan? — "Ship the release" (3 steps). Choose Build it / Cancel.',
        options,
        42,
      ),
    ).toEqual({ build: "1", cancel: "2", promptSeq: 42 });
  });

  it.each([
    ["missing question", null, options, 42],
    ["zero prompt sequence", 'Build this plan? — "Ship the release"', options, 0],
    ["unrelated question", "Choose a deployment region", options, 42],
    ["different plan", 'Build this plan? — "Delete production"', options, 42],
    ["missing Build option", 'Build this plan? — "Ship the release"', options.slice(1), 42],
    ["missing Cancel option", 'Build this plan? — "Ship the release"', options.slice(0, 1), 42],
  ])("does not bind an unsafe decision: %s", (_label, question, decisionOptions, promptSeq) => {
    expect(
      resolvePlanDecision(
        "Ship the release",
        question as string | null,
        decisionOptions as typeof options,
        promptSeq as number,
      ),
    ).toBeNull();
  });

  it("uses the actual option positions and never assumes 1/2", () => {
    expect(
      resolvePlanDecision(
        "Ship the release",
        'Build this plan? — "Ship the release"',
        [
          { label: "Revise", description: "change it" },
          { label: "Cancel", description: "discard" },
          { label: "Build it", description: "implement" },
        ],
        7,
      ),
    ).toEqual({ build: "3", cancel: "2", promptSeq: 7 });
  });
});
