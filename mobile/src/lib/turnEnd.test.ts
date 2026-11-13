import { describe, expect, it } from "vitest";

import { continuePrompt, countsLine, endedAgo, isResumable, turnEndAdvice, turnEndKey, turnEndTitle, visibleTurnEnd } from "./turnEnd";

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

  it("offers continue only for the stops continue fixes", () => {
    for (const reason of ["stalled", "tasks_unfinished", "provider_down", "provider_error", "interrupted", "daemon_restart_or_crash", "final_answer_mid_intent"]) {
      expect(continuePrompt({ reason, label: "x", at: 1 })).toBe("continue");
    }
    for (const reason of ["budget_exhausted", "pinned_rejected", "review_blocked", "no_healthy_model"]) {
      expect(continuePrompt({ reason, label: "x", at: 1 })).toBeNull();
    }
  });

  it("trusts the host's resumable flag over the built-in list", () => {
    expect(isResumable({ reason: "some_new_reason", resumable: true })).toBe(true);
    expect(isResumable({ reason: "stalled", resumable: false })).toBe(false);
  });

  it("hides the banner once a new turn is running", () => {
    const end = { reason: "stalled", label: "no progress", at: 10 };
    expect(visibleTurnEnd(end, false)).toBe(end);
    expect(visibleTurnEnd(end, true)).toBeNull();
    expect(visibleTurnEnd(null, false)).toBeNull();
  });

  it("says what to do in a sentence", () => {
    expect(turnEndAdvice({ reason: "stalled", label: "x", at: 1, advice: "edits are kept; send continue" })).toBe("Edits are kept; send continue.");
    expect(turnEndAdvice({ reason: "stalled", label: "x", at: 1 })).toContain("continue");
    expect(turnEndAdvice({ reason: "budget_exhausted", label: "x", at: 1 })).not.toContain("continue");
  });

  it("words the age of an ending", () => {
    expect(endedAgo(100, 130)).toBe("just now");
    expect(endedAgo(100, 100 + 600)).toBe("10 min ago");
    expect(endedAgo(100, 100 + 7200)).toBe("2 h ago");
  });

  it("sums the counts in plain words", () => {
    expect(countsLine([])).toBe("No turns recorded.");
    expect(
      countsLine([
        { reason: "final_answer", label: "finished", count: 9 },
        { reason: "stalled", label: "no progress", count: 2 },
      ]),
    ).toBe("11 turns: 9 finished, 2 no progress");
  });
});
