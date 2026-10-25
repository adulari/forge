import { describe, expect, it } from "vitest";

import { createFrameCoalescer } from "./frameCoalescer";

function manual() {
  const queue: (() => void)[] = [];
  const schedule = (run: () => void) => {
    queue.push(run);
    return () => queue.splice(queue.indexOf(run), 1);
  };
  return { queue, schedule };
}

describe("createFrameCoalescer", () => {
  it("commits only the newest value of a paint", () => {
    const { queue, schedule } = manual();
    const commits: number[] = [];
    const c = createFrameCoalescer<number>((v) => commits.push(v), schedule);
    c.push(1);
    c.push(2);
    c.push(3);
    expect(queue).toHaveLength(1);
    queue[0]();
    expect(commits).toEqual([3]);
  });

  it("flushes a terminal value at once and cancels without committing", () => {
    const { queue, schedule } = manual();
    const commits: number[] = [];
    const c = createFrameCoalescer<number>((v) => commits.push(v), schedule);
    c.push(1);
    c.flush();
    expect(commits).toEqual([1]);
    expect(queue).toHaveLength(0);
    c.push(2);
    c.cancel();
    expect(queue).toHaveLength(0);
    expect(commits).toEqual([1]);
  });
});
