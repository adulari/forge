import { ed25519, x25519 } from "@noble/curves/ed25519.js";
import { describe, expect, it } from "vitest";

import { base64Url } from "./anywhereApi";
import { deriveDeviceWrapKey, makeKeyWrap } from "./anywhereCrypto";
import { promoteCurrentDeviceWrap } from "./anywhereEpochRefresh";
import type { StoredAnywhereCredentials } from "./transport";
import { bytesToHex } from "./transport/anywhereEnvelope";

const accountId = new Uint8Array(16).fill(0x11);
const recipientId = new Uint8Array(16).fill(0x22);
const senderId = new Uint8Array(16).fill(0x33);
const recipientExchangePrivate = new Uint8Array(32).fill(0x44);
const senderExchangePrivate = new Uint8Array(32).fill(0x55);
const senderSigningPrivate = new Uint8Array(32).fill(0x66);

function credentials(): StoredAnywhereCredentials {
  return {
    version: 1,
    accountIdHex: bytesToHex(accountId),
    deviceIdHex: bytesToHex(recipientId),
    signingPrivateKeyHex: "77".repeat(32),
    exchangePrivateKeyHex: bytesToHex(recipientExchangePrivate),
    accountDataKeyHex: "88".repeat(32),
    dataKeyEpochs: { "1": "88".repeat(32) },
    keyEpoch: 1,
    accessToken: "access",
    refreshToken: "refresh",
    accessExpiresAtMs: 1,
    nextSequence: "91",
    acceptedSequences: {},
    signingPublicKeys: {
      [bytesToHex(senderId)]: bytesToHex(ed25519.getPublicKey(senderSigningPrivate)),
    },
  };
}

function currentWrap(dataKey: Uint8Array) {
  const wrapKey = deriveDeviceWrapKey(
    senderExchangePrivate,
    x25519.getPublicKey(recipientExchangePrivate),
    accountId,
    2,
  );
  return {
    epoch: 2,
    device_wrap_envelope: base64Url(makeKeyWrap(
      dataKey, wrapKey, accountId, senderId, 1, recipientId, 2, 7n, senderSigningPrivate,
    )),
    signing_public_key: base64Url(ed25519.getPublicKey(senderSigningPrivate)),
    exchange_public_key: base64Url(x25519.getPublicKey(senderExchangePrivate)),
  };
}

describe("Anywhere account epoch refresh", () => {
  it("authenticates the device wrap and promotes the new key while retaining history", () => {
    const nextKey = new Uint8Array(32).fill(0x99);
    const promoted = promoteCurrentDeviceWrap(credentials(), currentWrap(nextKey));
    expect(promoted.keyEpoch).toBe(2);
    expect(promoted.accountDataKeyHex).toBe(bytesToHex(nextKey));
    expect(promoted.dataKeyEpochs).toEqual({ "1": "88".repeat(32), "2": bytesToHex(nextKey) });
    expect(promoted.nextSequence).toBe("0");
  });

  it("rejects a rotation signed by a device outside the trusted enrollment set", () => {
    const input = credentials();
    input.signingPublicKeys = {};
    expect(() => promoteCurrentDeviceWrap(input, currentWrap(new Uint8Array(32).fill(0x99))))
      .toThrow("not signed by an enrolled device");
  });
});
