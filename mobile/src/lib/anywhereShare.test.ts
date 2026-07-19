import { ed25519 } from "@noble/curves/ed25519.js";
import { describe, expect, it, vi } from "vitest";

import { base64Url } from "./anywhereApi";
import { replayKeyFromHref, retrieveReplayShare } from "./anywhereShare";
import { sealEnvelope } from "./transport/anywhereEnvelope";

const key = new Uint8Array(32).fill(7);
const signingPrivate = new Uint8Array(32).fill(9);
const signingPublic = ed25519.getPublicKey(signingPrivate);
const shareId = "ab".repeat(16);
const created = 1_700_000_000_000;

function envelope(expires = created + 86_400_000): Uint8Array {
  const payload = new TextEncoder().encode(JSON.stringify({ version: 1, session_id: "session-1", created_at_ms: created, expires_at_ms: expires, replay: { rows: [] } }));
  return sealEnvelope({ kind: 7, flags: 0, accountId: new Uint8Array(16).fill(1), senderDeviceId: new Uint8Array(16).fill(2), recipientKind: 4, recipientId: new Uint8Array(16).fill(0xab), keyEpoch: 0, sequence: 1n, createdAtMs: BigInt(created), nonce: new Uint8Array(24).fill(3) }, payload, key, signingPrivate);
}

function response(bytes: Uint8Array): Response {
  return new Response(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer, { status: 200, headers: { "content-type": "application/vnd.forge-anywhere" } });
}

it("never transmits the fragment key", async () => {
  const fetcher = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) => response(envelope()));
  await retrieveReplayShare({ serviceUrl: "https://app.example", shareId, href: `https://app.example/shares/${shareId}#key=${base64Url(key)}&signing=${base64Url(signingPublic)}`, fetcher, resolveSigningKey: () => signingPublic, now: () => created + 1 });
  expect(fetcher).toHaveBeenCalledOnce();
  expect(fetcher.mock.calls[0]?.[0]).toBe(`https://app.example/v1/shares/${shareId}`);
  expect(JSON.stringify(fetcher.mock.calls[0])).not.toContain(base64Url(key));
  expect(JSON.stringify(fetcher.mock.calls[0])).not.toContain(base64Url(signingPublic));
});

describe("replay verification", () => {
  const input = (bytes: Uint8Array, suppliedKey = key, now = created + 1) => retrieveReplayShare({ serviceUrl: "https://app.example", shareId, href: `https://app.example/shares/${shareId}#key=${base64Url(suppliedKey)}&signing=${base64Url(signingPublic)}`, fetcher: async () => response(bytes), resolveSigningKey: () => signingPublic, now: () => now });
  it("rejects tampering", async () => { const bytes = envelope(); bytes[110] ^= 1; await expect(input(bytes)).rejects.toThrow(); });
  it("rejects the wrong key", async () => { await expect(input(envelope(), new Uint8Array(32).fill(4))).rejects.toThrow("authentication"); });
  it("rejects expiry", async () => { await expect(input(envelope(created + 10), key, created + 11)).rejects.toThrow("expired"); });
  it("rejects a signing key that disagrees with a locally trusted sender", async () => { await expect(retrieveReplayShare({ serviceUrl: "https://app.example", shareId, href: `https://app.example/shares/${shareId}#key=${base64Url(key)}&signing=${base64Url(new Uint8Array(32).fill(4))}`, fetcher: async () => response(envelope()), resolveSigningKey: () => signingPublic })).rejects.toThrow("trusted sender"); });
});

it("rejects alternate fragment fields and query leakage", () => {
  expect(() => replayKeyFromHref(`https://app.example/shares/${shareId}?key=x#key=${base64Url(key)}&signing=${base64Url(signingPublic)}`)).toThrow("query");
  expect(() => replayKeyFromHref(`https://app.example/shares/${shareId}#key=${base64Url(key)}&signing=${base64Url(signingPublic)}&next=x`)).toThrow("fragment");
  expect(() => replayKeyFromHref(`https://app.example/shares/${shareId}#key=${base64Url(key)}`)).toThrow("fragment");
});
