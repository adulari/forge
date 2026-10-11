// End-to-end sealed notification text (ADR-0012, "End-to-end sealed alerts"). Pure and
// platform-agnostic so it is unit-testable; the keychain storage and registration live in
// push.ios.ts, and the decrypting half that runs on the lock screen is the Swift Notification
// Service Extension (targets/notification-service), which this file mirrors byte for byte.
//
// The phone owns a P-256 key and a 16-byte auth secret. Only the public half and the auth secret
// are sent to the daemon (over the same authenticated channel as the APNs token); the daemon seals
// each notice with RFC 8291 aes128gcm, the construction Web Push already uses, and the hosted relay
// and Apple only ever carry the ciphertext in the `enc` payload field.
import { gcm } from "@noble/ciphers/aes.js";
import { p256 } from "@noble/curves/nist.js";
import { hkdf } from "@noble/hashes/hkdf.js";
import { sha256 } from "@noble/hashes/sha2.js";

import { base64Url as toBase64Url, fromBase64Url } from "../anywhereApi";
import { secureRandomBytes } from "../secureRandom";

/** The keychain access group shared by the app and its Notification Service Extension. */
export const PUSH_KEY_ACCESS_GROUP = "95VXXPD28Y.dev.adulari.forge.pushkeys";
/** The keychain service and key the extension queries; keep in sync with ForgePushKey.swift. */
export const PUSH_KEY_SERVICE = "forge.push.e2e";
export const PUSH_KEY_NAME = "forge-push-e2e-v1";

export interface DeviceKey {
  /** 32-byte P-256 scalar. Never leaves the device keychain. */
  privateKey: Uint8Array;
  /** 16-byte RFC 8291 auth secret. */
  auth: Uint8Array;
}

/** What the daemon stores and seals to: the `keys` object of `POST /api/push/subscribe`. */
export interface PushKeys {
  p256dh: string;
  auth: string;
}

export interface SealedNotice {
  v: number;
  title: string;
  message: string;
  session_id: string;
  seq: number;
  kind: string;
}

export function generateDeviceKey(random: (length: number) => Uint8Array = secureRandomBytes): DeviceKey {
  return { privateKey: p256.utils.randomSecretKey(random(48)), auth: random(16) };
}

export function publicKeys(key: DeviceKey): PushKeys {
  return {
    p256dh: toBase64Url(p256.getPublicKey(key.privateKey, false)),
    auth: toBase64Url(key.auth),
  };
}

export function encodeDeviceKey(key: DeviceKey): string {
  return JSON.stringify({ v: 1, d: toBase64Url(key.privateKey), a: toBase64Url(key.auth) });
}

export function decodeDeviceKey(raw: string | null): DeviceKey | null {
  if (!raw) return null;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return null;
    const { v, d, a } = parsed as Record<string, unknown>;
    if (v !== 1 || typeof d !== "string" || typeof a !== "string") return null;
    const privateKey = fromBase64Url(d);
    const auth = fromBase64Url(a);
    if (auth.length !== 16 || !p256.utils.isValidSecretKey(privateKey)) return null;
    return { privateKey, auth };
  } catch {
    return null;
  }
}

export interface DeviceKeyStorage {
  load(): Promise<string | null>;
  save(value: string): Promise<void>;
}

/** The stored key, or a fresh one persisted first. A key that fails to decode is replaced: the
 * daemon is re-registered with the new public half right after, so nothing keeps sealing to it. */
export async function loadOrCreateDeviceKey(
  storage: DeviceKeyStorage,
  random: (length: number) => Uint8Array = secureRandomBytes,
): Promise<DeviceKey> {
  const existing = decodeDeviceKey(await storage.load());
  if (existing) return existing;
  const created = generateDeviceKey(random);
  await storage.save(encodeDeviceKey(created));
  return created;
}

const enc = new TextEncoder();

function concat(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

/** RFC 8291 (aes128gcm) decrypt, the same steps as ForgePushCrypto.swift. Returns null on any
 * malformed or unauthentic input: the extension then leaves the generic alert untouched. */
export function openSealed(key: DeviceKey, blob: string): SealedNotice | null {
  try {
    const message = fromBase64Url(blob);
    // salt(16) || record size(4) || key id length(1) || key id (sender public key) || ciphertext
    if (message.length < 16 + 4 + 1 + 65 + 17) return null;
    const salt = message.slice(0, 16);
    const idLength = message[20];
    if (idLength !== 65) return null;
    const senderPublic = message.slice(21, 21 + 65);
    const ciphertext = message.slice(21 + 65);

    const uaPublic = p256.getPublicKey(key.privateKey, false);
    const ecdh = p256.getSharedSecret(key.privateKey, senderPublic, true).slice(1);
    const info = concat(enc.encode("WebPush: info\0"), uaPublic, senderPublic);
    const ikm = hkdf(sha256, ecdh, key.auth, info, 32);
    const cek = hkdf(sha256, ikm, salt, enc.encode("Content-Encoding: aes128gcm\0"), 16);
    const nonce = hkdf(sha256, ikm, salt, enc.encode("Content-Encoding: nonce\0"), 12);

    const record = gcm(cek, nonce).decrypt(ciphertext);
    let end = record.length;
    while (end > 0 && record[end - 1] === 0) end -= 1;
    if (end === 0 || record[end - 1] !== 0x02) return null;
    const parsed: unknown = JSON.parse(new TextDecoder().decode(record.slice(0, end - 1)));
    if (typeof parsed !== "object" || parsed === null) return null;
    const { v, title, message: text, session_id, seq, kind } = parsed as Record<string, unknown>;
    if (v !== 1 || typeof title !== "string" || typeof text !== "string") return null;
    return {
      v,
      title,
      message: text,
      session_id: typeof session_id === "string" ? session_id : "",
      seq: typeof seq === "number" ? seq : 0,
      kind: typeof kind === "string" ? kind : "",
    };
  } catch {
    return null;
  }
}
