import { describe, expect, it } from "vitest";

import { base64Url } from "../anywhereApi";
import {
  decodeBase64Url,
  decodeWireBytes,
  encodeBase64Url,
  encodeWireBytes,
  maxOutboundInline,
  negotiateWire,
  WIRE_V1,
  WIRE_V2,
} from "./anywhereWire";

describe("anywhereWire", () => {
  it("matches the existing base64url encoder and Rust's URL_SAFE_NO_PAD", () => {
    for (const length of [0, 1, 2, 3, 4, 5, 16, 32, 4095, 4096, 4097, 70_000]) {
      const bytes = Uint8Array.from({ length }, (_, index) => (index * 37 + 11) & 0xff);
      expect(encodeBase64Url(bytes)).toBe(base64Url(bytes));
      expect(decodeBase64Url(encodeBase64Url(bytes))).toEqual(bytes);
    }
    // Vector shared with the Rust tests: sixteen 0xab bytes.
    expect(encodeBase64Url(new Uint8Array(16).fill(0xab))).toBe("q6urq6urq6urq6urq6urqw");
  });

  it("writes arrays for v1 and strings for v2, and reads both", () => {
    const bytes = new Uint8Array([0, 1, 254, 255]);
    expect(encodeWireBytes(bytes, WIRE_V1)).toEqual([0, 1, 254, 255]);
    expect(encodeWireBytes(bytes, WIRE_V2)).toBe("AAH-_w");
    expect(decodeWireBytes("body", [0, 1, 254, 255])).toEqual(bytes);
    expect(decodeWireBytes("body", "AAH-_w")).toEqual(bytes);
    expect(decodeWireBytes("body", "")).toEqual(new Uint8Array());
  });

  it("fails closed on malformed byte fields", () => {
    expect(() => decodeWireBytes("body", [256])).toThrow("body must contain bytes");
    expect(() => decodeWireBytes("body", [1.5])).toThrow("body must contain bytes");
    expect(() => decodeWireBytes("body", "not base64!")).toThrow("body must contain bytes");
    expect(() => decodeWireBytes("body", "AAH+/w")).toThrow("body must contain bytes");
    expect(() => decodeWireBytes("body", "A")).toThrow("body must contain bytes");
    // Non-zero trailing bits are rejected, exactly as the Rust decoder does.
    expect(() => decodeWireBytes("body", "AB")).toThrow("body must contain bytes");
    expect(() => decodeWireBytes("body", { length: 1 })).toThrow("body must contain bytes");
    expect(() => decodeWireBytes("body", null)).toThrow("body must contain bytes");
  });

  it("negotiates down to v1 for anything that is not an advertised v2 or later", () => {
    expect(negotiateWire(undefined)).toBe(WIRE_V1);
    expect(negotiateWire(1)).toBe(WIRE_V1);
    expect(negotiateWire("2")).toBe(WIRE_V1);
    expect(negotiateWire(2)).toBe(WIRE_V2);
    expect(negotiateWire(3)).toBe(WIRE_V2);
    expect(maxOutboundInline(WIRE_V1)).toBe(64 * 1024);
    expect(maxOutboundInline(WIRE_V2)).toBe(192 * 1024);
  });

  it("keeps a v2 inline body at the threshold under the relay's 300 KiB envelope cap", () => {
    const body = new Uint8Array(maxOutboundInline(WIRE_V2)).fill(0xff);
    const payload = JSON.stringify({
      request_id: encodeWireBytes(new Uint8Array(16), WIRE_V2),
      status: 200,
      headers: [["content-type", "application/json"]],
      body: encodeWireBytes(body, WIRE_V2),
      wire: 2,
    });
    expect(payload.length + 105 + 16 + 64).toBeLessThan(300 * 1024);
    const v1 = JSON.stringify({ body: encodeWireBytes(body, WIRE_V1) });
    expect(v1.length).toBeGreaterThan(300 * 1024);
  });
});
