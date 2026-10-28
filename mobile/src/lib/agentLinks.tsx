// Which child agents a delegating tool call started, so its chat row can open them. The store
// keeps no call -> child mapping, so the link is the time window the call spanned: the children a
// session spawned between the call being written and its result being written. That also covers
// rows written before this existed. A call whose window holds no child shows no link.
import { createContext, useContext } from "react";

import type { AgentIndexRow } from "./api";
import type { ToolInvocation } from "./toolRows";

const DELEGATING_TOOLS = new Set(["spawn_agents", "run_workflow", "workflow"]);
// Row and session timestamps are whole seconds, written by different statements.
const SLACK_SECONDS = 2;

export function agentsForInvocation(invocation: ToolInvocation, agents: readonly AgentIndexRow[]): AgentIndexRow[] {
  if (invocation.tool == null || !DELEGATING_TOOLS.has(invocation.tool) || invocation.at == null) return [];
  const from = invocation.at - SLACK_SECONDS;
  const to = invocation.resultAt == null ? Infinity : invocation.resultAt + SLACK_SECONDS;
  return agents.filter((a) => a.created_at >= from && a.created_at <= to).slice().reverse();
}

const AgentIndexContext = createContext<readonly AgentIndexRow[]>([]);
export const AgentIndexProvider = AgentIndexContext.Provider;
export const useAgentIndex = () => useContext(AgentIndexContext);
