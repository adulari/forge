import type { GoalPhase, RemoteInput, Snapshot, SnapshotGoal } from "./ws";

export interface GoalView {
  objective: string;
  phase: GoalPhase;
  /** Read from the host's structured goal; `false` means it was inferred from transient notes by
   * a host that predates `Snapshot.goal`, which cannot be asked to pause or edit. */
  structured: boolean;
  iteration: number | null;
  tokensUsed: number;
  maxTokens: number | null;
  maxMinutes: number | null;
  elapsedSecs: number;
  syncedAt: number;
  gates: number;
  reason: string | null;
  pending: "pause" | "clear" | null;
}

/** The goal an older host announces only through the CLI's transient notes. */
export function deriveNoteGoal(notes: string[] | undefined): { condition: string } | null {
  if (!notes || notes.length === 0) return null;
  let condition: string | null = null;
  let active = false;
  for (const note of notes) {
    const lower = note.toLowerCase();
    if (lower.includes("goal set")) {
      const match = note.match(/goal set\s*[—–-]?\s*(.*)$/i);
      const text = match?.[1]?.trim();
      if (text) condition = text;
      active = true;
    } else if (lower.includes("goal started")) {
      active = true;
    } else if (lower.includes("goal stopped") || lower.includes("goal complete")) {
      active = false;
    }
  }
  if (!active) return null;
  return { condition: condition ?? "Autonomous goal running" };
}

function fromWire(goal: SnapshotGoal): GoalView {
  return {
    objective: goal.objective,
    phase: goal.state,
    structured: true,
    iteration: goal.iteration,
    tokensUsed: goal.tokens_used,
    maxTokens: goal.max_tokens,
    maxMinutes: goal.max_minutes,
    elapsedSecs: goal.elapsed_secs,
    syncedAt: goal.synced_at,
    gates: goal.gates.length,
    reason: goal.reason,
    pending: goal.pending,
  };
}

export function resolveGoal(snapshot: Pick<Snapshot, "goal" | "notes"> | null): GoalView | null {
  if (!snapshot) return null;
  if (snapshot.goal) return fromWire(snapshot.goal);
  const noted = deriveNoteGoal(snapshot.notes);
  if (!noted) return null;
  return {
    objective: noted.condition,
    phase: "active",
    structured: false,
    iteration: null,
    tokensUsed: 0,
    maxTokens: null,
    maxMinutes: null,
    elapsedSecs: 0,
    syncedAt: 0,
    gates: 0,
    reason: null,
    pending: null,
  };
}

export const GOAL_PHASE_LABEL: Record<GoalPhase, string> = {
  active: "active",
  paused: "paused",
  budget_limit: "budget limit",
  complete: "complete",
  failed: "failed",
  cancelled: "cancelled",
};

export type GoalAction = "pause" | "resume" | "stop" | "edit" | "clear";

/** What a goal in this state can be asked to do. Only a structured goal takes the `/goal` verbs:
 * an older host would read `/goal pause` as a new goal literally named "pause". */
export function goalActions(goal: GoalView): GoalAction[] {
  if (!goal.structured) return goal.phase === "active" ? ["stop"] : [];
  if (goal.phase === "active") {
    return goal.pending ? ["stop"] : ["pause", "edit", "stop"];
  }
  if (goal.phase === "paused") return ["resume", "edit", "clear"];
  return ["clear"];
}

export function goalInput(action: GoalAction, objective?: string): RemoteInput {
  switch (action) {
    case "stop":
      return { kind: "interrupt" };
    case "edit":
      return { kind: "prompt", text: `/goal edit ${(objective ?? "").trim()}` };
    default:
      return { kind: "prompt", text: `/goal ${action}` };
  }
}

export function compactTokens(n: number): string {
  if (n < 1_000) return String(n);
  if (n < 1_000_000) return `${(n / 1_000).toFixed(1).replace(/\.0$/, "")}k`;
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
}

export function formatElapsed(secs: number): string {
  const total = Math.max(0, Math.floor(secs));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${String(s).padStart(2, "0")}s`;
  return `${s}s`;
}

/** Run time now: the host's reading plus the time since it, while the goal is still running. */
export function liveElapsedSecs(goal: GoalView, nowMs: number): number {
  if (goal.phase !== "active" || goal.syncedAt <= 0) return goal.elapsedSecs;
  return goal.elapsedSecs + Math.max(0, (nowMs - goal.syncedAt) / 1000);
}

/** Share of the token budget spent, 0..1; `null` without a budget. */
export function budgetFraction(goal: GoalView): number | null {
  if (goal.maxTokens == null || goal.maxTokens <= 0) return null;
  return Math.min(1, goal.tokensUsed / goal.maxTokens);
}

export function goalMeta(goal: GoalView, nowMs: number): string {
  const parts: string[] = [];
  if (goal.iteration != null) parts.push(`turn ${goal.iteration}`);
  if (goal.tokensUsed > 0 || goal.maxTokens != null) {
    parts.push(
      goal.maxTokens != null
        ? `${compactTokens(goal.tokensUsed)} / ${compactTokens(goal.maxTokens)} tok`
        : `${compactTokens(goal.tokensUsed)} tok`,
    );
  }
  if (goal.structured) {
    const elapsed = formatElapsed(liveElapsedSecs(goal, nowMs));
    parts.push(goal.maxMinutes != null ? `${elapsed} / ${goal.maxMinutes}m` : elapsed);
  }
  if (goal.gates > 0) parts.push(`${goal.gates} ${goal.gates === 1 ? "gate" : "gates"}`);
  return parts.join(" · ");
}
