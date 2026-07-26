// Which tab a horizontal swipe lands on. Kept apart from the gesture itself so the decision is
// testable without a device — the gesture wiring in components/TabSwipe.tsx is not.
//
// Order mirrors `(tabs)/_layout.tsx`'s trigger order in BOTH navigators, which is the order the tab
// bar renders left to right. `src/lib/tabRoutes.test.ts` already guards that the tab set matches the
// directory; this guards that a swipe agrees with what the user can see.

/** Left-to-right, matching the tab bar. `/` is the Fleet index route. */
export const TAB_SWIPE_ORDER = ["/", "/inbox", "/history", "/settings"] as const;

export type TabHref = (typeof TAB_SWIPE_ORDER)[number];

/** Direction the CONTENT moves, i.e. the direction the finger travelled. */
export type SwipeDirection = "left" | "right";

/**
 * The tab a swipe should land on, or `null` when it should do nothing — an unrecognised path (a
 * pushed route is not a tab) or the end of the row.
 *
 * Deliberately does NOT wrap around. Both platforms' paged tab UIs stop at the ends, and wrapping
 * makes the last tab feel like it silently teleports; the dead swipe at the edge is the feedback
 * that there is nothing further.
 */
export function nextTabHref(pathname: string, direction: SwipeDirection): TabHref | null {
  const current = normalizeTabPath(pathname);
  if (current === null) return null;
  const index = TAB_SWIPE_ORDER.indexOf(current);
  // Swiping the content leftward reveals what is to its right, which is the NEXT tab.
  const target = direction === "left" ? index + 1 : index - 1;
  if (target < 0 || target >= TAB_SWIPE_ORDER.length) return null;
  return TAB_SWIPE_ORDER[target]!;
}

/**
 * `usePathname()` gives the tab index route as `/` on native but can carry a trailing slash or an
 * empty string depending on the platform and how the route was reached, so match on a normalised
 * form rather than trusting an exact string.
 */
function normalizeTabPath(pathname: string): TabHref | null {
  const trimmed = pathname.replace(/\/+$/, "");
  if (trimmed === "") return "/";
  return (TAB_SWIPE_ORDER as readonly string[]).includes(trimmed) ? (trimmed as TabHref) : null;
}
