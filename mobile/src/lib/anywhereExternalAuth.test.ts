import { describe, expect, it, vi } from "vitest";

import { openBrowserAuthUrl, reserveBrowserAuthWindow } from "./anywhereExternalAuth";

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
});
