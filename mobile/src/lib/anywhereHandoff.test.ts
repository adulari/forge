import { describe, expect, it } from "vitest";
import { handoffOutcome, handoffRecovery, type CapsuleStatus } from "./anywhereHandoff";

const status = (state: CapsuleStatus["state"]): CapsuleStatus => ({ version: 1, capsule_id: "ab".repeat(16), state, acknowledgement_envelope: null, acknowledgement_signing_public_key: null });

describe("handoff terminal and uncertain states", () => {
  it.each([["reserved", "pending"], ["ready", "pending"], ["claimed", "pending"], ["acknowledged", "accepted"], ["failed", "failed"], ["cancelled", "cancelled"]] as const)("maps %s to %s", (input, expected) => expect(handoffOutcome(status(input))).toBe(expected));
  it("never treats a network failure as safe to retry", () => {
    expect(handoffOutcome(null, false, true)).toBe("indeterminate");
    expect(handoffRecovery("indeterminate")).toContain("Do not resume on both hosts");
  });
  it("makes failed imports explicitly preserve the source lease", () => expect(handoffRecovery("failed")).toContain("source lease is unchanged"));
});
