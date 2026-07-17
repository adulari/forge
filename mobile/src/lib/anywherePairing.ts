export interface PairingChallenge { version: 1; pairing_id: string; exchange_public_key: string; expires_at_ms: number; service_origin: string }
export interface PairingCapability { supported: boolean; message: string }

export function parsePairingChallenge(value: string, serviceUrl: string, now = Date.now()): PairingChallenge {
  let encoded = value.trim();
  if (encoded.startsWith("forge-anywhere://pair?")) encoded = new URL(encoded).searchParams.get("challenge") ?? "";
  let challenge: PairingChallenge;
  try { challenge = JSON.parse(new TextDecoder().decode(decodeBase64(encoded))) as PairingChallenge; }
  catch { throw new Error("QR code is not a Forge Anywhere pairing challenge"); }
  const expectedOrigin = new URL(serviceUrl).origin;
  if (challenge.version !== 1 || !/^[0-9a-f]{32}$/.test(challenge.pairing_id)
    || !/^[A-Za-z0-9_-]{43}$/.test(challenge.exchange_public_key)
    || challenge.service_origin !== expectedOrigin) throw new Error("Pairing challenge is invalid for this service");
  if (!Number.isSafeInteger(challenge.expires_at_ms) || challenge.expires_at_ms <= now || challenge.expires_at_ms > now + 10 * 60_000) throw new Error("Pairing challenge has expired");
  return challenge;
}

export async function pairingCapability(serviceUrl: string, token: string, fetcher: typeof fetch = fetch): Promise<PairingCapability> {
  const response = await fetcher(new URL("/v1/pairings", serviceUrl), { method: "OPTIONS", headers: { authorization: `Bearer ${token}` }, cache: "no-store" });
  if (response.status === 404 || response.status === 405 || response.status === 501) return { supported: false, message: "Secure QR enrollment is not enabled by this Forge Anywhere service yet." };
  if (!response.ok) throw new Error(`Pairing capability check failed (${response.status})`);
  return { supported: true, message: "This service supports short-lived QR enrollment." };
}

function decodeBase64(value: string): Uint8Array {
  if (!/^[A-Za-z0-9_-]+$/.test(value)) throw new Error("base64url");
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
  const bytes: number[] = []; let bits = 0; let count = 0;
  for (const char of value) { bits = bits * 64 + alphabet.indexOf(char); count += 6; if (count >= 8) { count -= 8; bytes.push(Math.floor(bits / 2 ** count) & 255); bits %= 2 ** count; } }
  return new Uint8Array(bytes);
}
