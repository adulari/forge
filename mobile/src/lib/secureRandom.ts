import { hkdf } from "@noble/hashes/hkdf.js";
import { sha256 } from "@noble/hashes/sha2.js";

const RANDOM_SALT = sha256(new TextEncoder().encode("forge/mobile/native-random/v1"));
const RANDOM_INFO = new TextEncoder().encode("forge/mobile/secure-random-bytes/v1");

function uuidBytes(value: string): Uint8Array {
  const hex = value.replace(/-/g, "");
  if (!/^[0-9a-f]{32}$/i.test(hex)) throw new Error("Native secure random source returned an invalid UUID");
  return Uint8Array.from({ length: 16 }, (_, index) => Number.parseInt(hex.slice(index * 2, index * 2 + 2), 16));
}

function lengthInfo(length: number): Uint8Array {
  const info = new Uint8Array(RANDOM_INFO.length + 4);
  info.set(RANDOM_INFO);
  new DataView(info.buffer).setUint32(RANDOM_INFO.length, length, false);
  return info;
}

/**
 * Cryptographically secure bytes on browsers and Expo native runtimes.
 *
 * Hermes does not expose Web Crypto. Expo Modules Core is already embedded in every Forge
 * binary and its native UUID-v4 generator is backed by the operating system RNG. We combine
 * more UUID entropy than the requested output and extract it through HKDF-SHA256 so UUID
 * version/variant bits never leak into keys, nonces, or Recovery Kit entropy.
 */
export function secureRandomBytes(length: number): Uint8Array {
  if (!Number.isSafeInteger(length) || length < 0 || length > 65_536) {
    throw new RangeError("Secure random byte length must be between 0 and 65536");
  }
  if (length === 0) return new Uint8Array();

  const webCrypto = globalThis.crypto;
  if (typeof webCrypto?.getRandomValues === "function") {
    return webCrypto.getRandomValues(new Uint8Array(length));
  }

  const nativeUuidV4 = globalThis.expo?.uuidv4;
  if (typeof nativeUuidV4 !== "function") {
    throw new Error("The native secure random source is unavailable");
  }

  // UUID v4 contains 122 random bits. One extra sample provides extraction headroom.
  const sampleCount = Math.ceil((length * 8) / 122) + 1;
  const input = new Uint8Array(sampleCount * 16);
  for (let index = 0; index < sampleCount; index += 1) {
    input.set(uuidBytes(nativeUuidV4()), index * 16);
  }
  return hkdf(sha256, input, RANDOM_SALT, lengthInfo(length), length);
}
