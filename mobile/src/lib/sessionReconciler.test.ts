import { describe, expect, it } from "vitest";

import fixture from "../../../protocol/remote-v8.json";
import { PROTOCOL_VERSION, isValidSnapshotFrame } from "./remoteProtocol";
import { decideSnapshotRevision, reconcilePendingMessages } from "./sessionReconciler";

describe("remote protocol conformance", () => {
  it("uses the same protocol version and accepts every golden frame", () => {
    expect(PROTOCOL_VERSION).toBe(fixture.protocol);
    for (const frame of fixture.frames) expect(isValidSnapshotFrame(frame)).toBe(true);
  });

  it("rejects malformed frames before they reach session state", () => {
    expect(isValidSnapshotFrame({ protocol: 8, session_id: "x", revision: Number.NaN })).toBe(false);
    expect(isValidSnapshotFrame({ protocol: 8, session_id: "x", revision: 1, resync: false })).toBe(false);
  });
});

describe("snapshot revision reconciliation", () => {
  it("accepts contiguous and explicit resync frames", () => {
    expect(decideSnapshotRevision(4, { revision: 5, resync: false })).toBe("accept");
    expect(decideSnapshotRevision(4, { revision: 12, resync: true })).toBe("accept");
  });

  it("deduplicates old frames and requests replay for gaps", () => {
    expect(decideSnapshotRevision(4, { revision: 4, resync: false })).toBe("duplicate");
    expect(decideSnapshotRevision(4, { revision: 7, resync: false })).toBe("replay");
  });
});

describe("optimistic message reconciliation", () => {
  const pending = [
    { id: "one", text: "first", baselineSeq: 4, attachments: [] },
    { id: "two", text: "second", baselineSeq: 5, attachments: [] },
  ];

  it("does not clear a message for an unrelated newer row", () => {
    expect(reconcilePendingMessages(pending, [{ seq: 6, role: "assistant", content: "first" }])).toEqual(pending);
  });

  it("clears only the matching user message newer than its baseline", () => {
    expect(reconcilePendingMessages(pending, [{ seq: 6, role: "user", content: "first" }])).toEqual([pending[1]]);
  });
});
