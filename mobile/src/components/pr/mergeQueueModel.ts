import type { PrQueueAction, PrQueueRow, PrRepoQueue } from "../../lib/apiPr";
import type { WatchTone } from "./prWatchModel";

export interface QueueRowStatus {
  label: string;
  tone: WatchTone;
  detail: string | null;
}

export type QueueRowState = PrQueueRow["state"];

/** The owner's pause wins over whatever state the row carried when it was held. */
export function effectiveState(row: PrQueueRow): QueueRowState {
  return row.held ? "held" : row.state;
}

export function isWaiting(row: PrQueueRow): boolean {
  const state = effectiveState(row);
  return state === "queued" || state === "head" || state === "checks";
}

export function rowStatus(row: PrQueueRow): QueueRowStatus {
  const state = effectiveState(row);
  const detail = row.reason;
  switch (state) {
    case "head":
      return { label: "Next to land", tone: "accent", detail };
    case "queued":
      return { label: row.position != null ? `Queued #${row.position}` : "Queued", tone: "neutral", detail };
    case "checks": {
      const update = row.last_update ? `${row.last_update}.` : null;
      return {
        label: "Updated - in checks",
        tone: "accent",
        detail: [update, detail].filter(Boolean).join(" ") || null,
      };
    }
    case "held":
      return { label: "Held", tone: "neutral", detail: detail ?? "Paused by you. It does not block the queue." };
    case "skipped":
      return { label: "Handed back", tone: "warn", detail };
    case "merged":
      return { label: "Merged", tone: "success", detail };
    case "blocked":
      return { label: "Blocked", tone: "danger", detail };
  }
}

export const ACTION_LABELS: Record<PrQueueAction, string> = {
  hold: "Hold",
  release: "Release",
  front: "Move to front",
  retry: "Retry",
};

export function availableActions(row: PrQueueRow): PrQueueAction[] {
  const state = effectiveState(row);
  const actions: PrQueueAction[] = [];
  const first = state === "head" || row.position === 1;
  if (state === "held") actions.push("release");
  if (state === "queued" || state === "head" || state === "checks") actions.push("hold");
  if ((state === "queued" || state === "checks" || state === "held") && !first) actions.push("front");
  if (state === "skipped") actions.push("retry");
  return actions;
}

export interface QueueSummary {
  waiting: number;
  handedBack: number;
  held: number;
}

export function queueSummary(queues: readonly PrRepoQueue[]): QueueSummary {
  const summary: QueueSummary = { waiting: 0, handedBack: 0, held: 0 };
  for (const row of queues.flatMap((queue) => queue.rows)) {
    const state = effectiveState(row);
    if (isWaiting(row)) summary.waiting += 1;
    else if (state === "skipped") summary.handedBack += 1;
    else if (state === "held") summary.held += 1;
  }
  return summary;
}

export function summaryLabel(summary: QueueSummary): string {
  const parts: string[] = [];
  if (summary.waiting > 0) parts.push(`${summary.waiting} waiting`);
  if (summary.held > 0) parts.push(`${summary.held} held`);
  if (summary.handedBack > 0) parts.push(`${summary.handedBack} handed back`);
  return parts.length > 0 ? parts.join(", ") : "Empty";
}

/** `/home/me/code/forge/.git` -> `forge`; anything else is shown as given. */
export function repoName(repo: string): string {
  const trimmed = repo.replace(/\/+$/, "").replace(/\/\.git$/, "");
  const last = trimmed.split("/").filter(Boolean).pop();
  return last && last.length > 0 ? last : repo;
}

export function pushesLabel(row: PrQueueRow): string | null {
  const parts: string[] = [];
  if (row.pushes > 0) parts.push(`${row.pushes} ${row.pushes === 1 ? "push" : "pushes"} by the queue`);
  if (row.last_update && effectiveState(row) !== "checks") parts.push(row.last_update);
  return parts.length > 0 ? parts.join(" - ") : null;
}
