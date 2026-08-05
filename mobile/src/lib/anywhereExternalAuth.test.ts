import { describe, expect, it, vi } from "vitest";

import {
  openBrowserAuthUrl,
  openExternalAuthUrl,
  reserveBrowserAuthWindow,
  runReservedBrowserFlow,
} from "./anywhereExternalAuth";

const mocks = vi.hoisted(() => ({ isTauri: false, openUrl: vi.fn(async () => {}) }));

// `./platform` pulls in react-native, which vitest cannot parse — mock it the way updater.test.ts
// and browserPreview.test.ts already do. The getter lets one file cover both shells.
vi.mock("./platform", () => ({
  get isTauri() {
    return mocks.isTauri;
  },
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));

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

  it("opens through the Tauri opener when the shell has no popups", async () => {
    // WebKitGTK returns null from window.open, so the desktop shell must not depend on a reserved
    // tab: "Sign in with GitHub" started the device flow and then opened nothing at all.
    mocks.isTauri = true;
    mocks.openUrl.mockClear();
    const openWindow = vi.fn(() => null);

    await openExternalAuthUrl("https://github.com/login/device", openWindow);

    expect(mocks.openUrl).toHaveBeenCalledWith("https://github.com/login/device");
    expect(openWindow).not.toHaveBeenCalled();
    mocks.isTauri = false;
  });

  it("still uses window.open on the plain web build", async () => {
    mocks.isTauri = false;
    mocks.openUrl.mockClear();
    const openWindow = vi.fn(() => null);

    await openExternalAuthUrl("https://github.com/login/device", openWindow);

    expect(mocks.openUrl).not.toHaveBeenCalled();
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

});
