// Shared derivations for the Workflow suite. Everything here is pure — no wire calls,
// no fabricated fields. The elapsed timer is measured client-side from first-seen (the
// wire carries no run start timestamp), so callers own the "since mount" contract.
import { useEffect, useState } from "react";

import type { SnapshotSubagent, SnapshotWorkflow } from "../../lib/ws";

/** `45s` · `1m 32s` · `2m 04s` · `1h 04m` — mono, tabular. Seconds zero-pad under a minute
 *  boundary so a live counter doesn't jitter its width. */
export function formatDuration(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return `${h}h ${String(m).padStart(2, "0")}m`;
  if (m > 0) return `${m}m ${String(sec).padStart(2, "0")}s`;
  return `${sec}s`;
}

export type PhaseState = "done" | "running" | "pending";

export interface PhaseGroup {
  phase: string;
  rows: SnapshotSubagent[];
  state: PhaseState;
  doneCount: number;
  runningCount: number;
  cost: number;
  /** True when this group holds rows whose `phase` is not in the workflow's declared order. */
  unknown: boolean;
}

/** Group workflow agent rows (subagents whose `phase` is set) by phase, preserving the
 *  declared `phases[]` order. Rows with a phase not in that list collect into a trailing
 *  "other" group. A phase with zero rows is still emitted (pending). */
export function groupByPhase(rows: SnapshotSubagent[], phases: string[]): PhaseGroup[] {
  const byPhase = new Map<string, SnapshotSubagent[]>();
  for (const row of rows) {
    if (row.phase == null) continue;
    const list = byPhase.get(row.phase);
    if (list) list.push(row);
    else byPhase.set(row.phase, [row]);
  }

  const groups: PhaseGroup[] = phases.map((phase) => buildGroup(phase, byPhase.get(phase) ?? [], false));

  // Trailing group: any phase the row carried that the workflow never declared.
  const trailing: SnapshotSubagent[] = [];
  for (const [phase, list] of byPhase) {
    if (!phases.includes(phase)) trailing.push(...list);
  }
  if (trailing.length > 0) groups.push(buildGroup("other", trailing, true));

  return groups;
}

function buildGroup(phase: string, rows: SnapshotSubagent[], unknown: boolean): PhaseGroup {
  const doneCount = rows.filter((r) => r.done).length;
  const runningCount = rows.length - doneCount;
  const cost = rows.reduce((sum, r) => sum + (r.cost ?? 0), 0);
  const state: PhaseState = rows.length === 0 ? "pending" : runningCount > 0 ? "running" : "done";
  return { phase, rows, state, doneCount, runningCount, cost, unknown };
}

export function workflowRows(subagents: SnapshotSubagent[]): SnapshotSubagent[] {
  return subagents.filter((row) => row.phase != null);
}

export function totalCost(rows: SnapshotSubagent[]): number {
  return rows.reduce((sum, r) => sum + (r.cost ?? 0), 0);
}

/** A failed agent = finished but the last turn did not succeed. */
export function isFailed(row: SnapshotSubagent): boolean {
  return row.done && !row.ok;
}

export function workflowTitle(workflow: SnapshotWorkflow): string {
  return workflow.name && workflow.name.length > 0 ? workflow.name : "workflow";
}

/** Extract a JSON object/array embedded in free text (a result value or an agent's last line).
 *  Returns null for plain prose so the caller can fall back to text rendering. Only
 *  objects/arrays qualify — a bare quoted string is not a "structured output" block. */
export function extractJson(text: string | null | undefined): unknown | null {
  if (!text) return null;
  const trimmed = text.trim();
  const whole = tryParseObject(trimmed);
  if (whole !== undefined) return whole;

  const start = trimmed.search(/[{[]/);
  if (start < 0) return null;
  const open = trimmed[start];
  const close = open === "{" ? "}" : "]";
  const end = trimmed.lastIndexOf(close);
  if (end <= start) return null;
  return tryParseObject(trimmed.slice(start, end + 1)) ?? null;
}

function tryParseObject(candidate: string): unknown | undefined {
  try {
    const value = JSON.parse(candidate);
    if (value !== null && typeof value === "object") return value;
  } catch {
    // not JSON — fall through
  }
  return undefined;
}

/** Elapsed seconds since this component first mounted, ticking once a second while `active`.
 *  Freezes at the last value when `active` goes false. The wire carries no run start time, so
 *  this is honestly "since first-seen", not "since the run began". */
export function useElapsedSeconds(active: boolean): number {
  const [start] = useState(() => Date.now());
  const [now, setNow] = useState(start);

  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [active]);

  return Math.floor((now - start) / 1000);
}
