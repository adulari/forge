import { describe, expect, it, vi } from "vitest";

import { isGitReviewSupported } from "./gitReviewSupport";

// `lib/transport/index.ts` imports `../platform` (which imports `react-native` for `Platform.OS`)
// and re-exports `anywhereCredentialStore`, which imports `expo-secure-store` — both mocked the
// same way `lib/transport/tauriWebSocket.test.ts` does, so this stays a plain unit test instead
// of pulling in a native runtime.
vi.mock("../../lib/platform", () => ({
  isTauri: false,
  isWeb: true,
  isNative: false,
}));
vi.mock("expo-secure-store", () => ({}));

describe("isGitReviewSupported", () => {
  it("is supported with no active server", () => {
    expect(isGitReviewSupported(null)).toBe(true);
  });

  it("is supported over a direct daemon connection", () => {
    expect(isGitReviewSupported("http://192.168.1.5:4823")).toBe(true);
    expect(isGitReviewSupported("https://tunnel.example.com")).toBe(true);
  });

  it("is unsupported over a Forge Anywhere relay host", () => {
    expect(isGitReviewSupported("fany://host-1")).toBe(false);
    expect(isGitReviewSupported("fany-ws://host-1")).toBe(false);
  });
});
