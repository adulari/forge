import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import {
  decodeDeviceKey,
  encodeDeviceKey,
  generateDeviceKey,
  loadOrCreateDeviceKey,
  openSealed,
  publicKeys,
  type DeviceKeyStorage,
} from "./pushE2e";

const hex = (value: string) => Uint8Array.from(value.match(/../g)!.map((byte) => parseInt(byte, 16)));

// Written by crates/forge-cli/src/apns_e2e/tests.rs (the daemon's sealer) and opened by the same
// test there, so Rust and this mirror of the Swift extension agree on the wire format.
const vector = JSON.parse(
  readFileSync(resolve(__dirname, "../../../../protocol/fixtures/push-notice-vector.json"), "utf8"),
) as { receiver_private_hex: string; p256dh: string; auth: string; enc: string; plaintext: unknown };

function seeded(): (length: number) => Uint8Array {
  let counter = 1;
  return (length) => Uint8Array.from({ length }, () => (counter++ * 37) % 251);
}

describe("push E2E key", () => {
  it("registers exactly the public half the daemon seals to", () => {
    const key = { privateKey: hex(vector.receiver_private_hex), auth: new Uint8Array(16).fill(3) };
    expect(publicKeys(key)).toEqual({ p256dh: vector.p256dh, auth: vector.auth });
  });

  it("registers a 65-byte uncompressed point and a 16-byte auth secret", () => {
    const keys = publicKeys(generateDeviceKey(seeded()));
    expect(keys.p256dh).toHaveLength(87);
    expect(keys.auth).toHaveLength(22);
  });

  it("round-trips through its keychain encoding and rejects damaged values", () => {
    const key = generateDeviceKey(seeded());
    expect(decodeDeviceKey(encodeDeviceKey(key))).toEqual(key);
    for (const bad of [null, "", "{", "[]", '{"v":2,"d":"AA","a":"AA"}', '{"v":1,"d":"AA","a":"AA"}']) {
      expect(decodeDeviceKey(bad)).toBeNull();
    }
  });

  it("creates a key once and reuses it, replacing a corrupt one", async () => {
    let stored: string | null = null;
    let saves = 0;
    const storage: DeviceKeyStorage = {
      load: async () => stored,
      save: async (value) => {
        stored = value;
        saves += 1;
      },
    };
    const first = await loadOrCreateDeviceKey(storage, seeded());
    const again = await loadOrCreateDeviceKey(storage, seeded());
    expect(again).toEqual(first);
    expect(saves).toBe(1);
    stored = "garbage";
    const replaced = await loadOrCreateDeviceKey(storage, seeded());
    expect(replaced.privateKey).toHaveLength(32);
    expect(saves).toBe(2);
  });
});

describe("opening a sealed notice", () => {
  const key = () => ({ privateKey: hex(vector.receiver_private_hex), auth: new Uint8Array(16).fill(3) });

  it("opens the daemon's cross-language vector", () => {
    expect(openSealed(key(), vector.enc)).toEqual(vector.plaintext);
  });

  it("returns null, never throws, for anything that is not a notice sealed to this key", () => {
    const other = generateDeviceKey(seeded());
    expect(openSealed(other, vector.enc)).toBeNull();
    expect(openSealed({ ...key(), auth: new Uint8Array(16) }, vector.enc)).toBeNull();
    const flipped = vector.enc.slice(0, -4) + (vector.enc.endsWith("AAAA") ? "BBBB" : "AAAA");
    for (const bad of ["", "!!!", "AAAA", flipped, vector.enc.slice(0, 80)]) {
      expect(openSealed(key(), bad)).toBeNull();
    }
  });
});
