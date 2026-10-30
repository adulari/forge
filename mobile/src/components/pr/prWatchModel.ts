import type { PrWatch, PrWatchMethod, PrWatchMode } from "../../lib/apiPr";

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
  if (!watch || watch.mode === "off") return { label: "Not watching", tone: "neutral", detail: null };
  const pr = watch.pr_number != null ? `#${watch.pr_number}` : "the pull request";
  if (watch.state === "merged") return { label: "Merged", tone: "success", detail: `${pr} merged; the session is marked done.` };
  if (watch.state === "blocked") {
    return { label: "Blocked", tone: "danger", detail: watch.blocked_reason ?? "The watcher stopped." };
  }
  const detail = watch.blocked_reason ?? watch.last_action;
  return { label: watch.mode === "merge" ? "Watching to merge" : "Watching", tone: watch.blocked_reason ? "warn" : "accent", detail };
}

export function isWatching(watch: PrWatch | null | undefined): boolean {
  return watch != null && watch.mode !== "off" && watch.state === "watching";
}
