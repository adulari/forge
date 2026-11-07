import { describe, expect, it } from "vitest";

import { turnEndKey, turnEndTitle } from "./turnEnd";

describe("turnEnd", () => {
  it("titles the stop with the host's plain label", () => {
    expect(turnEndTitle({ reason: "stalled", label: "no progress", at: 10 })).toBe("Turn stopped: no progress");
  });

  it("falls back to the reason when the label is empty", () => {
    expect(turnEndTitle({ reason: "tasks_unfinished", label: " ", at: 10 })).toBe("Turn stopped: tasks unfinished");
  });

  it("keys a stop by reason and time", () => {
    expect(turnEndKey({ reason: "stalled", label: "x", at: 10 })).not.toBe(turnEndKey({ reason: "stalled", label: "x", at: 11 }));
  });
});
