import { ed25519, x25519 } from "@noble/curves/ed25519.js";
import { describe, expect, it } from "vitest";

import { base64Url, fromBase64Url } from "./anywhereApi";
import {
  createRecoveryKitV2,
  deriveRecoveryWrapKey,
  deriveSelfDeviceWrapKey,
  generatePendingKeys,
  generateRecoveryPhrase,
  makeKeyWrap,
  recoveryEntropy,
  recoveryEntropyFromInput,
} from "./anywhereCrypto";
import { bytesToHex, openEnvelope } from "./transport/anywhereEnvelope";

describe("Anywhere account cryptography", () => {
  it("generates setup secrets when Hermes has no Web Crypto global", () => {
    const cryptoDescriptor = Object.getOwnPropertyDescriptor(globalThis, "crypto");
    const expoDescriptor = Object.getOwnPropertyDescriptor(globalThis, "expo");
    let uuidCounter = 0;
    Object.defineProperty(globalThis, "crypto", { configurable: true, value: undefined });
    Object.defineProperty(globalThis, "expo", {
      configurable: true,
      value: {
        uuidv4: () => {
          uuidCounter += 1;
          const hex = uuidCounter.toString(16).padStart(32, "0");
          return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-4${hex.slice(13, 16)}-8${hex.slice(17, 20)}-${hex.slice(20)}`;
        },
      },
    });
    try {
      expect(generatePendingKeys().signingPrivateKey).toHaveLength(32);
      expect(generateRecoveryPhrase().entropy).toHaveLength(16);
    } finally {
      if (cryptoDescriptor) Object.defineProperty(globalThis, "crypto", cryptoDescriptor);
      else Reflect.deleteProperty(globalThis, "crypto");
      if (expoDescriptor) Object.defineProperty(globalThis, "expo", expoDescriptor);
      else Reflect.deleteProperty(globalThis, "expo");
    }
  });

  it("round-trips URL-safe base64 without padding", () => {
    const bytes = new Uint8Array([0, 1, 2, 250, 251, 252, 253, 254, 255]);
    const encoded = base64Url(bytes);
    expect(encoded).not.toMatch(/[+/=]/);
    expect(fromBase64Url(encoded)).toEqual(bytes);
  });

  it("creates exactly 12 valid recovery words from 128 bits", () => {
    const recovery = generateRecoveryPhrase();
    expect(recovery.words.split(" ")).toHaveLength(12);
    expect(recoveryEntropy(recovery.words)).toEqual(recovery.entropy);
    expect(() => recoveryEntropy("abandon abandon")).toThrow(/12-word/);
  });

  it("continues accepting legacy 24-word recovery phrases", () => {
    const words = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";
    expect(recoveryEntropy(words)).toHaveLength(32);
  });

  it("binds v2 Recovery Kit files to the service and account", () => {
    const recovery = generateRecoveryPhrase();
    const account = "11".repeat(16);
    const kit = createRecoveryKitV2(recovery.words, "https://app.forge.test/", account);
    expect(recoveryEntropyFromInput(kit, "https://app.forge.test", account)).toEqual(recovery.entropy);
    expect(() => recoveryEntropyFromInput(kit, "https://other.forge.test", account)).toThrow("another Forge service");
    expect(() => recoveryEntropyFromInput(kit, "https://app.forge.test", "22".repeat(16))).toThrow("another account");
    const corrupted = kit.replace(/"checksum": "[0-9a-f]+"/, `"checksum": "${"00".repeat(32)}"`);
    expect(() => recoveryEntropyFromInput(corrupted, "https://app.forge.test", account)).toThrow("corrupted");
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

    expect(bytesToHex(deriveRecoveryWrapKey(new Uint8Array(16).fill(0x42), accountId, 1)))
      .toBe("fe1e8aec769b9f6c31a63ceb7bb58b592738f19d2c6cdf45b6fe82b0e1b2e15f");
  });
});
