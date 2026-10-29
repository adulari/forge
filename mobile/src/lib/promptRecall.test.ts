import { describe, expect, it } from "vitest";

import { collapsePrompts, RECALL_IDLE, recallStep } from "./promptRecall";

describe("collapsePrompts", () => {
  it("trims, drops blanks and collapses back-to-back repeats only", () => {
    expect(collapsePrompts(["  a ", "a", "", "b", "a"])).toEqual(["a", "b", "a"]);
  });
});

describe("recallStep", () => {
  const entries = ["first", "second"];

  it("Up in an empty composer returns the newest prompt and keeps the draft", () => {
    const step = recallStep("up", entries, RECALL_IDLE, "", 0);
    expect(step).toEqual({ handled: true, text: "second", state: { pos: 1, draft: "" } });
  });

  it("walks back to the oldest entry and stops there", () => {
    let step = recallStep("up", entries, RECALL_IDLE, "", 0);
    step = recallStep("up", entries, step.state, step.text, 0);
    expect(step.text).toBe("first");
    step = recallStep("up", entries, step.state, step.text, 0);
    expect(step.text).toBe("first");
  });

  it("Down steps forward and then restores the draft", () => {
    let step = recallStep("up", entries, RECALL_IDLE, "half typed", 0);
    expect(step.state.draft).toBe("half typed");
    step = recallStep("up", entries, step.state, step.text, 0);
    step = recallStep("down", entries, step.state, step.text, step.text.length);
    expect(step.text).toBe("second");
    step = recallStep("down", entries, step.state, step.text, step.text.length);
    expect(step).toEqual({ handled: true, text: "half typed", state: RECALL_IDLE });
  });

  it("leaves Up alone when the caret is mid-draft or on a lower row", () => {
    expect(recallStep("up", entries, RECALL_IDLE, "typing here", 6).handled).toBe(false);
    const browsing = { pos: 1, draft: "" };
    expect(recallStep("up", entries, browsing, "a\nb", 3).handled).toBe(false);
    expect(recallStep("up", entries, browsing, "a\nb", 1).handled).toBe(true);
  });

  it("leaves Down alone when not browsing or above the last row", () => {
    expect(recallStep("down", entries, RECALL_IDLE, "x", 1).handled).toBe(false);
    expect(recallStep("down", entries, { pos: 0, draft: "" }, "a\nb", 0).handled).toBe(false);
  });

  it("does nothing with no history or an active selection", () => {
    expect(recallStep("up", [], RECALL_IDLE, "", 0).handled).toBe(false);
    expect(recallStep("up", entries, RECALL_IDLE, "abc", 0, true).handled).toBe(false);
  });
});
