// The fs-reading test follows the convention documented in tabRoutes.test.ts: the file-local
// reference directive avoids leaking Node globals into every React Native module.
/// <reference types="node" />
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { landedTabHref, pagerGeometry, tabHrefAt, TAB_SWIPE_ORDER } from "./tabSwipe";

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

describe("pagerGeometry", () => {
  const width = 390;

  it("gives the end tabs one neighbour and the middle ones two", () => {
    expect(pagerGeometry(0, width)).toEqual({ pages: 2, homePage: 0, homeOffset: 0, contentWidth: 780 });
    expect(pagerGeometry(1, width)).toEqual({ pages: 3, homePage: 1, homeOffset: 390, contentWidth: 1170 });
    expect(pagerGeometry(TAB_SWIPE_ORDER.length - 1, width)).toEqual({
      pages: 2, homePage: 1, homeOffset: 390, contentWidth: 780,
    });
  });

  // THE FLASH. A UIScrollView clamps contentOffset into contentSize, so a content width measured
  // short — which is what happens when it is left to the children's layout rather than pinned — drags
  // the offset to zero, and page zero holds the tab to the LEFT: the one just swiped away from. The
  // resting page must fit inside the pinned width for every tab, or the clamp is back.
  it("always pins a width the resting page fits inside", () => {
    for (const position of TAB_SWIPE_ORDER.keys()) {
      const { homeOffset, contentWidth } = pagerGeometry(position, width);
      expect(homeOffset + width).toBeLessThanOrEqual(contentWidth);
    }
  });

  it("survives the first render before a width is known", () => {
    expect(pagerGeometry(2, 0)).toMatchObject({ homeOffset: 0, contentWidth: 0 });
  });
});

describe("landedTabHref", () => {
  it("stays put when the settle came back to the resting page", () => {
    expect(landedTabHref(1, 1, 1)).toBeNull();
    expect(landedTabHref(0, 0, 0)).toBeNull();
  });

  it("maps a page either side to the neighbouring tab", () => {
    expect(landedTabHref(1, 0, 1)).toBe("/");
    expect(landedTabHref(1, 2, 1)).toBe("/history");
    expect(landedTabHref(0, 1, 0)).toBe("/inbox");
  });

  it("refuses to page past either end", () => {
    // Rubber-banding at the first and last tab can only ever settle back home, but the arithmetic
    // must not invent a tab if it is ever asked to.
    expect(landedTabHref(0, -1, 0)).toBeNull();
    expect(landedTabHref(TAB_SWIPE_ORDER.length - 1, 2, 1)).toBeNull();
  });
});
