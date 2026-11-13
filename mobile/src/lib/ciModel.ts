// Pure view logic for Local CI: what the plan sheet lists and the session banner says.
import type { CiPlanResponse, CiPlanRow, CiRowState } from "./apiCi";
import type { SnapshotCi } from "./ws";

export interface CiStatusChip {
  label: string;
  tone: "success" | "danger" | "neutral" | "accent";
}

export const CI_STATE_CHIP: Record<CiRowState, CiStatusChip> = {
  pending: { label: "not run", tone: "neutral" },
  pass: { label: "pass", tone: "success" },
  cached: { label: "pass (cached)", tone: "success" },
  fail: { label: "fail", tone: "danger" },
  not_run: { label: "not run", tone: "neutral" },
  skip: { label: "skipped", tone: "neutral" },
};

export interface CiSections {
  steps: CiPlanRow[];
  skips: CiPlanRow[];
}

export function ciSections(plan: CiPlanResponse | undefined): CiSections {
  const rows = plan?.rows ?? [];
  return {
    steps: rows.filter((row) => row.state !== "skip"),
    skips: rows.filter((row) => row.state === "skip"),
  };
}

export function ciHeadline(plan: CiPlanResponse | undefined, running: boolean): string {
  if (running) return "Running the plan on the host…";
  if (!plan) return "Reading the repository's workflows…";
  const { steps, skips } = ciSections(plan);
  if (steps.length === 0) {
    return skips.length > 0
      ? "Nothing in the workflows can run here; every job is listed below with its reason."
      : "No GitHub workflows found in this repository.";
  }
  const failed = steps.filter((s) => s.state === "fail").length;
  if (plan.ok === true) return `All ${steps.length} steps passed.`;
  if (failed > 0) return `${failed} of ${steps.length} steps failed.`;
  if (plan.summary) return plan.summary;
  return `${steps.length} steps would run, ${skips.length} skipped.`;
}

/** The gate note: goals and pushes only wait on the plan while `[ci] gate` is `auto`. */
export function ciGateNote(plan: CiPlanResponse | undefined): string | null {
  if (!plan) return null;
  return plan.gate === "off"
    ? "Gate is off on this host: goals and pushes do not wait on this plan."
    : "Goals finish and pushes happen only after this plan passes.";
}

export function ciDuration(secs: number): string {
  if (secs <= 0) return "";
  if (secs < 60) return `${secs}s`;
  return `${Math.floor(secs / 60)}m ${secs % 60}s`;
}

export interface CiBanner {
  tone: "danger" | "neutral";
  message: string;
  /** Opens the sheet; absent while a run is only in flight. */
  action: boolean;
}

const VIA: Record<string, string> = {
  goal: "The goal can't finish",
  push: "The push is held back",
};

/** The session screen's line about Local CI: which step blocked a goal or a push, or a run in
 * flight. Nothing when the plan is quiet or passing. */
export function ciBanner(ci: SnapshotCi | null | undefined): CiBanner | null {
  if (!ci) return null;
  if (ci.blocked) {
    const lead = VIA[ci.blocked.via] ?? "Work is held back";
    return { tone: "danger", message: `${lead}: local CI step "${ci.blocked.name}" failed.`, action: true };
  }
  if (ci.running) {
    return {
      tone: "neutral",
      message: `Local CI: ${ci.running.name} (${ci.running.index}/${ci.running.total})…`,
      action: true,
    };
  }
  return null;
}
