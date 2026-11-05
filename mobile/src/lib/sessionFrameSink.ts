// Where one committed frame of the session socket goes: the reply text to its store (paced), and
// the rest of the snapshot to React state only when something other than the text changed. A
// reply that streams for a minute therefore commits a handful of snapshots, not one per token.
import { stabilizeSnapshot } from "./snapshotStable";
import { createStreamPacer, type StreamPacer } from "./streamPacer";
import type { StreamStore } from "./streamStore";

interface FrameShape {
  streaming: string;
  closed?: boolean;
}

export interface SessionFrameSink<S extends FrameShape> {
  commit: (frame: S) => void;
  /** Forgets the snapshot and the reply (a different session, or a fresh connection). */
  reset: () => void;
  cancel: () => void;
}

export function createSessionFrameSink<S extends FrameShape>(
  store: StreamStore,
  setSnapshot: (snapshot: S | null) => void,
  pacer: StreamPacer<string> = createStreamPacer<string>((text) => store.set(text)),
): SessionFrameSink<S> {
  let committed: S | null = null;
  return {
    commit(frame) {
      const next = stabilizeSnapshot(committed, { ...frame, streaming: "" });
      const changed = next !== committed;
      // Only the reply text moved: pace it. Anything else (a turn edge, a tool, a cleared reply)
      // commits the text at once, first, so the render the snapshot causes already reads it.
      if (changed || frame.streaming === "" || frame.closed) pacer.flush(frame.streaming);
      else pacer.push(frame.streaming);
      if (changed) {
        committed = next;
        setSnapshot(next);
      }
    },
    reset() {
      pacer.cancel();
      store.set("");
      store.rememberCurrent();
      committed = null;
      setSnapshot(null);
    },
    cancel() {
      pacer.cancel();
    },
  };
}
