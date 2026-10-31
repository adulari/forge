import { describe, expect, it } from "vitest";

import { isTopBarTabActive } from "./topBarActive";

const TABS = ["/", "/inbox", "/floor", "/history", "/settings"];
const active = (pathname: string) => TABS.filter((href) => isTopBarTabActive(href, pathname));

describe("isTopBarTabActive", () => {
  it("underlines exactly one tab on /session-tree", () => {
    expect(active("/session-tree")).toEqual(["/settings"]);
  });
  it("keeps Fleet for the fleet and session routes", () => {
    expect(active("/")).toEqual(["/"]);
    expect(active("/session/abc")).toEqual(["/"]);
    expect(active("/session")).toEqual(["/"]);
  });
  it("keeps settings sub-pages on Settings", () => {
    expect(active("/appearance")).toEqual(["/settings"]);
  });
});
