import { ed25519, x25519 } from "@noble/curves/ed25519.js";
import { describe, expect, it } from "vitest";

import { base64Url, fromBase64Url } from "./anywhereApi";
import {
  deriveRecoveryWrapKey,
  deriveSelfDeviceWrapKey,
  generateRecoveryPhrase,
  makeKeyWrap,
  recoveryEntropy,
} from "./anywhereCrypto";
import { openEnvelope } from "./transport/anywhereEnvelope";

describe("Anywhere account cryptography", () => {
  it("round-trips URL-safe base64 without padding", () => {
    const bytes = new Uint8Array([0, 1, 2, 250, 251, 252, 253, 254, 255]);
    const encoded = base64Url(bytes);
    expect(encoded).not.toMatch(/[+/=]/);
    expect(fromBase64Url(encoded)).toEqual(bytes);
  });

  it("creates exactly 24 valid recovery words from 256 bits", () => {
    const recovery = generateRecoveryPhrase();
    expect(recovery.words.split(" ")).toHaveLength(24);
    expect(recoveryEntropy(recovery.words)).toEqual(recovery.entropy);
    expect(() => recoveryEntropy("abandon abandon")).toThrow(/24-word/);
  });

  it("derives and opens device and recovery key wraps", () => {
    const signingPrivate = new Uint8Array(32).fill(0x11);
    const exchangePrivate = new Uint8Array(32).fill(0x22);
    const exchangePublic = x25519.getPublicKey(exchangePrivate);
    const accountId = new Uint8Array(16).fill(0x33);
    const deviceId = new Uint8Array(16).fill(0x44);
    const dataKey = new Uint8Array(32).fill(0x55);
    const recovery = new Uint8Array(32).fill(0x66);

    const deviceWrapKey = deriveSelfDeviceWrapKey(exchangePrivate, exchangePublic, accountId, 9);
    const deviceWrap = makeKeyWrap(dataKey, deviceWrapKey, accountId, deviceId, 1, deviceId, 9, 0n, signingPrivate);
    expect(openEnvelope(deviceWrap, deviceWrapKey, ed25519.getPublicKey(signingPrivate)).plaintext).toEqual(dataKey);

    const recoveryWrapKey = deriveRecoveryWrapKey(recovery, accountId, 9);
    const recoveryWrap = makeKeyWrap(dataKey, recoveryWrapKey, accountId, deviceId, 3, accountId, 9, 1n, signingPrivate);
    expect(openEnvelope(recoveryWrap, recoveryWrapKey, ed25519.getPublicKey(signingPrivate)).plaintext).toEqual(dataKey);
  });
});
