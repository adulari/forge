// Keeps the chat's loaded history taller than the screen.
//
// A tools history page is 15 rows (lib/historyPaging.ts, sized for the Anywhere relay), and a
// tool-heavy turn folds a page's worth of call/result rows into one "Ran 7 scripts" line. The
// first page then filled a fifth of the screen: the inverted list sat its few rows at the
// bottom over a large empty band, and there was nothing to scroll. FlatList's `onEndReached`
// does not rescue it — it fires once per content length, and a page that only grows the oldest
// folded group leaves the length unchanged, so paging stopped for good. This fetches older
// pages until the content is a few screens tall, and again whenever a scroll comes near the
// oldest loaded row.
import { useCallback, useEffect, useRef, useState } from "react";
import type { LayoutChangeEvent, NativeScrollEvent } from "react-native";

/** Screens of content to keep loaded beyond the viewport before stopping. */
const FILL_SCREENS = 2;

export interface HistoryPager {
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  isError: boolean;
  fetchNextPage: () => unknown;
}

export function useHistoryAutofill(pager: HistoryPager, enabled: boolean, loadedRows: number) {
  const [viewport, setViewport] = useState(0);
  const [content, setContent] = useState(0);
  const pagerRef = useRef(pager);
  useEffect(() => {
    pagerRef.current = pager;
  });

  const fetchOlder = useCallback(() => {
    const p = pagerRef.current;
    if (p.hasNextPage && !p.isFetchingNextPage && !p.isError) void p.fetchNextPage();
  }, []);

  useEffect(() => {
    if (!enabled || viewport === 0) return;
    if (content < viewport * FILL_SCREENS) fetchOlder();
    // `loadedRows` re-checks after every page, including one that left the height unchanged.
  }, [enabled, viewport, content, loadedRows, pager.isFetchingNextPage, pager.hasNextPage, fetchOlder]);

  const onLayout = useCallback((e: LayoutChangeEvent) => setViewport(e.nativeEvent.layout.height), []);
  const onContentSizeChange = useCallback((_w: number, h: number) => setContent(h), []);
  /** Call from the list's onScroll: near the oldest row (the far end of the inverted list). */
  const onScrollNearEnd = useCallback(
    (e: NativeScrollEvent) => {
      const remaining = e.contentSize.height - e.layoutMeasurement.height - e.contentOffset.y;
      if (enabled && remaining < e.layoutMeasurement.height) fetchOlder();
    },
    [enabled, fetchOlder],
  );
  return { onLayout, onContentSizeChange, onScrollNearEnd };
}
