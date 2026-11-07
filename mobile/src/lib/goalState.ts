import type {
  GoalPhase,
  RemoteInput,
  Snapshot,
  SnapshotContract,
  SnapshotEnvelope,
  SnapshotGoal,
  SnapshotRelayLink,
} from "./ws";

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
  envelope: SnapshotEnvelope | null;
  /** The validation contract table, when the goal was started with one. */
  contract: SnapshotContract | null;
  /** The fresh-worker relay chain, oldest session first; empty when the goal never changed worker. */
  relay: SnapshotRelayLink[];
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
    envelope: goal.envelope ?? null,
    contract: goal.contract ?? null,
    relay: goal.relay ?? [],
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
    envelope: null,
    contract: null,
    relay: [],
  };
}

export const GOAL_PHASE_LABEL: Record<GoalPhase, string> = {
  active: "active",
  paused: "paused",
  budget_limit: "budget limit",
  complete: "complete",
  failed: "failed",
  cancelled: "cancelled",
  handed_off: "handed off",
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
  if (goal.phase === "handed_off") return [];
  return ["clear"];
}

/** The prompt that starts a goal; `contract` asks for a validation contract to be drafted and
 * approved first (`/goal --contract`), `mission` for milestones, each with its own contract
 * (`/goal --mission`, which includes the contract). */
export function goalStartCommand(objective: string, contract: boolean, mission = false): string {
  const flag = mission ? "--mission " : contract ? "--contract " : "";
  return `/goal ${flag}${objective.trim()}`;
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

/** Share of the spend envelope used, 0..1; `null` when the goal has no envelope. */
export function envelopeFraction(goal: GoalView): number | null {
  const envelope = goal.envelope;
  if (!envelope || envelope.slice <= 0) return null;
  return Math.max(0, Math.min(1, envelope.used / envelope.slice));
}

/** `Resumes at 04:00` for a goal paused on a spent window that continues by itself, else `null`.
 * Past a day out the weekday is added. */
export function resumeLine(goal: GoalView, nowMs: number): string | null {
  const at = goal.envelope?.resumes_at;
  if (goal.phase !== "paused" || at == null) return null;
  const when = new Date(at * 1000);
  const far = Math.abs(at * 1000 - nowMs) >= 20 * 3600 * 1000;
  const clock = `${String(when.getHours()).padStart(2, "0")}:${String(when.getMinutes()).padStart(2, "0")}`;
  const day = far ? `${["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][when.getDay()]} ` : "";
  return `Resumes at ${day}${clock}`;
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

export interface ContractRow {
  id: number;
  text: string;
  state: "pass" | "fail" | "pending";
  /** Symbol shown beside the row; the colour is never the only signal. */
  mark: string;
  command: string;
  /** Why it failed, then the last line of proof; empty for a pass without output. */
  detail: string;
  /** Screenshot a behaviour probe took, for the evidence image route; `null` without one. */
  artifact: string | null;
  /** The QA runner (not the worker) checks this row by running the thing. */
  behaviour: boolean;
}

/** The contract table the panel draws, one row per assertion; `null` when the goal has none. */
export function contractRows(goal: GoalView): ContractRow[] | null {
  const contract = goal.contract;
  if (!contract) return null;
  return contract.assertions.map((a) => {
    const lastProof = a.proof
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean)
      .pop();
    const detail = a.state === "fail" ? [a.reason, lastProof].filter(Boolean).join(" · ") : (lastProof ?? "");
    return {
      id: a.id,
      text: a.text,
      state: a.state,
      mark: a.state === "pass" ? "✓" : a.state === "fail" ? "✗" : "·",
      command: a.command,
      detail,
      artifact: a.artifact ?? null,
      behaviour: a.behaviour === true,
    };
  });
}

export interface MissionGroup {
  index: number;
  title: string;
  state: "done" | "active" | "upcoming";
  /** `done`, `current 3/5 pass` or `upcoming, 4 assertions`. */
  tally: string;
  rows: ContractRow[];
}

/** The contract rows grouped under their milestone; `null` unless the goal is a mission. */
export function missionGroups(goal: GoalView): MissionGroup[] | null {
  const mission = goal.contract?.mission;
  const rows = contractRows(goal);
  if (!mission || !rows) return null;
  return mission.milestones.map((m) => ({
    index: m.index,
    title: m.title,
    state: m.state,
    tally:
      m.state === "done"
        ? `done ${m.passed}/${m.total}`
        : m.state === "active"
          ? `current ${m.passed}/${m.total} pass`
          : `upcoming, ${m.total} assertions`,
    rows: rows.filter((r) => m.assertion_ids.includes(r.id)),
  }));
}

/** `Mission · milestone 2/4: 3/5 pass`; `null` unless the goal is a mission. */
export function missionLine(goal: GoalView): string | null {
  const mission = goal.contract?.mission;
  return mission ? `Mission · ${mission.progress}` : null;
}

/** `Contract · 2/3 pass`, or `Contract · 3 assertions` before the validator has run. */
export function contractHeadline(goal: GoalView): string | null {
  return goal.contract ? `Contract · ${goal.contract.headline}` : null;
}

/** `S1 handed off (wedged) → S2 running`; `null` for a goal that never changed worker. */
export function relayLine(goal: GoalView): string | null {
  if (goal.relay.length === 0) return null;
  return goal.relay
    .map((link, i) => {
      if (link.reason) return `S${i + 1} handed off (${link.reason})`;
      const tip = link.state === "active" ? "running" : (GOAL_PHASE_LABEL[link.state] ?? link.state);
      return `S${i + 1} ${tip}`;
    })
    .join(" → ");
}
