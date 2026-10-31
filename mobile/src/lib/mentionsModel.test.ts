import { describe, expect, it } from "vitest";

import { draftMentions, isMentionOption, mentionOptions, removeMention } from "./mentionsModel";

const sessions = [
  { id: "a", title: "auth refactor" },
  { id: "b", title: "docs pass" },
  { id: "me", title: "this one" },
  { id: "c", title: null },
];

describe("mentionOptions", () => {
  it("offers the three specials for a bare @", () => {
    expect(mentionOptions("", sessions, "me").map((o) => o.label)).toEqual(["@terminal", "@browser", "@Subagents"]);
  });

  it("narrows by prefix and leaves file queries alone", () => {
    expect(mentionOptions("te", sessions, "me").map((o) => o.kind)).toEqual(["terminal"]);
    expect(mentionOptions("src/main", sessions, "me")).toEqual([]);
    expect(mentionOptions("Cargo", sessions, "me")).toEqual([]);
  });

  it("lists other sessions by name for session:", () => {
    const all = mentionOptions("session:", sessions, "me");
    expect(all.map((o) => o.path)).toEqual(["session:auth refactor", "session:docs pass"]);
    expect(mentionOptions("session:doc", sessions, "me").map((o) => o.path)).toEqual(["session:docs pass"]);
    expect(mentionOptions("ses", sessions, "me").length).toBe(2);
  });

  it("tells a mention option from a workspace search hit", () => {
    expect(isMentionOption(mentionOptions("", [], null)[0])).toBe(true);
    expect(isMentionOption({ path: "a.ts", kind: "file", line: null, column: null, preview: null })).toBe(false);
  });
});

describe("draftMentions", () => {
  it("finds each kind once, in order", () => {
    const found = draftMentions("see @session:{auth refactor} and @terminal, @Browser @subagents @terminal x@session:no");
    expect(found.map((m) => [m.kind, m.label])).toEqual([
      ["session", "auth refactor"],
      ["terminal", "terminal"],
      ["browser", "browser"],
      ["subagents", "Subagents"],
    ]);
  });

  it("ignores file mentions", () => {
    expect(draftMentions("read @src/main.rs and @{my file.md}")).toEqual([]);
  });
});

describe("removeMention", () => {
  it("drops the token and one following space", () => {
    expect(removeMention("look at @terminal now", "@terminal")).toBe("look at now");
    expect(removeMention("nothing", "@terminal")).toBe("nothing");
  });
});
