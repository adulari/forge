import { describe, expect, it } from "vitest";

import { agentsForInvocation } from "./agentLinks";
import { agentListItems } from "./agentList";
import type { AgentIndexRow } from "./api";

const row = (id: string, created_at: number, turn = 1): AgentIndexRow => ({
  session_id: id,
  agent: id,
  task: "t",
  model: null,
  status: "done",
  cost_usd: 0,
  created_at,
  turn,
});

describe("agentsForInvocation", () => {
  const agents = [row("late", 200), row("b", 102), row("a", 101), row("early", 50)];

  it("links the children spawned between the call and its result, oldest first", () => {
    const got = agentsForInvocation({ seq: 1, tool: "spawn_agents", args: "", result: "ok", at: 100, resultAt: 110 }, agents);
    expect(got.map((r) => r.session_id)).toEqual(["a", "b"]);
  });

  it("keeps an unanswered call open-ended and ignores other tools", () => {
    const open = agentsForInvocation({ seq: 1, tool: "spawn_agents", args: "", result: null, at: 150 }, agents);
    expect(open.map((r) => r.session_id)).toEqual(["late"]);
    expect(agentsForInvocation({ seq: 1, tool: "read_file", args: "", result: "", at: 100, resultAt: 300 }, agents)).toEqual([]);
    expect(agentsForInvocation({ seq: 1, tool: "spawn_agents", args: "", result: "" }, agents)).toEqual([]);
  });
});

describe("agentListItems", () => {
  it("puts the live batch first, then earlier turns newest first, without repeating live rows", () => {
    const live = [{ id: "x", session_id: "x", agent: "x", task: "", model: null, phase: null, last: "", done: false, ok: true, cost: 0 }];
    const items = agentListItems(live, [row("x", 300, 3), row("c", 200, 2), row("a", 100, 1), row("b", 101, 1)]);
    expect(items.map((i) => (i.kind === "header" ? i.label : i.agent.id))).toEqual(["x", "Turn 2", "c", "Turn 1", "a", "b"]);
  });

  it("is just the live batch when the host has no index", () => {
    expect(agentListItems([], [])).toEqual([]);
  });
});
