// The fs-reading test follows the convention documented in tabRoutes.test.ts: the file-local
// reference directive avoids leaking Node globals into every React Native module.
/// <reference types="node" />
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { tabHrefAt, TAB_SWIPE_ORDER } from "./tabSwipe";

describe("tab swipe order", () => {
  // A swipe order that disagrees with the tab bar is worse than no swipe: the gesture would page to
  // a tab other than the neighbour the user can see peeking in. tabRoutes.test.ts guards the tab
  // SET; this guards the ORDER.
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

  // Each route passes its own index to TabPager, so a wrapper wired to the wrong number would page
  // to the wrong tab while still looking correct in the bar.
  it("agrees with the index each tab route hands its pager", () => {
    const tabsDir = join(dirname(fileURLToPath(import.meta.url)), "..", "app", "(tabs)");
    const fileFor: Record<string, string> = {
      "/": "index.tsx",
      "/inbox": "inbox.tsx",
      "/history": "history.tsx",
      "/settings": "settings.tsx",
    };

    for (const [position, href] of TAB_SWIPE_ORDER.entries()) {
      const source = readFileSync(join(tabsDir, fileFor[href]!), "utf8");
      expect(source, `${fileFor[href]!} must render a TabPager`).toMatch(/<TabPager\s+index=\{\d\}>/);
      expect(source, `${fileFor[href]!} must pass index ${position}`).toContain(
        `<TabPager index={${position}}>`,
      );
    }
  });

  it("resolves an href per position and nothing past the ends", () => {
    for (const [position, href] of TAB_SWIPE_ORDER.entries()) {
      expect(tabHrefAt(position)).toBe(href);
    }
    expect(tabHrefAt(-1)).toBeNull();
    expect(tabHrefAt(TAB_SWIPE_ORDER.length)).toBeNull();
  });
});
