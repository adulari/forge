import { describe, expect, it } from "vitest";

import { createSessionFrameSink } from "./sessionFrameSink";
import { createStreamStore } from "./streamStore";

interface Frame {
  busy: boolean;
  streaming: string;
  streaming_offset: number;
  revision: number;
  transcript: string[];
  closed?: boolean;
}

function frame(over: Partial<Frame> = {}): Frame {
  return { busy: true, streaming: "", streaming_offset: 0, revision: 1, transcript: ["you", "  hi"], ...over };
}

function manualPacer() {
  const calls: string[] = [];
  let pending: string | null = null;
  return {
    calls,
    pacer: {
      push: (v: string) => {
        pending = v;
      },
      flush: (v?: string) => {
        const value = v ?? pending;
        pending = null;
        if (value !== null) calls.push(value);
      },
      cancel: () => {
        pending = null;
      },
    },
    tick: () => {
      if (pending !== null) calls.push(pending);
      pending = null;
    },
  };
}

describe("createSessionFrameSink", () => {
  it("commits one snapshot for a reply that streams a hundred frames", () => {
    const store = createStreamStore();
    const snapshots: (Frame | null)[] = [];
    const p = manualPacer();
    const sink = createSessionFrameSink<Frame>(store, (s) => snapshots.push(s), p.pacer);
    sink.commit(frame({ streaming: "a" }));
    for (let n = 2; n <= 100; n++) {
      sink.commit(frame({ streaming: "a".repeat(n), revision: n, transcript: ["you", "  hi"] }));
    }
    expect(snapshots).toHaveLength(1);
    expect(snapshots[0]?.streaming).toBe("");
    p.tick();
    expect(p.calls.at(-1)).toBe("a".repeat(100));
  });

  it("commits the snapshot and the text together when something else changes", () => {
    const store = createStreamStore();
    const snapshots: (Frame | null)[] = [];
    const p = manualPacer();
    const sink = createSessionFrameSink<Frame>(store, (s) => snapshots.push(s), p.pacer);
    sink.commit(frame({ streaming: "abc" }));
    sink.commit(frame({ streaming: "", busy: false, revision: 2 }));
    expect(snapshots).toHaveLength(2);
    expect(snapshots[1]?.busy).toBe(false);
    expect(p.calls).toEqual(["abc", ""]);
  });

  it("flushes the text of a closing frame", () => {
    const store = createStreamStore();
    const p = manualPacer();
    const sink = createSessionFrameSink<Frame>(store, () => {}, p.pacer);
    sink.commit(frame({ streaming: "x" }));
    sink.commit(frame({ streaming: "xy", closed: true }));
    expect(p.calls.at(-1)).toBe("xy");
  });

  it("reset clears the reply and the snapshot", () => {
    const store = createStreamStore();
    const snapshots: (Frame | null)[] = [];
    const sink = createSessionFrameSink<Frame>(store, (s) => snapshots.push(s));
    sink.commit(frame({ streaming: "text" }));
    expect(store.get()).toBe("text");
    sink.reset();
    expect(store.get()).toBe("");
    expect(store.last()).toBe("");
    expect(snapshots.at(-1)).toBeNull();
    sink.commit(frame({ streaming: "again" }));
    expect(snapshots.at(-1)?.busy).toBe(true);
  });
});
