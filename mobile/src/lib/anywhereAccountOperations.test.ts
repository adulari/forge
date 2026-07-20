import { ed25519, x25519 } from "@noble/curves/ed25519.js";
import { describe, expect, it, vi } from "vitest";

import { base64Url, type AnywhereAuthSession } from "./anywhereApi";
import {
  DEFAULT_BILLING_PERIOD,
  billingCheckoutBody,
  commitPendingDeviceRevocation,
  prepareDeviceRevocation,
  refreshAnywhereCredentials,
  refreshAnywhereCredentialsExclusively,
  refreshPendingAnywhereAuth,
  stagePreparedDeviceRevocation,
} from "./anywhereAccountOperations";
import { deriveRecoveryWrapKey, generateRecoveryPhrase, makeKeyWrap } from "./anywhereCrypto";
import type { StoredAnywhereCredentials } from "./transport";
import { bytesToHex } from "./transport/anywhereEnvelope";

function credentials(): StoredAnywhereCredentials {
  return {
    version: 1,
    accountIdHex: "11".repeat(16),
    deviceIdHex: "22".repeat(16),
    signingPrivateKeyHex: "33".repeat(32),
    exchangePrivateKeyHex: "44".repeat(32),
    accountDataKeyHex: "55".repeat(32),
    keyEpoch: 4,
    accessToken: "old-access",
    refreshToken: "old-refresh",
    accessExpiresAtMs: 1,
    nextSequence: "8",
    acceptedSequences: {},
    signingPublicKeys: { ["22".repeat(16)]: bytesToHex(ed25519.getPublicKey(new Uint8Array(32).fill(0x33))) },
  };
}

describe("Anywhere account operations", () => {
  it("defaults checkout to the lower-priced annual plan", () => {
    expect(DEFAULT_BILLING_PERIOD).toBe("annual");
    expect(billingCheckoutBody()).toEqual({ billing_period: "annual" });
    expect(billingCheckoutBody("monthly")).toEqual({ billing_period: "monthly" });
  });

  it("rotates and persists expired access and refresh tokens", async () => {
    const persist = vi.fn(async () => undefined);
    const refresh = vi.fn(async (token: string) => {
      expect(token).toBe("old-refresh");
      return { access_token: "new-access", refresh_token: "new-refresh", access_expires_at_ms: 999_999 };
    });
    const result = await refreshAnywhereCredentials(credentials(), refresh, persist, 100);
    expect(result).toMatchObject({ accessToken: "new-access", refreshToken: "new-refresh", accessExpiresAtMs: 999_999 });
    expect(persist).toHaveBeenCalledOnce();
  });

  it("serializes browser tabs and adopts the token rotated by the first tab", async () => {
    let stored = credentials();
    let lock = Promise.resolve();
    const exclusive = <T>(work: () => Promise<T>): Promise<T> => {
      const result = lock.then(work);
      lock = result.then(() => undefined, () => undefined);
      return result;
    };
    const persist = vi.fn(async (next: StoredAnywhereCredentials) => { stored = next; });
    const refresh = vi.fn(async () => ({
      access_token: "shared-access",
      refresh_token: "shared-refresh",
      access_expires_at_ms: 999_999,
    }));

    const [first, second] = await Promise.all([
      refreshAnywhereCredentialsExclusively(credentials(), exclusive, async () => stored, refresh, persist, 100),
      refreshAnywhereCredentialsExclusively(credentials(), exclusive, async () => stored, refresh, persist, 100),
    ]);

    expect(refresh).toHaveBeenCalledOnce();
    expect(first.refreshToken).toBe("shared-refresh");
    expect(second.refreshToken).toBe("shared-refresh");
  });

  it("rotates expired credentials before delayed recovery enrollment", async () => {
    const pending: AnywhereAuthSession = {
      version: 1,
      account_id: "11".repeat(16),
      device_id: "22".repeat(16),
      github_login: "forge-user",
      access_token: "expired-access",
      refresh_token: "valid-refresh",
      access_expires_at_ms: 1,
      new_account: true,
    };
    const refresh = vi.fn(async (token: string) => {
      expect(token).toBe("valid-refresh");
      return { access_token: "fresh-access", refresh_token: "rotated-refresh", access_expires_at_ms: 999_999 };
    });

    await expect(refreshPendingAnywhereAuth(pending, refresh, 100)).resolves.toEqual({
      ...pending,
      access_token: "fresh-access",
      refresh_token: "rotated-refresh",
      access_expires_at_ms: 999_999,
    });
    expect(refresh).toHaveBeenCalledOnce();
  });

  it("durably retries an ambiguously committed revoke with exact bytes and key", async () => {
    const current = credentials();
    const recovery = generateRecoveryPhrase();
    const accountId = new Uint8Array(16).fill(0x11);
    const senderId = new Uint8Array(16).fill(0x22);
    const recoveryEnvelope = makeKeyWrap(
      new Uint8Array(32).fill(0x55),
      deriveRecoveryWrapKey(recovery.entropy, accountId, current.keyEpoch),
      accountId,
      senderId,
      3,
      accountId,
      current.keyEpoch,
      7n,
      new Uint8Array(32).fill(0x33),
    );
    const targetPrivate = new Uint8Array(32).fill(0x66);
    const prepared = prepareDeviceRevocation(
      current,
      [
        {
          id: current.deviceIdHex,
          name: "This device",
          created_at: "1",
          last_seen_at: "1",
          signing_public_key: base64Url(ed25519.getPublicKey(new Uint8Array(32).fill(0x33))),
          exchange_public_key: base64Url(x25519.getPublicKey(new Uint8Array(32).fill(0x44))),
        },
        {
          id: "77".repeat(16),
          name: "Old phone",
          created_at: "1",
          last_seen_at: null,
          signing_public_key: base64Url(ed25519.getPublicKey(targetPrivate)),
          exchange_public_key: base64Url(x25519.getPublicKey(targetPrivate)),
        },
      ],
      "77".repeat(16),
      recovery.words,
      {
        version: 1,
        epoch: current.keyEpoch,
        recovery_wrap_envelope: base64Url(recoveryEnvelope),
        signing_public_key: base64Url(ed25519.getPublicKey(new Uint8Array(32).fill(0x33))),
      },
      (length) => new Uint8Array(length).fill(0x88),
    );
    const persisted: StoredAnywhereCredentials[] = [];
    const persist = vi.fn(async (value: StoredAnywhereCredentials) => { persisted.push(value); });
    const pending = await stagePreparedDeviceRevocation(
      current, prepared, "77".repeat(16), "stable-key", persist,
    );
    expect(persisted[0]?.pendingDeviceRevocation).toEqual(pending);
    expect(persisted[0]?.keyEpoch).toBe(4);
    const submit = vi.fn(async (request, key: string): Promise<{ epoch: number }> => {
      expect(request).toEqual(pending.request);
      expect(key).toBe("stable-key");
      throw new Error("response lost after commit");
    });
    await commitPendingDeviceRevocation(pending, submit, async () => true, persist);
    expect(persisted.at(-1)).toMatchObject({ keyEpoch: 5, pendingDeviceRevocation: undefined });
    expect(submit).toHaveBeenCalledOnce();
    expect(current).toMatchObject({ keyEpoch: 4, accountDataKeyHex: "55".repeat(32), nextSequence: "8" });
  });
});
