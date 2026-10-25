// At most one committed value per paint. The session socket receives up to ~33 full snapshots a
// second while a reply streams, and over Forge Anywhere several can land in the same tick after a
// slow decrypt; committing each one re-rendered the whole session screen per frame, and the JS
// thread fell further behind with every frame. Snapshots are full state, so only the newest of a
// paint's worth matters.

type Schedule = (run: () => void) => () => void;

/** Next animation frame, with a timer backstop: a hidden web tab or a backgrounded webview stops
 * delivering frames, and the session state must still advance there. */
const nextPaint: Schedule = (run) => {
  let done = false;
  const once = () => {
    if (done) return;
    done = true;
    run();
  };
  const raf = typeof requestAnimationFrame === "function" ? requestAnimationFrame(once) : null;
  const timer = setTimeout(once, 50);
  return () => {
    done = true;
    if (raf != null) cancelAnimationFrame(raf);
    clearTimeout(timer);
  };
};

export interface FrameCoalescer<T> {
  push: (value: T) => void;
  /** Commits a pending value now (a terminal frame that must not wait). */
  flush: () => void;
  /** Drops a pending value without committing it. */
  cancel: () => void;
}

export function createFrameCoalescer<T>(commit: (value: T) => void, schedule: Schedule = nextPaint): FrameCoalescer<T> {
  let pending: { value: T } | null = null;
  let cancelScheduled: (() => void) | null = null;
  const flush = () => {
    cancelScheduled?.();
    cancelScheduled = null;
    const next = pending;
    pending = null;
    if (next) commit(next.value);
  };
  return {
    push(value) {
      pending = { value };
      if (!cancelScheduled) cancelScheduled = schedule(flush);
    },
    flush,
    cancel() {
      cancelScheduled?.();
      cancelScheduled = null;
      pending = null;
    },
  };
}
