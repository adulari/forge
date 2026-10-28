// The Agents tab list: the live batch first, then every earlier child the host indexes, newest
// turn first. An index row the live batch already shows is skipped; a host with no index (an older
// daemon) simply yields the live batch.
import type { AgentIndexRow } from "./api";
import type { SnapshotSubagent } from "./ws";

export type AgentListItem =
  | { kind: "header"; key: string; label: string }
  | { kind: "agent"; key: string; agent: SnapshotSubagent };

function fromIndex(row: AgentIndexRow): SnapshotSubagent {
  return {
    id: row.session_id,
    session_id: row.session_id,
    agent: row.agent,
    task: row.task,
    model: row.model,
    phase: null,
    last: "",
    done: row.status !== "running",
    ok: row.status === "done",
    cost: row.cost_usd,
  };
}

export function agentListItems(live: readonly SnapshotSubagent[], index: readonly AgentIndexRow[]): AgentListItem[] {
  const items: AgentListItem[] = live.map((agent, i) => ({ kind: "agent", key: `live-${i}-${agent.agent}`, agent }));
  const shown = new Set(live.map((a) => a.session_id ?? a.id));
  const earlier = index.filter((row) => !shown.has(row.session_id));
  if (earlier.length === 0) return items;
  const turns = new Map<number, AgentIndexRow[]>();
  for (const row of earlier) turns.set(row.turn, [...(turns.get(row.turn) ?? []), row]);
  const ordered = [...turns.entries()].sort((a, b) => b[0] - a[0]);
  for (const [turn, rows] of ordered) {
    items.push({ kind: "header", key: `turn-${turn}`, label: `Turn ${turn}` });
    for (const row of rows) items.push({ kind: "agent", key: `idx-${row.session_id}`, agent: fromIndex(row) });
  }
  return items;
}
