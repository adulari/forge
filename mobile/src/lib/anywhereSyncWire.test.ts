import { describe, expect, it } from "vitest";

import { normalizeSyncRecord } from "./anywhereSyncBrowser";
import { encodeBase64Url } from "./transport/anywhereWire";

const v1 = {
  stable_id: "message/1/42",
  kind: "message",
  revision: 3,
  logical_clock: 99,
  device_id: Array(16).fill(5),
  operation: "upsert" as const,
  base_hash: Array(32).fill(6),
  content_hash: Array(32).fill(8),
  payload: Array.from(new TextEncoder().encode('{"text":"hi"}')),
};

describe("normalizeSyncRecord", () => {
  it("passes a v1 record through unchanged", () => {
    expect(normalizeSyncRecord(v1)).toEqual(v1);
  });

  it("reads a v2 record with base64url byte fields into the same shape", () => {
    const v2 = {
      ...v1,
      device_id: encodeBase64Url(new Uint8Array(v1.device_id)),
      base_hash: encodeBase64Url(new Uint8Array(v1.base_hash)),
      content_hash: encodeBase64Url(new Uint8Array(v1.content_hash)),
      payload: encodeBase64Url(new Uint8Array(v1.payload)),
    };
    expect(normalizeSyncRecord(v2)).toEqual(v1);
  });

  it("keeps a root base hash and an empty tombstone payload", () => {
    const tombstone = { ...v1, operation: "tombstone" as const, base_hash: null, payload: undefined };
    expect(normalizeSyncRecord(tombstone)).toMatchObject({ base_hash: null, payload: [] });
  });

  it("rejects malformed bytes", () => {
    expect(() => normalizeSyncRecord({ ...v1, payload: "!!" })).toThrow("sync record payload must contain bytes");
  });
});
