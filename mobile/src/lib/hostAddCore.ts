export type AddHostPhase = "running" | "awaiting_approval" | "done" | "failed";

export interface AddedHost {
  name: string;
  target: string;
  added_ms: number;
  paired: boolean;
  version: string;
}

export interface AddHostJob {
  id: string;
  target: string;
  phase: AddHostPhase;
  step: string;
  lines: string[];
  /** The code THIS machine computed for the waiting device: the one to compare. */
  safety_code: string | null;
  /** What the new machine printed for itself. */
  remote_safety_code: string | null;
  error: string | null;
  host: AddedHost | null;
}

/** `[user@]host` as ssh takes it; the daemon enforces the same rule. */
export function isValidHostTarget(target: string): boolean {
  const value = target.trim();
  return value.length > 0 && value.length <= 253 && !value.startsWith("-") && /^[A-Za-z0-9._@:[\]%-]+$/.test(value);
}

/** The command the same machine can be added with from a terminal. */
export function hostAddCommand(target: string, name: string): string {
  const parts = ["forge host add", target.trim() || "user@host"];
  if (name.trim()) parts.push(`--name "${name.trim().replace(/(["\\$`])/g, "\\$1")}"`);
  return parts.join(" ");
}

export function isTerminalPhase(phase: AddHostPhase): boolean {
  return phase === "done" || phase === "failed";
}

/** A relay-reached daemon cannot run this: the add-a-machine routes are direct-only. */
export function canAddMachineVia(transport: "direct" | "anywhere" | undefined): boolean {
  return (transport ?? "direct") === "direct";
}

/** Poll quickly while things move; after failed polls back off instead of giving up. */
export function nextPollDelay(consecutiveFailures: number, baseMs = 1500): number {
  if (consecutiveFailures <= 0) return baseMs;
  return Math.min(15_000, baseMs * 2 ** Math.min(consecutiveFailures, 4));
}

/** The job still running for `target`, if any: lets a reopened sheet pick the work back up. */
export function jobToResume(jobs: readonly AddHostJob[], target?: string): AddHostJob | null {
  const active = jobs.filter((job) => !isTerminalPhase(job.phase));
  if (target) return active.find((job) => job.target === target) ?? null;
  return active[0] ?? null;
}
