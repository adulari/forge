import type { PrChecks, PrSummary, RunStatus } from "../../lib/apiPr";

export type PrTone = "neutral" | "accent" | "success" | "danger" | "warn";

export interface PrChipModel {
  label: string;
  tone: PrTone;
  detail: string;
}

export function checksLabel(checks: PrChecks): string {
  if (checks.total === 0) return "no checks";
  if (checks.failing > 0) return `${checks.failing} of ${checks.total} failing`;
  if (checks.pending > 0) return `${checks.pending} of ${checks.total} running`;
  return `${checks.total} passing`;
}

export function checksTone(checks: PrChecks): PrTone {
  if (checks.total === 0) return "neutral";
  if (checks.failing > 0) return "danger";
  if (checks.pending > 0) return "warn";
  return "success";
}

export function prChip(pr: PrSummary): PrChipModel {
  const base = `#${pr.number}`;
  if (pr.state === "merged") return { label: `${base} merged`, tone: "accent", detail: pr.title };
  if (pr.state === "closed") return { label: `${base} closed`, tone: "neutral", detail: pr.title };
  if (pr.draft) return { label: `${base} draft`, tone: "neutral", detail: pr.title };
  return { label: base, tone: checksTone(pr.checks), detail: `${pr.title} - ${checksLabel(pr.checks)}` };
}

export function shouldPoll(pr: PrSummary | null | undefined): boolean {
  return pr != null && pr.state === "open" && pr.checks.pending > 0;
}

export interface MergeGate {
  allowed: boolean;
  reason: string | null;
}

export function mergeGate(pr: PrSummary): MergeGate {
  if (pr.state !== "open") return { allowed: false, reason: `Already ${pr.state}` };
  if (pr.draft) return { allowed: false, reason: "Draft pull requests cannot be merged" };
  if (pr.mergeable === "conflicting") return { allowed: false, reason: "Merge conflicts with the base branch" };
  if (pr.head_sha === "") return { allowed: false, reason: "Head commit unknown" };
  return { allowed: true, reason: pr.checks.failing > 0 ? "Some checks are failing" : null };
}

export function runTone(run: { status: string; conclusion: string }): PrTone {
  if (run.status !== "completed") return "warn";
  if (run.conclusion === "success") return "success";
  if (run.conclusion === "cancelled" || run.conclusion === "skipped") return "neutral";
  return "danger";
}

export function runLabel(run: { status: RunStatus | string; conclusion: string }): string {
  if (run.status === "completed") return run.conclusion || "completed";
  return run.status.replace(/_/g, " ");
}

export const isRunActive = (run: { status: string }): boolean => run.status !== "completed";

/** Pinned workflows float to the top, otherwise workflow order is kept. */
export function orderWorkflows<T extends { path: string }>(workflows: T[], pinned: readonly string[]): T[] {
  const rank = (w: T) => {
    const at = pinned.indexOf(w.path);
    return at === -1 ? pinned.length : at;
  };
  return workflows
    .map((w, i) => ({ w, i }))
    .sort((a, b) => rank(a.w) - rank(b.w) || a.i - b.i)
    .map(({ w }) => w);
}

export function togglePinned(pinned: readonly string[], path: string): string[] {
  return pinned.includes(path) ? pinned.filter((p) => p !== path) : [...pinned, path];
}

export function parsePinned(stored: string | null): string[] {
  if (!stored) return [];
  try {
    const value: unknown = JSON.parse(stored);
    return Array.isArray(value) ? value.filter((v): v is string => typeof v === "string") : [];
  } catch {
    return [];
  }
}

export interface ProjectRep {
  cwd: string;
  sessionId: string;
}

/** One representative session per distinct project directory, most recently active first. */
export function projectReps(sessions: readonly { id: string; cwd: string; last_activity: number }[]): ProjectRep[] {
  const seen = new Map<string, { id: string; at: number }>();
  for (const s of sessions) {
    if (!s.cwd) continue;
    const prior = seen.get(s.cwd);
    if (!prior || s.last_activity > prior.at) seen.set(s.cwd, { id: s.id, at: s.last_activity });
  }
  return [...seen.entries()]
    .sort((a, b) => b[1].at - a[1].at)
    .map(([cwd, v]) => ({ cwd, sessionId: v.id }));
}
