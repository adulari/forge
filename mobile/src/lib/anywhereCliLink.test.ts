import { describe, expect, it, vi } from "vitest";

import { linkWithLocalCli, type CliLinkDeps } from "./anywhereCliLink";

const ACCOUNT = "0123456789abcdef0123456789abcdef";
const keys = {
  signingPrivateKey: new Uint8Array(32).fill(1),
  signingPublicKey: new Uint8Array(32).fill(2),
  exchangePrivateKey: new Uint8Array(32).fill(3),
  exchangePublicKey: new Uint8Array(32).fill(4),
};
const created = { version: 1 as const, pairing_id: "pid", pairing_token: "ptok", expires_at_ms: 0, challenge: "forge-anywhere://pair?challenge=abc_-1" };
const approved = {
  version: 1 as const, status: "approved" as const, account_id: ACCOUNT, device_id: "d".repeat(32),
  access_token: "at", refresh_token: "rt", access_expires_at_ms: 9, epoch: 3,
  device_wrap_envelope: "env", signing_public_key: "spk", exchange_public_key: "epk",
};

function deps(overrides: Partial<CliLinkDeps> = {}) {
  const invoke = vi.fn(async (command: string) => command === "forge_anywhere_cli_account"
    ? { account_id: ACCOUNT, github_login: "floris" }
    : undefined);
  return {
    invoke: invoke as unknown as CliLinkDeps["invoke"],
    createPairing: vi.fn(async () => created),
    pollPairing: vi.fn(async () => approved),
    cancelPairing: vi.fn(async () => undefined),
    openApprovedPairing: vi.fn(() => ({ accountDataKey: new Uint8Array(32).fill(9), epoch: 3 })),
    generateKeys: () => keys,
    sleep: async () => undefined,
    ...overrides,
  };
}

describe("linkWithLocalCli", () => {
  it("does nothing when the CLI is not signed in", async () => {
    const d = deps({ invoke: (async () => null) as unknown as CliLinkDeps["invoke"] });
    await expect(linkWithLocalCli("https://svc", "Forge Desktop", d)).resolves.toBeNull();
    expect(d.createPairing).not.toHaveBeenCalled();
  });

  it("leaves an expired sign-in of another account alone", async () => {
    const d = deps();
    await expect(linkWithLocalCli("https://svc", "Forge Desktop", d, "f".repeat(32))).resolves.toBeNull();
    expect(d.createPairing).not.toHaveBeenCalled();
  });

  it("asks the CLI to approve this app's own key and installs the approved account", async () => {
    const d = deps();
    const linked = await linkWithLocalCli("https://svc", "Forge Desktop", d);
    expect(d.invoke).toHaveBeenCalledWith("approve_anywhere_pairing_with_cli", {
      challenge: "abc_-1",
      signingPublicKey: "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI",
    });
    expect(linked?.auth).toMatchObject({ account_id: ACCOUNT, github_login: "floris", refresh_token: "rt", new_account: false });
    expect(linked?.epoch).toBe(3);
  });

  it("refuses an approval from another account and cancels the request", async () => {
    const d = deps({ pollPairing: vi.fn(async () => ({ ...approved, account_id: "f".repeat(32) })) });
    await expect(linkWithLocalCli("https://svc", "Forge Desktop", d)).rejects.toThrow("different Forge Anywhere account");
    expect(d.openApprovedPairing).not.toHaveBeenCalled();
    expect(d.cancelPairing).toHaveBeenCalledWith("https://svc", "pid", "ptok");
  });

  it("cancels the request when the CLI cannot approve", async () => {
    const invoke = vi.fn(async (command: string) => {
      if (command === "forge_anywhere_cli_account") return { account_id: ACCOUNT, github_login: null };
      throw new Error("unexpected argument '--expect-signing-key'");
    });
    const d = deps({ invoke: invoke as unknown as CliLinkDeps["invoke"] });
    await expect(linkWithLocalCli("https://svc", "Forge Desktop", d)).rejects.toThrow("--expect-signing-key");
    expect(d.pollPairing).not.toHaveBeenCalled();
    expect(d.cancelPairing).toHaveBeenCalled();
  });
});
