// Which timeline row sits at the top of the view, for the sticky prompt bar. Row heights are
// measured as they lay out and summed from the newest end, so the answer does not depend on the
// list's own viewability, whose offsets go stale on web when an earlier row resizes.
import { useCallback, useEffect, useRef, useState } from "react";

import { rowAtReach } from "../../lib/stickyPrompt";

export function useTopRow(items: readonly { id: string }[]) {
  const heights = useRef(new Map<string, number>());
  const itemsRef = useRef(items);
  const view = useRef({ offset: 0, viewport: 0 });
  const frame = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [top, setTop] = useState<number | null>(null);

  const recompute = useCallback(() => {
    if (view.current.viewport === 0) return;
    const reach = view.current.offset + view.current.viewport;
    const next = rowAtReach(
      itemsRef.current.map((item) => heights.current.get(item.id)),
      reach,
    );
    setTop((current) => (current === next ? current : next));
  }, []);

  const schedule = useCallback(() => {
    if (frame.current != null) return;
    frame.current = setTimeout(() => {
      frame.current = null;
      recompute();
    }, 48);
  }, [recompute]);

  useEffect(() => {
    itemsRef.current = items;
    recompute();
  }, [items, recompute]);

  useEffect(
    () => () => {
      if (frame.current != null) clearTimeout(frame.current);
    },
    [],
  );

  const onRowLayout = useCallback(
    (id: string, height: number) => {
      if (heights.current.get(id) === height) return;
      heights.current.set(id, height);
      schedule();
    },
    [schedule],
  );
  const onScroll = useCallback(
    (offsetFromNewest: number, viewportHeight: number) => {
      view.current = { offset: offsetFromNewest, viewport: viewportHeight };
      schedule();
    },
    [schedule],
  );
  const onViewport = useCallback(
    (viewportHeight: number) => {
      view.current = { ...view.current, viewport: viewportHeight };
      schedule();
    },
    [schedule],
  );
  return { top, onRowLayout, onScroll, onViewport };
}
