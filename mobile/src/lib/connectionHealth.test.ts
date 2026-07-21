import { describe, expect, it } from "vitest";

import { connectionHealthFromFleet, desktopFleetStatusFromFleet } from "./connectionHealth";

describe("Settings connection health", () => {
  it("uses the active server fleet result as the connection source of truth", () => {
    expect(connectionHealthFromFleet({ isSuccess: true, isLoading: false, error: null })).toBe("ok");
  });

  it("maps the active fleet failure without retaining an earlier probe result", () => {
    expect(connectionHealthFromFleet({ isSuccess: false, isLoading: false, error: { status: 0 } })).toBe("unreachable");
    expect(connectionHealthFromFleet({ isSuccess: false, isLoading: false, error: { status: 404 } })).toBe("bad-token");
    expect(connectionHealthFromFleet({ isSuccess: false, isLoading: false, error: { status: 503 } })).toBe("server-error");
  });
});

describe("Desktop Fleet status", () => {
  it.each([
    [{ isSuccess: true, isLoading: false, error: null }, { state: "online", label: "online" }],
    [{ isSuccess: false, isLoading: true, error: null }, { state: "loading", label: "connecting" }],
    [{ isSuccess: false, isLoading: false, error: { status: 0 } }, { state: "offline", label: "offline" }],
    [{ isSuccess: false, isLoading: false, error: { status: 503 } }, { state: "error", label: "service unavailable" }],
  ] as const)("maps query state instead of assuming success", (query, expected) => {
    expect(desktopFleetStatusFromFleet(query)).toEqual(expected);
  });
});
