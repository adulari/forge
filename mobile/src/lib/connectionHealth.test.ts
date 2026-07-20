import { describe, expect, it } from "vitest";

import { connectionHealthFromFleet } from "./connectionHealth";

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
