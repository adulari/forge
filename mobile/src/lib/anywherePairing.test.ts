import { expect, it } from "vitest";
import { base64Url } from "./anywhereApi";
import { parsePairingChallenge, pairingCapability } from "./anywherePairing";

const challenge = (expires: number) => base64Url(new TextEncoder().encode(JSON.stringify({ version: 1, pairing_id: "ab".repeat(16), exchange_public_key: base64Url(new Uint8Array(32).fill(1)), expires_at_ms: expires, service_origin: "https://app.example" })));

it("accepts a same-service short-lived QR challenge", () => expect(parsePairingChallenge(challenge(101_000), "https://app.example", 100_000).pairing_id).toBe("ab".repeat(16)));
it("rejects expired and overlong QR challenges", () => {
  expect(() => parsePairingChallenge(challenge(99_999), "https://app.example", 100_000)).toThrow("expired");
  expect(() => parsePairingChallenge(challenge(701_000), "https://app.example", 100_000)).toThrow("expired");
});
it("gates services without a pairing API explicitly", async () => expect(await pairingCapability("https://app.example", "token", async () => new Response(null, { status: 404 }))).toEqual({ supported: false, message: expect.stringContaining("not enabled") }));
