// Scrolls the transcript to a message the palette or Find asked for. History pages in from the
// newest end, so a hit deep in an old session is not loaded yet: ask the host for the window
// around it (one request, one swap) and, for a host that cannot, keep pulling older pages until
// the row appears. Then scroll it into the middle of the view.
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
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
  jumpAround?: (seq: number) => Promise<boolean>,
) {
  const target = useSyncExternalStore(
    subscribeMessageJump,
    () => (sessionId ? peekMessageJump(sessionId) : null),
    () => null,
  );
  const pagesPulled = useRef(0);
  const around = useRef<{ seq: number; state: "pending" | "failed" | "done" } | null>(null);
  const [, settle] = useState(0);
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
    if (target == null) {
      pagesPulled.current = 0;
      around.current = null;
    }
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
    if (jumpAround && around.current?.seq !== target) {
      around.current = { seq: target, state: "pending" };
      void jumpAround(target).then((ok) => {
        if (around.current?.seq !== target) return;
        around.current = { seq: target, state: ok ? "done" : "failed" };
        settle((n) => n + 1);
      });
      return;
    }
    if (jumpAround && around.current?.state === "pending") return;
    if (jumpAround && around.current?.state === "done") {
      clearMessageJump(sessionId);
      return;
    }
    if (pager.hasNextPage && pagesPulled.current < MAX_JUMP_PAGES) {
      pagesPulled.current += 1;
      void pager.fetchNextPage();
      return;
    }
    if (items.length > 0) clearMessageJump(sessionId);
  }, [sessionId, target, items, pager, scrollToTarget, jumpAround]);

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
