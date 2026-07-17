import { describe, expect, it } from "vitest";

import { acceptReplaySequences } from "./anywhereReplayWindow";

describe("Anywhere sparse replay window", () => {
  it("accepts an unseen lower blob tuple after a concurrent inline response", () => {
    const inline = acceptReplaySequences(undefined, [11n]);
    expect(inline.accepted).toBe(true);
    const delayedBlobAndReference = acceptReplaySequences(inline.next, [10n, 12n]);
    expect(delayedBlobAndReference).toEqual({ accepted: true, next: ["10", "11", "12"] });
  });

  it("rejects duplicates and tuples outside its bounded window", () => {
    expect(acceptReplaySequences(["10", "11"], [10n]).accepted).toBe(false);
    expect(acceptReplaySequences(["300"], [1n]).accepted).toBe(false);
    const advanced = acceptReplaySequences(["1"], [300n]);
    expect(advanced).toEqual({ accepted: true, next: ["300"] });
  });
});
