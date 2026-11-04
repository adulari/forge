import { describe, expect, it } from "vitest";

import { isIdeRoute } from "./ideRoutes";

describe("isIdeRoute", () => {
  it("covers home and sessions, not full pages", () => {
    expect(isIdeRoute("/")).toBe(true);
    expect(isIdeRoute("/session/abc")).toBe(true);
    expect(isIdeRoute("/session/abc/review")).toBe(true);
    expect(isIdeRoute("/settings")).toBe(false);
    expect(isIdeRoute("/new-session")).toBe(false);
    expect(isIdeRoute("/inbox")).toBe(false);
    expect(isIdeRoute("/sessions-list")).toBe(false);
  });
});
