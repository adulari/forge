// Commits the reply text at most once per `intervalMs`. Painting every token costs a React
// render plus a native text re-measure each time, and on a phone that cost, not the network,
// is what keeps the JS thread busy while a reply streams. Twenty updates a second still read as
// continuous writing, and the newest text always lands: the last value is committed when the
// interval ends, and `flush` commits it at once (end of a turn, anything that is not text).

type Clock = {
  now: () => number;
  setTimer: (run: () => void, ms: number) => unknown;
  clearTimer: (handle: unknown) => void;
};

const realClock: Clock = {
  now: () => Date.now(),
  setTimer: (run, ms) => setTimeout(run, ms),
  clearTimer: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

export const STREAM_COMMIT_INTERVAL_MS = 50;

export interface StreamPacer<T> {
  /** Commits now when the interval has passed, otherwise on its last tick. */
  push: (value: T) => void;
  /** Commits `value` (or the pending one) now and cancels the wait. */
  flush: (value?: T) => void;
  /** Drops the pending value. */
  cancel: () => void;
}

export function createStreamPacer<T>(
  commit: (value: T) => void,
  intervalMs: number = STREAM_COMMIT_INTERVAL_MS,
  clock: Clock = realClock,
): StreamPacer<T> {
  let pending: { value: T } | null = null;
  let timer: unknown = null;
  let lastAt = Number.NEGATIVE_INFINITY;

  const run = () => {
    if (timer !== null) clock.clearTimer(timer);
    timer = null;
    const next = pending;
    pending = null;
    if (!next) return;
    lastAt = clock.now();
    commit(next.value);
  };

  return {
    push(value) {
      pending = { value };
      if (timer !== null) return;
      const wait = lastAt + intervalMs - clock.now();
      if (wait <= 0) run();
      else timer = clock.setTimer(run, wait);
    },
    flush(value) {
      if (arguments.length > 0) pending = { value: value as T };
      run();
    },
    cancel() {
      if (timer !== null) clock.clearTimer(timer);
      timer = null;
      pending = null;
    },
  };
}
