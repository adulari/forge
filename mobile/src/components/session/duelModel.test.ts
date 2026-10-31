import { describe, expect, it } from "vitest";

import type { Overlay } from "../../lib/ws";
import { duelCommand, panesFromOverlay, parseDuelHeading, parseRowLabel, splitJudgeField } from "./duelModel";

describe("duelCommand", () => {
  it("leaves the default plain", () => {
    expect(duelCommand("  fix it ", { judge: false, candidates: 3 })).toBe("/duel fix it");
  });
  it("adds judge and a non-default count, clamped", () => {
    expect(duelCommand("fix it", { judge: true, candidates: 4 })).toBe("/duel --judge --n 4 fix it");
    expect(duelCommand("x", { judge: false, candidates: 99 })).toBe("/duel --n 5 x");
    expect(duelCommand("x", { judge: false, candidates: 0 })).toBe("/duel --n 2 x");
  });
});

describe("splitJudgeField", () => {
  it("peels the judge field off the end", () => {
    const parts = ["judge #1 9.5/10: tidy and small", "2 files +5 -1", "tests ✓", "2.5s", "$0.0123", "did it"];
    const out = splitJudgeField(parts);
    expect(out.parts).toHaveLength(5);
    expect(out.judge).toEqual({ rank: 1, score: 9.5, critique: "tidy and small" });
  });
  it("leaves an unjudged row alone", () => {
    const parts = ["1 files +1 -0", "tests –", "1.0s", "$0.0000", "summary"];
    expect(splitJudgeField(parts)).toEqual({ parts });
  });
});

describe("parseDuelHeading", () => {
  it("reads a verdict, a failure and nothing", () => {
    expect(parseDuelHeading("⚔ duel — pick the winner (3 candidates) · judge m::x: c is tidy")).toEqual({
      judgeModel: "m::x",
      rationale: "c is tidy",
    });
    expect(parseDuelHeading("⚔ duel — pick the winner (3 candidates) · judge failed: 429")).toEqual({ judgeError: "429" });
    expect(parseDuelHeading("⚔ duel — pick the winner (2 candidates)")).toEqual({});
  });
});

describe("parseRowLabel", () => {
  it("strips badges and notices the crown", () => {
    expect(parseRowLabel("★ ✓ ns::model")).toEqual({ model: "ns::model", crowned: true });
    expect(parseRowLabel("✗ ns::model")).toEqual({ model: "ns::model", crowned: false });
  });
});

describe("panesFromOverlay", () => {
  const overlay = (rows: { id: string; label: string; detail: string }[]): Overlay => ({
    kind: "picker:duel",
    title: "x",
    rows: rows.map((r) => ({ ...r, selected: false, group: null })) as Overlay["rows"],
    selected: 0,
    filter: null,
    free_text: false,
    body: null,
  });

  it("reads a judged row", () => {
    const [pane] = panesFromOverlay(
      overlay([
        {
          id: "forge/c",
          label: "★ ✓ ns::c",
          detail: "judge #1 9.0/10: tidy · 2 files +5 -1 · tests ✓ · 2.5s · $0.0123 · did the thing",
        },
      ]),
    );
    expect(pane.model).toBe("ns::c");
    expect(pane.crowned).toBe(true);
    expect(pane.judge).toEqual({ rank: 1, score: 9, critique: "tidy" });
    expect(pane.diffstat).toEqual({ files: 2, added: 5, removed: 1 });
    expect(pane.tests).toBe(true);
    expect(pane.latencySec).toBe(2.5);
    expect(pane.costUsd).toBeCloseTo(0.0123);
    expect(pane.body).toBe("did the thing");
  });

  it("keeps an unjudged row as it was", () => {
    const [pane] = panesFromOverlay(
      overlay([{ id: "forge/a", label: "✗ ns::a", detail: "0 files +0 -0 · tests ✗ · 1.0s · $0.0000 · oops" }]),
    );
    expect(pane.crowned).toBe(false);
    expect(pane.judge).toBeUndefined();
    expect(pane.tests).toBe(false);
    expect(pane.body).toBe("oops");
  });
});
