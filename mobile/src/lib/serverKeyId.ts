import { sha256 } from "@noble/hashes/sha2.js";

/** A paired server's baseUrl is `origin/<token>`, so the raw value must never become a storage key. */
export function serverKeyId(baseUrl: string | null): string {
  const digest = sha256(new TextEncoder().encode(`forge/server-key/v1|${baseUrl ?? "unknown"}`));
  let hex = "";
  for (let i = 0; i < 16; i += 1) hex += digest[i].toString(16).padStart(2, "0");
  return hex;
}
