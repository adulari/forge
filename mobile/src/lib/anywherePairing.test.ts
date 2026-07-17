import { ed25519, x25519 } from "@noble/curves/ed25519.js";
import { expect, it } from "vitest";
import { base64Url, fromBase64Url } from "./anywhereApi";
import { deriveDeviceWrapKey } from "./anywhereCrypto";
import { openApprovedPairing, parsePairingChallenge, pairingCapability, preparePairingApproval } from "./anywherePairing";
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
