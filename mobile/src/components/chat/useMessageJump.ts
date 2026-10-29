// Scrolls the transcript to a message the palette asked for. History pages in from the newest
// end, so a hit deep in an old session is not loaded yet: keep pulling older pages until the row
// appears, then scroll it into the middle of the view.
import { useCallback, useEffect, useRef, useSyncExternalStore } from "react";
import type { FlatList } from "react-native";

import {
  MAX_JUMP_PAGES,
  clearMessageJump,
  findJumpIndex,
  peekMessageJump,
  subscribeMessageJump,
} from "../../lib/messageJump";

interface Pager {
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  fetchNextPage: () => unknown;
}

// Rows of a freshly loaded page measure over the next few frames, and every size change snaps the
// list back toward the newest row until a scroll event records the new position. So the scroll is
// asserted again a few times while the layout settles.
const SCROLL_PASSES_MS = [0, 120, 350, 800];

export function useMessageJump<T extends { id: string }>(
  sessionId: string | null,
  items: readonly T[],
  listRef: React.RefObject<FlatList<T> | null>,
  pager: Pager,
) {
  const target = useSyncExternalStore(
    subscribeMessageJump,
    () => (sessionId ? peekMessageJump(sessionId) : null),
    () => null,
  );
  const pagesPulled = useRef(0);
  const scrollIndex = useRef(-1);
  const timers = useRef<ReturnType<typeof setTimeout>[]>([]);

  const scrollToTarget = useCallback(() => {
    if (scrollIndex.current < 0) return;
    try {
      listRef.current?.scrollToIndex({ index: scrollIndex.current, animated: false, viewPosition: 0.5 });
    } catch {
      // Not measured yet: onScrollToIndexFailed lands near it and a later pass finishes the job.
    }
  }, [listRef]);

  useEffect(() => {
    if (target == null) pagesPulled.current = 0;
  }, [target]);

  useEffect(() => {
    if (!sessionId || target == null) return;
    const index = findJumpIndex(items, target);
    if (index >= 0) {
      clearMessageJump(sessionId);
      scrollIndex.current = index;
      timers.current.forEach(clearTimeout);
      timers.current = SCROLL_PASSES_MS.map((ms) => setTimeout(scrollToTarget, ms));
      return;
    }
    if (pager.isFetchingNextPage) return;
    if (pager.hasNextPage && pagesPulled.current < MAX_JUMP_PAGES) {
      pagesPulled.current += 1;
      void pager.fetchNextPage();
      return;
    }
    if (items.length > 0) clearMessageJump(sessionId);
  }, [sessionId, target, items, pager, scrollToTarget]);

  useEffect(
    () => () => {
      timers.current.forEach(clearTimeout);
    },
    [],
  );

  return useCallback(
    (info: { averageItemLength: number }) => {
      listRef.current?.scrollToOffset({ offset: info.averageItemLength * scrollIndex.current, animated: false });
    },
    [listRef],
  );
}
