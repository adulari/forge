// Negotiated encoding of byte fields inside sealed Anywhere payloads (docs/rfcs/anywhere-wire-v2-and-device-revoke.md).
//
// Wire v1 writes bytes as JSON number arrays (~3.57x); wire v2 writes unpadded base64url
// (1.33x). The FANY envelope, its AEAD and signature do not change. Every reader accepts both
// forms; a sender writes v2 only to a peer that advertised `"wire": 2`, so an older host keeps
// receiving exactly the arrays it has always parsed.

export const WIRE_V1 = 1;
export const WIRE_V2 = 2;
/** The highest version this build reads; sent as `wire` on every bridge request. */
export const WIRE_LATEST = WIRE_V2;

export type WireBytes = number[] | string;

/** 64 KiB seals to ~229 KiB in v1, under the relay's 300 KiB envelope cap. */
const MAX_OUTBOUND_INLINE_V1 = 64 * 1024;
/** 192 KiB encodes to 256 KiB in v2: under the relay cap and every host's 256 KiB inline limit. */
const MAX_OUTBOUND_INLINE_V2 = 192 * 1024;

export function negotiateWire(peer: unknown): number {
  return typeof peer === "number" && Number.isInteger(peer) && peer >= WIRE_V2 ? WIRE_V2 : WIRE_V1;
}

export function maxOutboundInline(wire: number): number {
  return wire >= WIRE_V2 ? MAX_OUTBOUND_INLINE_V2 : MAX_OUTBOUND_INLINE_V1;
}

export function encodeWireBytes(bytes: Uint8Array, wire: number): WireBytes {
  return wire >= WIRE_V2 ? encodeBase64Url(bytes) : Array.from(bytes);
}

/** Decodes a byte field in either form; throws on anything else, so a malformed frame fails closed. */
export function decodeWireBytes(label: string, value: unknown): Uint8Array {
  if (typeof value === "string") {
    try {
      return decodeBase64Url(value);
    } catch {
      throw new Error(`${label} must contain bytes`);
    }
  }
  if (Array.isArray(value) && value.every((byte) => Number.isInteger(byte) && byte >= 0 && byte <= 255)) {
    return new Uint8Array(value as number[]);
  }
  throw new Error(`${label} must contain bytes`);
}

export function decodeOptionalWireBytes(label: string, value: unknown): Uint8Array {
  return value === undefined ? new Uint8Array() : decodeWireBytes(label, value);
}

export function wireByteLength(value: unknown): number {
  if (typeof value === "string") return Math.floor((value.length * 3) / 4);
  return Array.isArray(value) ? value.length : 0;
}

const ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const DIGITS = (() => {
  const table = new Int16Array(128).fill(-1);
  for (let index = 0; index < ALPHABET.length; index += 1) table[ALPHABET.charCodeAt(index)] = index;
  return table;
})();
const CODES = Array.from(ALPHABET, (character) => character.charCodeAt(0));

export function encodeBase64Url(bytes: Uint8Array): string {
  // Chunked fromCharCode: a 192 KiB body built a character at a time was the slow path on Hermes.
  const chunks: string[] = [];
  const chunk = new Array<number>(4096);
  let filled = 0;
  const flush = () => {
    chunks.push(String.fromCharCode.apply(null, chunk.slice(0, filled)));
    filled = 0;
  };
  for (let index = 0; index < bytes.length; index += 3) {
    const first = bytes[index];
    const second = index + 1 < bytes.length ? bytes[index + 1] : -1;
    const third = index + 2 < bytes.length ? bytes[index + 2] : -1;
    chunk[filled++] = CODES[first >>> 2];
    chunk[filled++] = CODES[((first & 0x03) << 4) | (second < 0 ? 0 : second >>> 4)];
    if (second >= 0) chunk[filled++] = CODES[((second & 0x0f) << 2) | (third < 0 ? 0 : third >>> 6)];
    if (third >= 0) chunk[filled++] = CODES[third & 0x3f];
    if (filled > chunk.length - 4) flush();
  }
  if (filled > 0) flush();
  return chunks.join("");
}

export function decodeBase64Url(value: string): Uint8Array {
  if (value.length % 4 === 1) throw new Error("invalid base64url");
  const output = new Uint8Array(Math.floor((value.length * 3) / 4));
  let bits = 0;
  let bitCount = 0;
  let offset = 0;
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    const digit = code < 128 ? DIGITS[code] : -1;
    if (digit < 0) throw new Error("invalid base64url");
    bits = ((bits << 6) | digit) & 0xffffff;
    bitCount += 6;
    if (bitCount >= 8) {
      bitCount -= 8;
      output[offset] = (bits >>> bitCount) & 0xff;
      offset += 1;
    }
  }
  // Rust's decoder rejects non-zero trailing bits; match it so both sides agree on validity.
  if (bitCount > 0 && (bits & ((1 << bitCount) - 1)) !== 0) throw new Error("non-canonical base64url");
  return output;
}
