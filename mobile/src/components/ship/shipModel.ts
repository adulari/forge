// The Ship sheet as plain data: the four progress rows (committed, pushed, PR, watching), what each
// says in every state, and the headline. Pure so vitest covers it without react-native.
import type { ShipPlan, ShipResponse, ShipStepKey } from "../../lib/apiShip";

export type ShipRowKey = Exclude<ShipStepKey, "branch">;
export type ShipRowState = "pending" | "active" | "done" | "skipped" | "refused" | "blocked";

export interface ShipRow {
  key: ShipRowKey;
  label: string;
  state: ShipRowState;
  detail: string | null;
}

const LABELS: Record<ShipRowKey, string> = {
  commit: "Commit changes",
  push: "Push branch",
  pr: "Open pull request",
  watch: "Watch and merge",
};

const ORDER: ShipRowKey[] = ["commit", "push", "pr", "watch"];

/** `response` null with `running` true is the request in flight; both falsy is the idle sheet. */
export function shipRows(response: ShipResponse | null, running: boolean): ShipRow[] {
  return ORDER.map((key): ShipRow => {
    const label = LABELS[key];
    if (!response) {
      return { key, label, state: running && key === "commit" ? "active" : "pending", detail: null };
    }
    const step = response.steps.find((s) => s.step === key);
    if (step) return { key, label, state: step.status, detail: step.detail };
    // The daemon stops at the first refusal, so a row with no step after one was never reached.
    return { key, label, state: response.refused ? "blocked" : "pending", detail: null };
  });
}

export function shipHeadline(response: ShipResponse | null, running: boolean): string {
  if (running) return "Shipping…";
  if (!response) return "Commit, push, open the PR and let Forge merge it when checks pass.";
  if (response.ok) {
    const pr = response.pr?.number != null ? `PR #${response.pr.number}` : "The pull request";
    return `${pr} is open and being watched. Forge merges it when checks pass.`;
  }
  const refused = response.refused;
  if (!refused) return "Shipping stopped.";
  const where = refused.step === "branch" ? "before committing" : `at ${LABELS[refused.step].toLowerCase()}`;
  return `Stopped ${where}: ${refused.reason}`;
}

/** The request body for a title typed into the sheet; an empty title means "generate one". */
export function shipRequestBody(title: string): { title?: string } {
  const trimmed = title.trim().replace(/\s+/g, " ");
  return trimmed ? { title: trimmed } : {};
}

export function shipFinished(response: ShipResponse | null): boolean {
  return response != null && !response.preview && (response.ok || response.refused != null);
}

const SOURCE: Record<ShipPlan["source"], string> = {
  index: "the files you staged",
  session: "this session's files",
  worktree: "all changes in the worktree",
};

const MARK = { created: "A", modified: "M", deleted: "D", renamed: "R" } as const;

export interface ShipPlanView {
  heading: string;
  files: { mark: string; path: string }[];
  leftOut: string | null;
}

/** The exact commit set for the sheet, shown before the user confirms. */
export function shipPlanView(plan: ShipPlan): ShipPlanView {
  const heading =
    plan.files.length === 0
      ? "No file changes: the commits already on the branch ship."
      : `Commits ${plan.files.length} ${plan.files.length === 1 ? "file" : "files"} from ${SOURCE[plan.source]}`;
  const leftOut =
    plan.left_out.length === 0
      ? null
      : `Left out: ${plan.left_out.slice(0, 5).join(", ")}${plan.left_out.length > 5 ? ` +${plan.left_out.length - 5} more` : ""}`;
  return {
    heading,
    files: plan.files.map((f) => ({ mark: MARK[f.kind] ?? "M", path: f.path })),
    leftOut,
  };
}
