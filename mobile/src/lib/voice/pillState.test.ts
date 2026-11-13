import { describe, expect, it } from "vitest";

import { initialPillState, MAX_RECORDING_SECONDS, pillReducer, recordingCapReached } from "./pillState";

describe("pillReducer", () => {
  it("keeps the recording retryable after a transcription failure", () => {
    const transcribing = pillReducer(initialPillState, { type: "accept" });
    const failed = pillReducer(transcribing, { type: "failed", message: "model downloading", retryable: true });
    expect(failed).toEqual({ phase: "error", error: "model downloading", retryable: true });
  });

  it("retry re-enters transcribing only when a recording is held", () => {
    const failed = pillReducer(initialPillState, { type: "failed", message: "413", retryable: true });
    expect(pillReducer(failed, { type: "retry" }).phase).toBe("transcribing");
    const lost = pillReducer(initialPillState, { type: "failed", message: "no speech detected", retryable: false });
    expect(pillReducer(lost, { type: "retry" })).toBe(lost);
  });

  it("accept is ignored outside recording", () => {
    const failed = pillReducer(initialPillState, { type: "failed", message: "x", retryable: true });
    expect(pillReducer(failed, { type: "accept" })).toBe(failed);
  });
});

describe("recordingCapReached", () => {
  it("stops at the cap", () => {
    expect(recordingCapReached(MAX_RECORDING_SECONDS - 1)).toBe(false);
    expect(recordingCapReached(MAX_RECORDING_SECONDS)).toBe(true);
  });
});
