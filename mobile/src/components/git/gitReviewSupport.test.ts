import { describe, expect, it, vi } from "vitest";

import { isGitReviewReadOnly, isGitReviewSupported } from "./gitReviewSupport";

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

  // Reviewing now works over the relay too: git_status/git_branches/git_diff are bridged routes.
  it("is supported over a Forge Anywhere relay host", () => {
    expect(isGitReviewSupported("fany://host-1")).toBe(true);
    expect(isGitReviewSupported("fany-ws://host-1")).toBe(true);
  });
});

describe("isGitReviewReadOnly", () => {
  // The host refuses staging, committing and branch switches over the bridge, so the dock must
  // drop those controls rather than render presses that come back denied.
  it("is read-only over a Forge Anywhere relay host", () => {
    expect(isGitReviewReadOnly("fany://host-1")).toBe(true);
    expect(isGitReviewReadOnly("fany-ws://host-1")).toBe(true);
  });

  it("is writable over a direct daemon connection or with no active server", () => {
    expect(isGitReviewReadOnly(null)).toBe(false);
    expect(isGitReviewReadOnly("http://192.168.1.5:4823")).toBe(false);
    expect(isGitReviewReadOnly("https://tunnel.example.com")).toBe(false);
  });
});
