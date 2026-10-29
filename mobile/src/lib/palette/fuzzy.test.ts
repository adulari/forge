import { describe, expect, it } from "vitest";

import { fuzzyScore, rankItems } from "./fuzzy";

describe("fuzzyScore", () => {
  it("matches an abbreviation of word initials", () => {
    expect(fuzzyScore("tgr", "Toggle Git review")).not.toBeNull();
    expect(fuzzyScore("tgr", "Toggle Git review")!).toBeGreaterThan(fuzzyScore("tgr", "Strong grounds review")!);
  });

  it("orders exact, prefix, word start, substring, subsequence", () => {
    const exact = fuzzyScore("git", "git")!;
    const prefix = fuzzyScore("git", "git review")!;
    const word = fuzzyScore("git", "toggle git review")!;
    const substring = fuzzyScore("git", "digital")!;
    const subsequence = fuzzyScore("gtr", "digital review")!;
    expect(exact).toBeGreaterThan(prefix);
    expect(prefix).toBeGreaterThan(word);
    expect(word).toBeGreaterThan(substring);
    expect(substring).toBeGreaterThan(subsequence);
  });

  it("rejects text that lacks the characters in order", () => {
    expect(fuzzyScore("zq", "toggle git review")).toBeNull();
    expect(fuzzyScore("rg", "git review")).toBeNull();
  });

  it("requires every word of a multi-word query", () => {
    expect(fuzzyScore("git rev", "Toggle Git review")).not.toBeNull();
    expect(fuzzyScore("git zzz", "Toggle Git review")).toBeNull();
  });

  it("is case-insensitive and treats an empty query as a match", () => {
    expect(fuzzyScore("TOGGLE", "toggle terminal")).not.toBeNull();
    expect(fuzzyScore("  ", "anything")).toBe(0);
  });
});

describe("rankItems", () => {
  const items = [
    { title: "Open settings", id: "a" },
    { title: "Settings", id: "b" },
    { title: "Reset sandbox", subtitle: "settings of the sandbox", id: "c" },
  ];

  it("puts the best fuzzy score first and drops non-matches", () => {
    const ranked = rankItems(items, "settings");
    expect(ranked.map((i) => i.id)).toEqual(["b", "a", "c"]);
    expect(rankItems(items, "zzz")).toEqual([]);
  });

  it("breaks equal scores by priority, then recency, then input order", () => {
    const rows = [
      { title: "alpha one", id: "old", recency: 1 },
      { title: "alpha two", id: "new", recency: 9 },
      { title: "alpha six", id: "pinned", priority: true, recency: 0 },
    ];
    expect(rankItems(rows, "alpha").map((r) => r.id)).toEqual(["pinned", "new", "old"]);
    expect(rankItems(rows, "").map((r) => r.id)).toEqual(["pinned", "new", "old"]);
  });

  it("does not let scattered letters match a subtitle or keywords", () => {
    const rows = [{ title: "Servers", subtitle: "Parent and child sessions", keywords: "tree subagents", id: "noise" }];
    expect(rankItems(rows, "tgr")).toEqual([]);
    expect(rankItems(rows, "child").map((r) => r.id)).toEqual(["noise"]);
  });

  it("weights title matches above subtitle and keyword matches", () => {
    const rows = [
      { title: "Unrelated", keywords: "deploy", id: "kw" },
      { title: "Deploy", id: "title" },
      { title: "Other", subtitle: "deploy notes", id: "sub" },
    ];
    expect(rankItems(rows, "deploy").map((r) => r.id)).toEqual(["title", "sub", "kw"]);
  });

  it("keeps a daemon-matched row that the title alone would not match", () => {
    const rows = [
      { title: "Fix login redirect", id: "title" },
      { title: "Weekly notes", id: "hit", fixedScore: 300 },
    ];
    expect(rankItems(rows, "login").map((r) => r.id)).toEqual(["title", "hit"]);
  });
});
