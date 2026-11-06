import { describe, expect, it } from "vitest";

import { deepEqual, stabilizeSnapshot } from "./snapshotStable";

const base = {
  busy: true,
  title: "t",
  tasks: [{ title: "a", status: "done" }],
  transcript: ["you", "  hi"],
  plan: null as null | { steps: string[] },
  streaming: "",
  streaming_offset: 0,
  revision: 1,
};

describe("deepEqual", () => {
  it("compares nested arrays and objects by value", () => {
    expect(deepEqual({ a: [1, { b: 2 }] }, { a: [1, { b: 2 }] })).toBe(true);
    expect(deepEqual({ a: [1, { b: 2 }] }, { a: [1, { b: 3 }] })).toBe(false);
    expect(deepEqual([1], { 0: 1 })).toBe(false);
    expect(deepEqual(null, {})).toBe(false);
    expect(deepEqual({ a: undefined }, {})).toBe(false);
  });
});

describe("stabilizeSnapshot", () => {
  it("takes the first snapshot as it is", () => {
    expect(stabilizeSnapshot(null, base)).toBe(base);
  });

  it("returns the previous object when only streamed fields moved", () => {
    const next = { ...base, tasks: [...base.tasks], transcript: [...base.transcript], streaming: "x", streaming_offset: 4, revision: 2 };
    expect(stabilizeSnapshot(base, next)).toBe(base);
  });

  it("keeps the references of fields that did not change when another one did", () => {
    const next = { ...base, tasks: [...base.tasks], transcript: [...base.transcript], busy: false, revision: 2 };
    const out = stabilizeSnapshot(base, next);
    expect(out).not.toBe(base);
    expect(out.busy).toBe(false);
    expect(out.tasks).toBe(base.tasks);
    expect(out.transcript).toBe(base.transcript);
  });

  it("notices a changed nested value and a field that appears", () => {
    const edited = stabilizeSnapshot(base, { ...base, transcript: ["you", "  hi", "forge"] });
    expect(edited.transcript).toHaveLength(3);
    const extra = stabilizeSnapshot(base, { ...base, history_gap: 1 } as typeof base);
    expect(extra).not.toBe(base);
  });
});
