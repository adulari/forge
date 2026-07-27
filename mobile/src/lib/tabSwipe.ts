// The tab row, as the swipe pager sees it.
//
// Order mirrors `(tabs)/_layout.tsx`'s trigger order in BOTH navigators, which is the order the tab
// bar renders left to right. `tabRoutes.test.ts` guards that the tab SET matches the directory;
// `tabSwipe.test.ts` guards this ORDER, which only the pager depends on — a swipe that landed on a
// tab other than the visible neighbour would be worse than no swipe at all.
//
// The pager works in indices rather than paths (it knows which tab is rendering it), so there is no
// path parsing here and no "is this even a tab" question: /floor, /plans and /session/[id] live in
// the ROOT stack, never mount a pager, and therefore cannot swipe between tabs by construction.

/** Left-to-right, matching the tab bar. `/` is the Fleet index route. */
export const TAB_SWIPE_ORDER = ["/", "/inbox", "/history", "/settings"] as const;

export type TabHref = (typeof TAB_SWIPE_ORDER)[number];

/** `null` past either end, so the pager's edges need no special case of their own. */
export function tabHrefAt(index: number): TabHref | null {
  return TAB_SWIPE_ORDER[index] ?? null;
}

export interface PagerGeometry {
  /** How many page slots this tab's pager holds: itself, plus a neighbour on each side it has. */
  pages: number;
  /** Which of those slots holds the tab's own screen. */
  homePage: number;
  /** Scroll offset at rest. */
  homeOffset: number;
  /**
   * The content width to PIN on the scroll view, rather than letting it be measured from the pages.
   *
   * A UIScrollView clamps contentOffset into contentSize, so a layout pass that measures the content
   * as narrower than it really is drags the offset to zero — and page zero is the tab to the LEFT.
   * Arriving on a tab you swiped rightwards to would then draw the tab you just left for a frame.
   * Pinning it means no pass can measure it short, so there is no clamp and nothing to recover from.
   */
  contentWidth: number;
}

export function pagerGeometry(index: number, width: number): PagerGeometry {
  const hasPrev = index > 0;
  const pages = 1 + (hasPrev ? 1 : 0) + (index < TAB_SWIPE_ORDER.length - 1 ? 1 : 0);
  const homePage = hasPrev ? 1 : 0;
  return { pages, homePage, homeOffset: homePage * width, contentWidth: pages * width };
}

/**
 * Which tab a settle on `landed` means, or `null` for "stay here".
 *
 * Separate from the pager so the arithmetic that turns a page into a tab is checked without a
 * device: getting it wrong lands you on a tab that was never the one sliding into view.
 */
export function landedTabHref(index: number, landed: number, homePage: number): TabHref | null {
  if (landed === homePage) return null;
  return tabHrefAt(index + (landed - homePage));
}
