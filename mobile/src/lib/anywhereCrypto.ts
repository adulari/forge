import { ed25519, x25519 } from "@noble/curves/ed25519.js";
import { hkdf } from "@noble/hashes/hkdf.js";
import { sha256 } from "@noble/hashes/sha2.js";
import { entropyToMnemonic, mnemonicToEntropy, validateMnemonic } from "@scure/bip39";
import { wordlist } from "@scure/bip39/wordlists/english.js";

import { fromBase64Url } from "./anywhereApi";
import {
  bytesFromHex,
  decodeEnvelope,
  openEnvelope,
  sealEnvelope,
} from "./transport/anywhereEnvelope";

const DEVICE_WRAP_CONTEXT = new TextEncoder().encode("forge-anywhere/v1/device-wrap");
const RECOVERY_WRAP_CONTEXT = new TextEncoder().encode("forge-anywhere/v1/recovery-wrap");

export interface PendingAnywhereKeys {
  signingPrivateKey: Uint8Array;
  exchangePrivateKey: Uint8Array;
  signingPublicKey: Uint8Array;
  exchangePublicKey: Uint8Array;
}

export function generatePendingKeys(): PendingAnywhereKeys {
  const signingPrivateKey = crypto.getRandomValues(new Uint8Array(32));
  const exchangePrivateKey = crypto.getRandomValues(new Uint8Array(32));
  return {
    signingPrivateKey,
    exchangePrivateKey,
    signingPublicKey: ed25519.getPublicKey(signingPrivateKey),
    exchangePublicKey: x25519.getPublicKey(exchangePrivateKey),
  };
}

export function generateRecoveryPhrase(): { words: string; entropy: Uint8Array } {
  const entropy = crypto.getRandomValues(new Uint8Array(32));
  return { words: entropyToMnemonic(entropy, wordlist), entropy };
}

export function recoveryEntropy(words: string): Uint8Array {
  const normalized = words.trim().toLowerCase().replace(/\s+/g, " ");
  if (normalized.split(" ").length !== 24 || !validateMnemonic(normalized, wordlist)) {
    throw new Error("Enter the complete, valid 24-word recovery phrase");
  }
  return mnemonicToEntropy(normalized, wordlist);
}

export function deriveRecoveryWrapKey(entropy: Uint8Array, accountId: Uint8Array, epoch: number): Uint8Array {
  return hkdf(sha256, entropy, accountId, concat(RECOVERY_WRAP_CONTEXT, u32(epoch)), 32);
}

export function deriveSelfDeviceWrapKey(
  exchangePrivateKey: Uint8Array,
  exchangePublicKey: Uint8Array,
  accountId: Uint8Array,
  epoch: number,
): Uint8Array {
  return deriveDeviceWrapKey(exchangePrivateKey, exchangePublicKey, accountId, epoch);
}

export function deriveDeviceWrapKey(
  exchangePrivateKey: Uint8Array,
  recipientExchangePublicKey: Uint8Array,
  accountId: Uint8Array,
  epoch: number,
): Uint8Array {
  const shared = x25519.getSharedSecret(exchangePrivateKey, recipientExchangePublicKey);
  if (shared.every((byte) => byte === 0)) throw new Error("Invalid device exchange key");
  return hkdf(sha256, shared, accountId, concat(DEVICE_WRAP_CONTEXT, u32(epoch)), 32);
}

export function makeKeyWrap(
  dataKey: Uint8Array,
  wrapKey: Uint8Array,
  accountId: Uint8Array,
  deviceId: Uint8Array,
  recipientKind: 1 | 3,
  recipientId: Uint8Array,
  epoch: number,
  sequence: bigint,
  signingPrivateKey: Uint8Array,
): Uint8Array {
  return sealEnvelope({
    kind: 5,
    flags: 0,
    accountId,
    senderDeviceId: deviceId,
    recipientKind,
    recipientId,
    keyEpoch: epoch,
    sequence,
    createdAtMs: BigInt(Date.now()),
    nonce: crypto.getRandomValues(new Uint8Array(24)),
  }, dataKey, wrapKey, signingPrivateKey);
}

export function openRecoveryWrap(
  encodedEnvelope: string,
  encodedSigningKey: string,
  words: string,
  accountIdHex: string,
): { dataKey: Uint8Array; epoch: number } {
  const envelopeBytes = fromBase64Url(encodedEnvelope);
  const envelope = decodeEnvelope(envelopeBytes);
  const accountId = bytesFromHex(accountIdHex);
  if (envelope.metadata.kind !== 5 || envelope.metadata.recipientKind !== 3) {
    throw new Error("Recovery wrap has invalid routing metadata");
  }
  if (!equal(envelope.metadata.accountId, accountId) || !equal(envelope.metadata.recipientId, accountId)) {
    throw new Error("Recovery wrap belongs to another account");
  }
  const key = deriveRecoveryWrapKey(recoveryEntropy(words), accountId, envelope.metadata.keyEpoch);
  const opened = openEnvelope(envelopeBytes, key, fromBase64Url(encodedSigningKey));
  if (opened.plaintext.length !== 32) throw new Error("Recovered account key has an invalid length");
  return { dataKey: opened.plaintext, epoch: envelope.metadata.keyEpoch };
}

function u32(value: number): Uint8Array {
  const bytes = new Uint8Array(4);
  new DataView(bytes.buffer).setUint32(0, value, false);
  return bytes;
}

function concat(...parts: Uint8Array[]): Uint8Array {
  const output = new Uint8Array(parts.reduce((total, part) => total + part.length, 0));
  let offset = 0;
  for (const part of parts) { output.set(part, offset); offset += part.length; }
  return output;
}

function equal(left: Uint8Array, right: Uint8Array): boolean {
  if (left.length !== right.length) return false;
  return left.every((byte, index) => byte === right[index]);
}
