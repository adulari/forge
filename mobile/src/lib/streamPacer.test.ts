import { describe, expect, it } from "vitest";

import { createStreamPacer } from "./streamPacer";

function fakeClock() {
  let now = 1000;
  let timer: { at: number; run: () => void } | null = null;
  return {
    clock: {
      now: () => now,
      setTimer: (run: () => void, ms: number) => {
        timer = { at: now + ms, run };
        return timer;
      },
      clearTimer: (handle: unknown) => {
        if (timer === handle) timer = null;
      },
    },
    advance(ms: number) {
      now += ms;
      if (timer && timer.at <= now) {
        const t = timer;
        timer = null;
        t.run();
      }
    },
    pending: () => timer !== null,
  };
}

describe("createStreamPacer", () => {
  it("commits the first value at once and then at most once per interval", () => {
    const t = fakeClock();
    const seen: string[] = [];
    const pacer = createStreamPacer<string>((v) => seen.push(v), 50, t.clock);
    pacer.push("a");
    expect(seen).toEqual(["a"]);
    t.advance(10);
    pacer.push("ab");
    t.advance(10);
    pacer.push("abc");
    expect(seen).toEqual(["a"]);
    t.advance(30);
    expect(seen).toEqual(["a", "abc"]);
    expect(t.pending()).toBe(false);
  });

  it("commits immediately once the interval has passed", () => {
    const t = fakeClock();
    const seen: string[] = [];
    const pacer = createStreamPacer<string>((v) => seen.push(v), 50, t.clock);
    pacer.push("a");
    t.advance(80);
    pacer.push("ab");
    expect(seen).toEqual(["a", "ab"]);
  });

  it("flush commits the given value and cancels the wait", () => {
    const t = fakeClock();
    const seen: string[] = [];
    const pacer = createStreamPacer<string>((v) => seen.push(v), 50, t.clock);
    pacer.push("a");
    pacer.push("ab");
    expect(t.pending()).toBe(true);
    pacer.flush("");
    expect(seen).toEqual(["a", ""]);
    expect(t.pending()).toBe(false);
    t.advance(100);
    expect(seen).toEqual(["a", ""]);
  });

  it("flush with no argument commits the pending value, and cancel drops it", () => {
    const t = fakeClock();
    const seen: string[] = [];
    const pacer = createStreamPacer<string>((v) => seen.push(v), 50, t.clock);
    pacer.push("a");
    pacer.push("ab");
    pacer.flush();
    expect(seen).toEqual(["a", "ab"]);
    pacer.push("abc");
    pacer.cancel();
    t.advance(100);
    expect(seen).toEqual(["a", "ab"]);
  });
});
