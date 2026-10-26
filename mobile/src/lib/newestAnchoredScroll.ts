// The chat lists newest-first and, on native, renders an `inverted` FlatList so offset 0 is the
// newest row. react-native-web inverts with `scaleY(-1)` on the scroll container, which flips the
// scrollbar thumb, wheel and trackpad direction. On web the list is therefore rendered upright
// (oldest at the top, newest at the bottom) and these helpers keep the inverted list's contract:
// "offset" is the distance from the NEWEST end, the view stays pinned to the newest row while
// it grows, and an older page landing at the top does not move what the reader is looking at.

export interface ScrollMetrics {
  scrollTop: number;
  contentHeight: number;
  layoutHeight: number;
}

/** Distance from the newest (bottom) end: what an inverted list reports as `contentOffset.y`. */
export function offsetFromNewest({ scrollTop, contentHeight, layoutHeight }: ScrollMetrics): number {
  return Math.max(0, contentHeight - layoutHeight - scrollTop);
}

/** The `scrollTop` that puts the view `fromNewest` px above the newest end. */
export function scrollTopFor(fromNewest: number, contentHeight: number, layoutHeight: number): number {
  return Math.max(0, contentHeight - layoutHeight - Math.max(0, fromNewest));
}

/** Within this distance of the newest end the reader counts as following the conversation. */
export const PINNED_PX = 24;

export interface TimelineShape {
  length: number;
  newestKey: string | null;
  oldestKey: string | null;
}

export function timelineShape(keys: readonly string[]): TimelineShape {
  return { length: keys.length, newestKey: keys[0] ?? null, oldestKey: keys[keys.length - 1] ?? null };
}

/** Whether a data change grew the list at its OLDEST end (an older history page landed): the
 * newest row is unchanged but the oldest row or the length is not. A new message, or a streaming
 * reply growing in place, changes the newest key or nothing at all. */
export function grewAtOldestEnd(prev: TimelineShape | null, next: TimelineShape): boolean {
  if (!prev || prev.newestKey === null || prev.newestKey !== next.newestKey) return false;
  return prev.oldestKey !== next.oldestKey || prev.length !== next.length;
}

/** The `scrollTop` to set after the content height changed, or null to leave the view alone.
 * Pinned readers stay on the newest row; an older page at the top keeps the visible rows still;
 * anything else (a group expanded mid-list, a reply growing below a reader who scrolled up) is
 * the browser's own business. */
export function scrollTopAfterResize(args: {
  fromNewest: number;
  contentHeight: number;
  layoutHeight: number;
  topGrowth: boolean;
}): number | null {
  const { fromNewest, contentHeight, layoutHeight, topGrowth } = args;
  if (fromNewest <= PINNED_PX) return scrollTopFor(0, contentHeight, layoutHeight);
  if (topGrowth) return scrollTopFor(fromNewest, contentHeight, layoutHeight);
  return null;
}
