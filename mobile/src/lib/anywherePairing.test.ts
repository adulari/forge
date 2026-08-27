import { ed25519, x25519 } from "@noble/curves/ed25519.js";
import { expect, it } from "vitest";
import { base64Url, fromBase64Url } from "./anywhereApi";
import { deriveDeviceWrapKey } from "./anywhereCrypto";
import type { PairingDetails } from "./anywherePairing";
import { describeRejectedPairings, listPairings, openApprovedPairing, parsePairingChallenge, pairingCapability, pairingSafetyCode, pollPairing, preparePairingApproval } from "./anywherePairing";
import type { StoredAnywhereCredentials } from "./transport";
import { bytesToHex, openEnvelope } from "./transport/anywhereEnvelope";

const pairingId = base64Url(new Uint8Array(32).fill(0xab));
const challenge = (expires: number) => base64Url(new TextEncoder().encode(JSON.stringify({ version: 1, pairing_id: pairingId, exchange_public_key: base64Url(new Uint8Array(32).fill(1)), expires_at_ms: expires, service_origin: "https://app.example" })));

it("accepts a same-service short-lived QR challenge", () => expect(parsePairingChallenge(challenge(101_000), "https://app.example", 100_000).pairing_id).toBe(pairingId));
it("rejects expired and overlong QR challenges", () => {
  expect(() => parsePairingChallenge(challenge(99_999), "https://app.example", 100_000)).toThrow("expired");
  expect(() => parsePairingChallenge(challenge(701_000), "https://app.example", 100_000)).toThrow("expired");
});
it("gates services without a pairing API explicitly", async () => expect(await pairingCapability("https://app.example", "token", async () => new Response(null, { status: 404 }))).toEqual({ supported: false, message: expect.stringContaining("not enabled") }));

it("exposes the service retry budget instead of turning a pairing 429 into a terminal failure", async () => {
  const result = pollPairing(
    "https://app.example",
    pairingId,
    base64Url(new Uint8Array(32).fill(0xcd)),
    async () => new Response(JSON.stringify({ code: "rate_limited" }), {
      status: 429,
      headers: { "retry-after": "12", "content-type": "application/json" },
    }),
  );
  await expect(result).rejects.toMatchObject({ retryAfterMs: 12_000 });
});

it("matches the CLI transcript-derived safety code", () => {
  expect(pairingSafetyCode({
    version: 1,
    pairing_id: base64Url(new Uint8Array(32).fill(1)),
    exchange_public_key: base64Url(new Uint8Array(32).fill(2)),
    expires_at_ms: 160_000,
    service_origin: "https://app.forge.test",
  }, base64Url(new Uint8Array(32).fill(3)), bytesToHex(new Uint8Array(16).fill(4))))
    .toBe("065 385");
});

it("wraps the current account key to a supported QR pairing challenge", () => {
  const accountId = new Uint8Array(16).fill(0x11);
  const senderId = new Uint8Array(16).fill(0x22);
  const recipientId = new Uint8Array(16).fill(0x33);
  const senderSigning = new Uint8Array(32).fill(0x44);
  const senderExchange = new Uint8Array(32).fill(0x55);
  const recipientExchange = new Uint8Array(32).fill(0x66);
  const dataKey = new Uint8Array(32).fill(0x77);
  const scanned = {
    version: 1 as const,
    pairing_id: pairingId,
    exchange_public_key: base64Url(x25519.getPublicKey(recipientExchange)),
    expires_at_ms: 101_000,
    service_origin: "https://app.example",
  };
  const credentials: StoredAnywhereCredentials = {
    version: 1, accountIdHex: bytesToHex(accountId), deviceIdHex: bytesToHex(senderId),
    signingPrivateKeyHex: bytesToHex(senderSigning), exchangePrivateKeyHex: bytesToHex(senderExchange),
    accountDataKeyHex: bytesToHex(dataKey), keyEpoch: 4, accessToken: "access", refreshToken: "refresh",
    accessExpiresAtMs: 1, nextSequence: "8", acceptedSequences: {}, signingPublicKeys: {},
  };
  const approval = preparePairingApproval(credentials, scanned, {
    version: 1, pairing_id: scanned.pairing_id, device_id: bytesToHex(recipientId), device_name: "phone",
    signing_public_key: base64Url(ed25519.getPublicKey(new Uint8Array(32).fill(0x78))),
    exchange_public_key: scanned.exchange_public_key, expires_at_ms: scanned.expires_at_ms,
  }, 8n);
  const recipientWrapKey = deriveDeviceWrapKey(recipientExchange, x25519.getPublicKey(senderExchange), accountId, 4);
  const opened = openEnvelope(
    fromBase64Url(approval.device_wrap_envelope),
    recipientWrapKey,
    ed25519.getPublicKey(senderSigning),
  );
  expect(opened.plaintext).toEqual(dataKey);
  expect(opened.metadata.recipientId).toEqual(recipientId);
  expect(opened.metadata.sequence).toBe(8n);
  expect(openApprovedPairing({
    version: 1,
    status: "approved",
    account_id: bytesToHex(accountId),
    device_id: bytesToHex(recipientId),
    access_token: "access",
    refresh_token: "refresh",
    access_expires_at_ms: 123,
    epoch: 4,
    device_wrap_envelope: approval.device_wrap_envelope,
    signing_public_key: base64Url(ed25519.getPublicKey(senderSigning)),
    exchange_public_key: base64Url(x25519.getPublicKey(senderExchange)),
  }, recipientExchange)).toEqual({ accountDataKey: dataKey, epoch: 4 });
});


// --- approval inbox -----------------------------------------------------------------------
// The inbox path had no coverage when it shipped: the suite above never imports listPairings, so
// a green run said nothing about it. These pin the behaviour the screen depends on.

const key32 = base64Url(new Uint8Array(32).fill(7));
const entry = (over: Partial<PairingDetails> = {}): PairingDetails => ({
  version: 1, pairing_id: pairingId, device_id: "a".repeat(32), device_name: "phone",
  signing_public_key: key32, exchange_public_key: key32,
  expires_at_ms: Date.now() + 600_000, ...over,
});

const inboxOf = (...pairings: PairingDetails[]) => async () =>
  new Response(JSON.stringify({ version: 1, pairings }), { headers: { "content-type": "application/json" } });

async function inbox(...pairings: PairingDetails[]) {
  const original = globalThis.fetch;
  globalThis.fetch = inboxOf(...pairings);
  try { return await listPairings("https://app.example", "token"); }
  finally { globalThis.fetch = original; }
}

it("keeps a valid pending request and rejects nothing", async () => {
  const result = await inbox(entry());
  expect(result.pairings).toHaveLength(1);
  expect(result.rejected).toEqual([]);
});

it("names why an entry could not be shown instead of dropping it silently", async () => {
  const result = await inbox(entry({ device_id: "not-hex" }));
  expect(result.pairings).toEqual([]);
  expect(result.rejected).toEqual([{ pairingId: `${pairingId.slice(0, 8)}\u2026`, reason: "malformed device id" }]);
});

it("rejects a key that is not 32 bytes, naming which key", async () => {
  const short = base64Url(new Uint8Array(31).fill(9));
  expect((await inbox(entry({ signing_public_key: short }))).rejected[0].reason).toBe("malformed signing key");
  expect((await inbox(entry({ exchange_public_key: short }))).rejected[0].reason).toBe("malformed exchange key");
});

// Expiry is ordinary lifecycle and the service already filters it, so the only way one arrives is
// clock skew. Reporting it would put a false alarm on the screen this path exists to make trusted.
it("drops an expired entry quietly rather than reporting it as a failure", async () => {
  const result = await inbox(entry({ expires_at_ms: Date.now() - 1_000 }));
  expect(result.pairings).toEqual([]);
  expect(result.rejected).toEqual([]);
});

it("never puts a whole opaque pairing id in the message", async () => {
  const { rejected } = await inbox(entry({ device_id: "not-hex" }));
  const message = describeRejectedPairings(rejected);
  expect(message).not.toContain(pairingId);
  expect(message).toContain(pairingId.slice(0, 8));
});

it("caps the banner at three entries and counts the rest", async () => {
  const many = Array.from({ length: 5 }, (_, i) => ({ pairingId: `id${i}`, reason: "malformed device id" }));
  const message = describeRejectedPairings(many);
  expect(message).toContain("5 device requests");
  expect(message).toContain("and 2 more");
  expect(message).not.toContain("id3");
  expect(message).not.toContain("id4");
});

it("says 1 device request, singular, for a single rejection", () => {
  expect(describeRejectedPairings([{ pairingId: "abc", reason: "malformed expiry" }])).toContain("1 device request could not");
});
