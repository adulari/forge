// The fs-reading test at the bottom follows the convention documented in tabRoutes.test.ts: the
// file-local reference directive avoids leaking Node globals into every React Native module.
/// <reference types="node" />
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { nextTabHref, TAB_SWIPE_ORDER } from "./tabSwipe";

describe("nextTabHref", () => {
  it("moves toward the tab the swipe uncovers", () => {
    expect(nextTabHref("/", "left")).toBe("/inbox");
    expect(nextTabHref("/inbox", "left")).toBe("/history");
    expect(nextTabHref("/history", "left")).toBe("/settings");
    expect(nextTabHref("/settings", "right")).toBe("/history");
    expect(nextTabHref("/history", "right")).toBe("/inbox");
    expect(nextTabHref("/inbox", "right")).toBe("/");
  });

  it("stops at both ends instead of wrapping", () => {
    expect(nextTabHref("/", "right")).toBeNull();
    expect(nextTabHref("/settings", "left")).toBeNull();
  });

  it("ignores routes that are not tabs, so a pushed screen never switches tabs", () => {
    // Floor and Plans deliberately live in the ROOT stack, not `(tabs)` — see the invariant comment
    // in (tabs)/_layout.tsx. A swipe on one of them must do nothing at all.
    for (const path of ["/floor", "/plans", "/session/abc", "/anywhere", "/anywhere/notifications"]) {
      expect(nextTabHref(path, "left")).toBeNull();
      expect(nextTabHref(path, "right")).toBeNull();
    }
  });

  it("treats an empty or trailing-slash path as the index tab", () => {
    expect(nextTabHref("", "left")).toBe("/inbox");
    expect(nextTabHref("/inbox/", "left")).toBe("/history");
  });

  // A swipe order that disagrees with the tab bar is worse than no swipe: the gesture would move
  // to a tab other than the neighbour the user can see. tabRoutes.test.ts guards the tab SET;
  // this guards the ORDER, which only this feature depends on.
  it("matches the tab bar's own order in both navigators", () => {
    const layout = readFileSync(
      join(dirname(fileURLToPath(import.meta.url)), "..", "app", "(tabs)", "_layout.tsx"),
      "utf8",
    );
    const inOrder = (component: string) =>
      [...layout.matchAll(new RegExp(`<${component}\\s+name="([^"]+)"`, "g"))].map((m) => m[1]);
    const expected = TAB_SWIPE_ORDER.map((href) => (href === "/" ? "index" : href.slice(1)));

    expect(inOrder("NativeTabs\\.Trigger")).toEqual(expected);
    expect(inOrder("Tabs\\.Screen")).toEqual(expected);
  });

  it("covers every tab in the order, so a new tab cannot be silently unreachable", () => {
    for (const [index, href] of TAB_SWIPE_ORDER.entries()) {
      const left = nextTabHref(href, "left");
      const right = nextTabHref(href, "right");
      expect(left).toBe(index === TAB_SWIPE_ORDER.length - 1 ? null : TAB_SWIPE_ORDER[index + 1]);
      expect(right).toBe(index === 0 ? null : TAB_SWIPE_ORDER[index - 1]);
    }
  });
});
