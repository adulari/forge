import { describe, expect, it, vi } from "vitest";

import {
  openBrowserAuthUrl,
  reserveBrowserAuthWindow,
  resumePasskeyBrowserAfterPayload,
  runReservedBrowserFlow,
} from "./anywhereExternalAuth";

describe("Anywhere browser authentication", () => {
  it("keeps Forge loaded while GitHub authorization runs in a reserved tab", () => {
    const replace = vi.fn();
    const close = vi.fn();
    const popup = { closed: false, close, location: { replace }, opener: {} };
    const openWindow = vi.fn(() => popup);

    const reserved = reserveBrowserAuthWindow(openWindow);
    expect(openWindow).toHaveBeenCalledWith("about:blank", "_blank");
    expect(popup.opener).toBeNull();

    reserved?.navigate("https://github.com/login/device");
    expect(replace).toHaveBeenCalledWith("https://github.com/login/device");

    reserved?.close();
    expect(close).toHaveBeenCalledOnce();
  });

  it("falls back to the visible login link when the popup is blocked", () => {
    expect(reserveBrowserAuthWindow(() => null)).toBeNull();
  });

  it("opens a retry without navigating the Forge tab", () => {
    const openWindow = vi.fn(() => null);
    openBrowserAuthUrl("https://github.com/login/device", openWindow);
    expect(openWindow).toHaveBeenCalledWith(
      "https://github.com/login/device",
      "_blank",
      "noopener,noreferrer",
    );
  });

  it("closes a reserved passkey tab when setup fails before navigation", async () => {
    const reserved = { navigate: vi.fn(), close: vi.fn() };
    const failure = new Error("secure session expired");

    await expect(runReservedBrowserFlow(reserved, async () => { throw failure; }))
      .rejects.toBe(failure);

    expect(reserved.close).toHaveBeenCalledOnce();
    expect(reserved.navigate).not.toHaveBeenCalled();
  });

  it("foregrounds the system browser after native sends the passkey payload", async () => {
    const openUrl = vi.fn(async () => undefined);

    await resumePasskeyBrowserAfterPayload("ios", "https://app.test/passkey", openUrl);
    await resumePasskeyBrowserAfterPayload("web", "https://app.test/passkey", openUrl);

    expect(openUrl).toHaveBeenCalledOnce();
    expect(openUrl).toHaveBeenCalledWith("https://app.test/passkey");
  });
});
