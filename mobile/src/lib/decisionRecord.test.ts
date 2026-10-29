import { describe, expect, it } from "vitest";

import { isDecision, parseDecision } from "./decisionRecord";
import type { ToolInvocation } from "./toolRows";

const inv = (args: unknown, result: string | null, tool = "ask_user"): ToolInvocation => ({
  seq: 1,
  tool,
  args: typeof args === "string" ? args : JSON.stringify(args),
  result,
});

describe("decision records", () => {
  it("reads one answered question with its option description", () => {
    const call = inv(
      { question: "Which DB?", header: "Database", options: [{ label: "Postgres", description: "relational" }, { label: "SQLite" }] },
      "Postgres",
    );
    expect(isDecision(call)).toBe(true);
    expect(parseDecision(call)).toEqual([
      { header: "Database", question: "Which DB?", chosen: "Postgres", description: "relational", note: "", answered: true },
    ]);
  });

  it("splits a form answer back into its questions and keeps the note", () => {
    const call = inv(
      {
        questions: [
          { header: "DB", question: "Which DB?", options: [{ label: "Postgres", description: "sql" }] },
          { header: "Scope", question: "Which features?", multi: true, options: [{ label: "Auth" }, { label: "Search" }] },
        ],
      },
      "1. DB: Postgres\n2. Scope: Auth, Search — note: later",
    );
    const items = parseDecision(call);
    expect(items.map((i) => i.chosen)).toEqual(["Postgres", "Auth, Search"]);
    expect(items[0].description).toBe("sql");
    expect(items[1]).toMatchObject({ description: "", note: "later", answered: true });
  });

  it("marks a non-interactive run as unanswered", () => {
    const call = inv({ question: "Go?" }, "(no answer — non-interactive)");
    expect(parseDecision(call)[0]).toMatchObject({ answered: false, chosen: "" });
  });

  it("still names the question when the stored arguments were capped", () => {
    const call = inv('{"header":"Deploy","question":"Ship it now?","options":[{"label":"Yes","descr…', "Yes");
    expect(parseDecision(call)[0]).toMatchObject({ header: "Deploy", question: "Ship it now?", chosen: "Yes" });
  });

  it("is not a decision while unanswered, failed, or another tool", () => {
    expect(isDecision(inv({ question: "x" }, null))).toBe(false);
    expect(isDecision(inv({ question: "x" }, "error: ask_user: question 1 has no text"))).toBe(false);
    expect(isDecision(inv({ question: "x" }, "Yes", "shell"))).toBe(false);
    expect(isDecision(inv({ question: "x" }, "Yes", "mcp__forge__ask_user"))).toBe(true);
  });
});
