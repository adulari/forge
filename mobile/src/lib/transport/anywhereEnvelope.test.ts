import { ed25519 } from "@noble/curves/ed25519.js";
import { describe, expect, it } from "vitest";

import fixture from "../../../../protocol/fixtures/anywhere-v1/envelope-bridge-request.json";
import {
  bytesFromHex,
  bytesToHex,
  decodeEnvelope,
  openEnvelope,
  sealEnvelope,
  type EnvelopeKind,
  type RecipientKind,
} from "./anywhereEnvelope";

describe("Anywhere v1 golden envelope", () => {
  it("matches the Rust/backend fixture byte for byte", () => {
    const privateKey = bytesFromHex(fixture.signing_private_key_hex);
    expect(bytesToHex(ed25519.getPublicKey(privateKey))).toBe(fixture.signing_public_key_hex);
    const encoded = sealEnvelope(
      {
        kind: fixture.kind as EnvelopeKind,
        flags: fixture.flags,
        accountId: bytesFromHex(fixture.account_id_hex),
        senderDeviceId: bytesFromHex(fixture.sender_device_id_hex),
        recipientKind: fixture.recipient_kind as RecipientKind,
        recipientId: bytesFromHex(fixture.recipient_id_hex),
        keyEpoch: fixture.key_epoch,
        sequence: BigInt(fixture.sequence),
        createdAtMs: BigInt(fixture.created_at_ms),
        nonce: bytesFromHex(fixture.nonce_hex),
      },
      bytesFromHex(fixture.plaintext_hex),
      bytesFromHex(fixture.encryption_key_hex),
      privateKey,
    );
    expect(bytesToHex(encoded)).toBe(fixture.envelope_hex);
  });

  it("opens the normative fixture and rejects wrong keys or tampering", () => {
    const encoded = bytesFromHex(fixture.envelope_hex);
    const opened = openEnvelope(
      encoded,
      bytesFromHex(fixture.encryption_key_hex),
      bytesFromHex(fixture.signing_public_key_hex),
    );
    expect(bytesToHex(opened.plaintext)).toBe(fixture.plaintext_hex);
    expect(() =>
      openEnvelope(encoded, new Uint8Array(32), bytesFromHex(fixture.signing_public_key_hex)),
    ).toThrow("authentication failed");

    const corrupted = encoded.slice();
    corrupted[105] ^= 1;
    expect(() =>
      openEnvelope(
        corrupted,
        bytesFromHex(fixture.encryption_key_hex),
        bytesFromHex(fixture.signing_public_key_hex),
      ),
    ).toThrow("signature");
  });

  it("rejects non-canonical trailing bytes", () => {
    const encoded = bytesFromHex(`${fixture.envelope_hex}00`);
    expect(() => decodeEnvelope(encoded)).toThrow("ciphertext length");
  });
});
