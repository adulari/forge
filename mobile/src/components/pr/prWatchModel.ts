import type { PrWatch, PrWatchMethod, PrWatchMode } from "../../lib/apiPr";
import { reviewLabel, withReview } from "./prReviewModel";

export const WATCH_MODES: { value: PrWatchMode; label: string }[] = [
  { value: "off", label: "Off" },
  { value: "fix", label: "Fix" },
  { value: "merge", label: "Fix + merge" },
];

export const WATCH_METHODS: { value: PrWatchMethod; label: string }[] = [
  { value: "squash", label: "Squash" },
  { value: "merge", label: "Merge" },
  { value: "rebase", label: "Rebase" },
  { value: "auto", label: "Auto-merge" },
];

export type WatchTone = "neutral" | "accent" | "success" | "danger" | "warn";

export interface WatchStatus {
  label: string;
  tone: WatchTone;
  detail: string | null;
}

export function watchStatus(watch: PrWatch | null): WatchStatus {
  const status = baseStatus(watch);
  const review = watch?.state === "watching" ? reviewLabel(watch.review) : null;
  if (!review) return status;
  const tone = watch?.review?.verdict === "clean" ? status.tone : "warn";
  return { ...status, tone: status.tone === "danger" ? status.tone : tone, detail: withReview(status.detail, review) };
}

function baseStatus(watch: PrWatch | null): WatchStatus {
  if (!watch || watch.mode === "off") return { label: "Not watching", tone: "neutral", detail: null };
  const pr = watch.pr_number != null ? `#${watch.pr_number}` : "the pull request";
  if (watch.state === "merged") return { label: "Merged", tone: "success", detail: `${pr} merged; the session is marked done.` };
  if (watch.state === "blocked") {
    return { label: "Blocked", tone: "danger", detail: watch.blocked_reason ?? "The watcher stopped." };
  }
  const detail = watch.blocked_reason ?? watch.last_action;
  const queue = watch.mode === "merge" ? watch.queue : null;
  if (queue) {
    const detailed = queue.reason ?? watch.hold ?? detail;
    switch (queue.state) {
      case "queued":
        return { label: "Queued to merge", tone: "neutral", detail: detailed };
      case "head":
        return { label: "Landing next", tone: "accent", detail: detailed };
      case "checks":
        return {
          label: "Updated, in checks",
          tone: "accent",
          detail: queue.last_update ? `${queue.last_update}. ${detailed ?? ""}`.trim() : detailed,
        };
      case "held":
        return { label: "Held", tone: "neutral", detail: detailed };
      case "skipped":
        return { label: "Handed back", tone: "warn", detail: detailed };
    }
  }
  return { label: watch.mode === "merge" ? "Watching to merge" : "Watching", tone: watch.blocked_reason ? "warn" : "accent", detail };
}

export function isWatching(watch: PrWatch | null | undefined): boolean {
  return watch != null && watch.mode !== "off" && watch.state === "watching";
}

/** The session's watch follows a PR other than `number`, so its controls do not belong on that PR's page. */
export function watchesAnotherPr(watch: PrWatch | null, number: number): boolean {
  return watch?.pr_number != null && watch.pr_number !== number;
}
