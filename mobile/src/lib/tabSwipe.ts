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
