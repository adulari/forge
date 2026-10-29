import { describe, expect, it } from "vitest";

import { routeForDeepLink } from "./deepLinkCore";

describe("routeForDeepLink", () => {
  it("maps the supported destinations", () => {
    expect(routeForDeepLink("forge://session/abc-123")).toBe("/session/abc-123");
    expect(routeForDeepLink("forge://new")).toBe("/new-session");
    expect(routeForDeepLink("forge://inbox/")).toBe("/inbox");
    expect(routeForDeepLink("forge://fleet")).toBe("/");
    expect(routeForDeepLink("forge://session/abc?x=1#y")).toBe("/session/abc");
  });

  it("rejects other schemes, unknown hosts, traversal and extra segments", () => {
    expect(routeForDeepLink("https://example.com/session/abc")).toBeNull();
    expect(routeForDeepLink("forge://settings")).toBeNull();
    expect(routeForDeepLink("forge://session/../../etc")).toBeNull();
    expect(routeForDeepLink("forge://session/a/b")).toBeNull();
    expect(routeForDeepLink("forge://session")).toBeNull();
    expect(routeForDeepLink("forge://new/extra")).toBeNull();
  });
});
