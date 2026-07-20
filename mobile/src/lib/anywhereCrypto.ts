import { ed25519, x25519 } from "@noble/curves/ed25519.js";
import { hkdf } from "@noble/hashes/hkdf.js";
import { sha256 } from "@noble/hashes/sha2.js";
import { entropyToMnemonic, mnemonicToEntropy, validateMnemonic } from "@scure/bip39";
import { wordlist } from "@scure/bip39/wordlists/english.js";

import { fromBase64Url } from "./anywhereApi";
import {
  bytesFromHex,
  bytesToHex,
  decodeEnvelope,
  openEnvelope,
  sealEnvelope,
} from "./transport/anywhereEnvelope";

const DEVICE_WRAP_CONTEXT = new TextEncoder().encode("forge-anywhere/v1/device-wrap");
const RECOVERY_WRAP_V1_CONTEXT = new TextEncoder().encode("forge-anywhere/v1/recovery-wrap");
const RECOVERY_WRAP_V2_CONTEXT = new TextEncoder().encode("forge-anywhere/v2/recovery-wrap");

export interface PendingAnywhereKeys {
  signingPrivateKey: Uint8Array;
  exchangePrivateKey: Uint8Array;
  signingPublicKey: Uint8Array;
  exchangePublicKey: Uint8Array;
}

export interface RecoveryKitV2 {
  version: 2;
  service: string;
  account_id: string;
  words: string;
  checksum: string;
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
  const entropy = crypto.getRandomValues(new Uint8Array(16));
  return { words: entropyToMnemonic(entropy, wordlist), entropy };
}

export function recoveryEntropy(words: string): Uint8Array {
  const normalized = words.trim().toLowerCase().replace(/\s+/g, " ");
  const wordCount = normalized ? normalized.split(" ").length : 0;
  if (!matchesRecoveryWordCount(wordCount) || !validateMnemonic(normalized, wordlist)) {
    throw new Error("Enter a complete, valid 12-word Recovery Kit or legacy 24-word phrase");
  }
  return mnemonicToEntropy(normalized, wordlist);
}

export function createRecoveryKitV2(
  words: string,
  serviceUrl: string,
  accountIdHex: string,
): string {
  const entropy = recoveryEntropy(words);
  if (entropy.length !== 16) throw new Error("A v2 Recovery Kit requires a 12-word phrase");
  const service = normalizeService(serviceUrl);
  const accountId = bytesFromHex(accountIdHex);
  if (accountId.length !== 16) throw new Error("Recovery Kit account binding is invalid");
  const kit: RecoveryKitV2 = {
    version: 2,
    service,
    account_id: accountIdHex,
    words: words.trim().toLowerCase().replace(/\s+/g, " "),
    checksum: recoveryKitChecksum(service, accountId, entropy),
  };
  return JSON.stringify(kit, null, 2);
}

export function recoveryEntropyFromInput(
  input: string,
  serviceUrl: string,
  accountIdHex: string,
): Uint8Array {
  const trimmed = input.trim();
  if (!trimmed.startsWith("{")) return recoveryEntropy(trimmed);
  let kit: Partial<RecoveryKitV2>;
  try { kit = JSON.parse(trimmed) as Partial<RecoveryKitV2>; }
  catch { throw new Error("Recovery Kit file is malformed"); }
  const service = normalizeService(serviceUrl);
  if (kit.version !== 2 || typeof kit.words !== "string" || typeof kit.checksum !== "string"
    || typeof kit.service !== "string" || typeof kit.account_id !== "string") {
    throw new Error("Recovery Kit file is malformed");
  }
  if (kit.service !== service) throw new Error("Recovery Kit belongs to another Forge service");
  if (kit.account_id !== accountIdHex) throw new Error("Recovery Kit belongs to another account");
  const accountId = bytesFromHex(accountIdHex);
  const entropy = recoveryEntropy(kit.words);
  if (entropy.length !== 16
    || recoveryKitChecksum(service, accountId, entropy) !== kit.checksum) {
    throw new Error("Recovery Kit is corrupted");
  }
  return entropy;
}

export function deriveRecoveryWrapKey(entropy: Uint8Array, accountId: Uint8Array, epoch: number): Uint8Array {
  const context = entropy.length === 16
    ? RECOVERY_WRAP_V2_CONTEXT
    : entropy.length === 32
      ? RECOVERY_WRAP_V1_CONTEXT
      : null;
  if (!context) throw new Error("Recovery entropy must contain 128 or 256 bits");
  return hkdf(sha256, entropy, accountId, concat(context, u32(epoch)), 32);
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
  serviceUrl = "https://app.forge.adulari.dev",
): { dataKey: Uint8Array; epoch: number } {
  return openRecoveryWrapWithEntropy(
    encodedEnvelope,
    encodedSigningKey,
    recoveryEntropyFromInput(words, serviceUrl, accountIdHex),
    accountIdHex,
  );
}

export function openRecoveryWrapWithEntropy(
  encodedEnvelope: string,
  encodedSigningKey: string,
  entropy: Uint8Array,
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
  const key = deriveRecoveryWrapKey(
    entropy,
    accountId,
    envelope.metadata.keyEpoch,
  );
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

function normalizeService(value: string): string {
  const service = value.trim().replace(/\/+$/, "");
  if (!service) throw new Error("Recovery Kit service binding is invalid");
  return service;
}

function recoveryKitChecksum(service: string, accountId: Uint8Array, entropy: Uint8Array): string {
  return bytesToHex(sha256(concat(
    new TextEncoder().encode("forge-anywhere/v2/recovery-kit-checksum\0"),
    u64(BigInt(new TextEncoder().encode(service).length)),
    new TextEncoder().encode(service),
    accountId,
    entropy,
  )));
}

function u64(value: bigint): Uint8Array {
  const bytes = new Uint8Array(8);
  new DataView(bytes.buffer).setBigUint64(0, value, false);
  return bytes;
}

function equal(left: Uint8Array, right: Uint8Array): boolean {
  if (left.length !== right.length) return false;
  return left.every((byte, index) => byte === right[index]);
}

function matchesRecoveryWordCount(value: number): value is 12 | 24 {
  return value === 12 || value === 24;
}
