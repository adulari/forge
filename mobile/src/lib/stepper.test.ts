import { describe, expect, it } from "vitest";

import { stepperStatuses, type StepperStep } from "./stepper";

const steps = (...statuses: (StepperStep["status"] | undefined)[]): StepperStep[] =>
  statuses.map((status, i) => ({ key: `s${i}`, label: `Step ${i}`, status }));

describe("stepperStatuses", () => {
  it("follows the active index when no status is given", () => {
    expect(stepperStatuses(steps(undefined, undefined, undefined, undefined), 2)).toEqual([
      "done",
      "done",
      "current",
      "todo",
    ]);
  });

  it("keeps an explicit status, whatever the active index says", () => {
    expect(stepperStatuses(steps("skipped", undefined, "done", "todo"), 1)).toEqual([
      "skipped",
      "current",
      "done",
      "todo",
    ]);
  });

  it("handles an empty list", () => {
    expect(stepperStatuses([], 0)).toEqual([]);
  });
});
