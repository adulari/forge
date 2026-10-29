import { describe, expect, it } from "vitest";

import { buildDiffEntries, defaultExpandedKeys, diffEntryKey, type DiffEntry } from "./gitDiffEntries";
import { type GitFileRow, type GitStatusResponse } from "../../lib/api";

const row = (path: string, adds = 1, dels = 0): GitFileRow => ({
  path,
  status: "M",
  orig_path: null,
  adds,
  dels,
  binary: false,
});

const status = (partial: Partial<GitStatusResponse>): GitStatusResponse => ({
  root: "/r",
  branch: "main",
  base_branch: null,
  staged: [],
  unstaged: [],
  untracked: [],
  truncated: 0,
  ...partial,
});

describe("buildDiffEntries", () => {
  it("orders staged, unstaged, untracked and keeps both cards of a twice-listed file", () => {
    const entries = buildDiffEntries(
      status({ staged: [row("a.rs")], unstaged: [row("a.rs"), row("b.rs")], untracked: [row("c.rs")] }),
    );
    expect(entries.map((entry) => entry.key)).toEqual([
      "staged:a.rs",
      "unstaged:a.rs",
      "unstaged:b.rs",
      "untracked:c.rs",
    ]);
    expect(entries.map((entry) => entry.staged)).toEqual([true, false, false, false]);
  });

  it("gives conflicted paths no card", () => {
    const entries = buildDiffEntries(status({ conflicted: [row("x.rs")], unstaged: [row("y.rs")] }));
    expect(entries.map((entry) => entry.path)).toEqual(["y.rs"]);
  });
});

describe("defaultExpandedKeys", () => {
  const entries = (sizes: number[]): DiffEntry[] =>
    buildDiffEntries(status({ unstaged: sizes.map((size, index) => row(`f${index}`, size)) }));

  it("always opens the first card, however large", () => {
    expect([...defaultExpandedKeys(entries([5000, 1]))]).toEqual([diffEntryKey("unstaged", "f0")]);
  });

  it("stops at six cards", () => {
    expect(defaultExpandedKeys(entries([1, 1, 1, 1, 1, 1, 1, 1])).size).toBe(6);
  });

  it("stops once the counted lines would pass the budget", () => {
    expect([...defaultExpandedKeys(entries([400, 300, 200]))]).toEqual(["unstaged:f0", "unstaged:f1"]);
  });

  it("is empty with nothing changed", () => {
    expect(defaultExpandedKeys([]).size).toBe(0);
  });
});
