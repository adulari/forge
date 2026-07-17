import { xchacha20poly1305 } from "@noble/ciphers/chacha.js";
import { ed25519 } from "@noble/curves/ed25519.js";

export const ANYWHERE_MAGIC = new Uint8Array([0x46, 0x41, 0x4e, 0x59]);
export const ANYWHERE_VERSION = 1;
export const ANYWHERE_HEADER_BYTES = 105;
export const ANYWHERE_SIGNATURE_BYTES = 64;
export const ANYWHERE_TAG_BYTES = 16;

export type EnvelopeKind = 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10;
export type RecipientKind = 1 | 2 | 3 | 4;

export interface EnvelopeMetadata {
  kind: EnvelopeKind;
  flags: number;
  accountId: Uint8Array;
  senderDeviceId: Uint8Array;
  recipientKind: RecipientKind;
  recipientId: Uint8Array;
  keyEpoch: number;
  sequence: bigint;
  createdAtMs: bigint;
  nonce: Uint8Array;
}

export interface DecodedEnvelope {
  metadata: EnvelopeMetadata;
  ciphertext: Uint8Array;
  signature: Uint8Array;
  header: Uint8Array;
}

export function sealEnvelope(
  metadata: EnvelopeMetadata,
  plaintext: Uint8Array,
  encryptionKey: Uint8Array,
  signingPrivateKey: Uint8Array,
): Uint8Array {
  assertBytes("encryption key", encryptionKey, 32);
  assertBytes("signing private key", signingPrivateKey, 32);
  const ciphertextLength = plaintext.length + ANYWHERE_TAG_BYTES;
  if (ciphertextLength > 0xffff_ffff) throw new Error("Anywhere plaintext is too large");
  const header = encodeHeader(metadata, ciphertextLength);
  const ciphertext = xchacha20poly1305(encryptionKey, metadata.nonce, header).encrypt(plaintext);
  const signature = ed25519.sign(concat(header, ciphertext), signingPrivateKey);
  return concat(header, ciphertext, signature);
}

export function decodeEnvelope(bytes: Uint8Array): DecodedEnvelope {
  if (bytes.length < ANYWHERE_HEADER_BYTES + ANYWHERE_TAG_BYTES + ANYWHERE_SIGNATURE_BYTES) {
    throw new Error("Anywhere envelope is shorter than the v1 minimum");
  }
  if (!equal(bytes.subarray(0, 4), ANYWHERE_MAGIC)) throw new Error("invalid Anywhere magic");
  if (bytes[4] !== ANYWHERE_VERSION) {
    throw new Error(`unsupported Anywhere version ${bytes[4]}`);
  }
  const kind = bytes[5];
  if (kind < 1 || kind > 10) throw new Error(`unknown Anywhere kind ${kind}`);
  const recipientKind = bytes[40];
  if (recipientKind < 1 || recipientKind > 4) {
    throw new Error(`unknown Anywhere recipient kind ${recipientKind}`);
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const ciphertextLength = view.getUint32(101, false);
  const ciphertextEnd = ANYWHERE_HEADER_BYTES + ciphertextLength;
  if (
    ciphertextLength < ANYWHERE_TAG_BYTES ||
    ciphertextEnd + ANYWHERE_SIGNATURE_BYTES !== bytes.length
  ) {
    throw new Error("invalid Anywhere ciphertext length");
  }
  return {
    metadata: {
      kind: kind as EnvelopeKind,
      flags: view.getUint16(6, false),
      accountId: bytes.slice(8, 24),
      senderDeviceId: bytes.slice(24, 40),
      recipientKind: recipientKind as RecipientKind,
      recipientId: bytes.slice(41, 57),
      keyEpoch: view.getUint32(57, false),
      sequence: view.getBigUint64(61, false),
      createdAtMs: view.getBigUint64(69, false),
      nonce: bytes.slice(77, 101),
    },
    header: bytes.slice(0, ANYWHERE_HEADER_BYTES),
    ciphertext: bytes.slice(ANYWHERE_HEADER_BYTES, ciphertextEnd),
    signature: bytes.slice(ciphertextEnd),
  };
}

export function verifyEnvelope(
  envelope: DecodedEnvelope,
  signingPublicKey: Uint8Array,
): boolean {
  assertBytes("signing public key", signingPublicKey, 32);
  return ed25519.verify(
    envelope.signature,
    concat(envelope.header, envelope.ciphertext),
    signingPublicKey,
    { zip215: false },
  );
}

export function openEnvelope(
  bytes: Uint8Array,
  encryptionKey: Uint8Array,
  signingPublicKey: Uint8Array,
): { metadata: EnvelopeMetadata; plaintext: Uint8Array } {
  assertBytes("encryption key", encryptionKey, 32);
  const envelope = decodeEnvelope(bytes);
  if (!verifyEnvelope(envelope, signingPublicKey)) {
    throw new Error("invalid Anywhere sender signature");
  }
  let plaintext: Uint8Array;
  try {
    plaintext = xchacha20poly1305(
      encryptionKey,
      envelope.metadata.nonce,
      envelope.header,
    ).decrypt(envelope.ciphertext);
  } catch {
    throw new Error("Anywhere payload authentication failed");
  }
  return { metadata: envelope.metadata, plaintext };
}

function encodeHeader(metadata: EnvelopeMetadata, ciphertextLength: number): Uint8Array {
  assertBytes("account id", metadata.accountId, 16);
  assertBytes("sender device id", metadata.senderDeviceId, 16);
  assertBytes("recipient id", metadata.recipientId, 16);
  assertBytes("nonce", metadata.nonce, 24);
  if (!Number.isInteger(metadata.flags) || metadata.flags < 0 || metadata.flags > 0xffff) {
    throw new Error("Anywhere flags must fit u16");
  }
  if (!Number.isInteger(metadata.keyEpoch) || metadata.keyEpoch < 0 || metadata.keyEpoch > 0xffff_ffff) {
    throw new Error("Anywhere key epoch must fit u32");
  }
  const header = new Uint8Array(ANYWHERE_HEADER_BYTES);
  header.set(ANYWHERE_MAGIC, 0);
  header[4] = ANYWHERE_VERSION;
  header[5] = metadata.kind;
  header.set(metadata.accountId, 8);
  header.set(metadata.senderDeviceId, 24);
  header[40] = metadata.recipientKind;
  header.set(metadata.recipientId, 41);
  header.set(metadata.nonce, 77);
  const view = new DataView(header.buffer);
  view.setUint16(6, metadata.flags, false);
  view.setUint32(57, metadata.keyEpoch, false);
  view.setBigUint64(61, metadata.sequence, false);
  view.setBigUint64(69, metadata.createdAtMs, false);
  view.setUint32(101, ciphertextLength, false);
  return header;
}

function concat(...parts: Uint8Array[]): Uint8Array {
  const output = new Uint8Array(parts.reduce((length, part) => length + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    output.set(part, offset);
    offset += part.length;
  }
  return output;
}

function equal(left: Uint8Array, right: Uint8Array): boolean {
  if (left.length !== right.length) return false;
  let difference = 0;
  for (let index = 0; index < left.length; index += 1) {
    difference |= left[index] ^ right[index];
  }
  return difference === 0;
}

function assertBytes(label: string, value: Uint8Array, length: number): void {
  if (value.length !== length) throw new Error(`${label} must contain ${length} bytes`);
}

export function bytesFromHex(value: string): Uint8Array {
  if (value.length % 2 !== 0 || !/^[0-9a-f]*$/i.test(value)) throw new Error("invalid hex");
  const output = new Uint8Array(value.length / 2);
  for (let index = 0; index < output.length; index += 1) {
    output[index] = Number.parseInt(value.slice(index * 2, index * 2 + 2), 16);
  }
  return output;
}

export function bytesToHex(value: Uint8Array): string {
  return Array.from(value, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

