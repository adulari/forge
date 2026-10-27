import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  applyDesktopOta,
  describeDesktopOta,
  dismissDesktopOtaBanner,
  getDesktopOtaStatus,
  markDesktopOtaHealthy,
  pendingBundleToOffer,
  refreshDesktopOtaStatus,
  type DesktopOtaStatus,
} from "./desktopOta";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));

vi.mock("./platform", () => ({ isTauri: true }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

function status(overrides: Partial<DesktopOtaStatus> = {}): DesktopOtaStatus {
  return {
    enabled: true,
    disabledReason: null,
    runtimeVersion: "0123456789abcdef",
    appVersion: "2.23.6",
    embeddedBundleId: "1234567890abcdef",
    embeddedCreatedAt: 1_700_000_000,
    active: null,
    pending: null,
    lastCheck: 1_700_000_100,
    lastError: null,
    lastRollback: null,
    booting: false,
    ...overrides,
  };
}

describe("desktop OTA client", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.listen.mockReset();
  });

  it("tells the shell the UI is healthy and then refreshes what it reports", async () => {
    mocks.invoke.mockImplementation(async (command: string) =>
      command === "ota_status" ? status({ booting: false }) : undefined,
    );
    await markDesktopOtaHealthy();
    expect(mocks.invoke.mock.calls.map(([command]) => command)).toEqual([
      "ota_mark_healthy",
      "ota_status",
    ]);
    expect(getDesktopOtaStatus()?.booting).toBe(false);
  });

  it("hides the rows when the shell predates the OTA commands", async () => {
    mocks.invoke.mockRejectedValue(new Error("command ota_status not found"));
    await refreshDesktopOtaStatus();
    expect(getDesktopOtaStatus()).toBeNull();
  });

  it("asks the shell to switch bundles", async () => {
    mocks.invoke.mockResolvedValue(true);
    await expect(applyDesktopOta()).resolves.toBe(true);
    expect(mocks.invoke).toHaveBeenCalledWith("ota_apply_pending");
  });

  it("offers a pending bundle once, until the user dismisses that bundle", async () => {
    const pending = { id: "abcdef0123456", createdAt: 1_700_000_500 };
    mocks.invoke.mockResolvedValue(status({ pending }));
    await refreshDesktopOtaStatus();
    expect(pendingBundleToOffer(getDesktopOtaStatus())).toEqual(pending);
    dismissDesktopOtaBanner();
    expect(pendingBundleToOffer(getDesktopOtaStatus())).toBeNull();
    expect(pendingBundleToOffer(status({ pending: { id: "fedcba9876543", createdAt: 1_700_000_900 } }))).not.toBeNull();
    expect(pendingBundleToOffer(status({ enabled: false, pending }))).toBeNull();
  });

  it("describes the live interface and what is waiting", () => {
    expect(describeDesktopOta(status()).badge.label).toBe("Up to date");
    expect(describeDesktopOta(status()).title).toContain("Built-in 1234567");
    const active = describeDesktopOta(status({ active: { id: "aaaaaaa1111", createdAt: 1_700_000_200 } }));
    expect(active.title).toContain("Bundle aaaaaaa");
    const ready = describeDesktopOta(status({ pending: { id: "bbbbbbb2222", createdAt: 1_700_000_300 } }));
    expect(ready.badge).toEqual({ label: "Ready", tone: "accent" });
    expect(ready.subtitle).toContain("bbbbbbb");
    expect(describeDesktopOta(status({ lastError: "offline" })).badge.tone).toBe("warn");
    expect(describeDesktopOta(status({ lastRollback: "ccccccc3333" })).subtitle).toContain("Rolled back ccccccc");
    const off = describeDesktopOta(status({ enabled: false, disabledReason: "FORGE_DESKTOP_OTA=0" }));
    expect(off.badge.label).toBe("Off");
    expect(off.subtitle).toContain("FORGE_DESKTOP_OTA=0");
  });
});
