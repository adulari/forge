// Prompts and result wording for the session-level worktree actions. The TUI owns the
// `/worktree` slash command; the mobile and desktop UI drives it by sending the same prompt.
import type { WorktreePullResult } from "./apiProjects";

export function worktreeNewCommand(name: string, carry: boolean): string {
  const clean = name.trim();
  return ["/worktree new", clean, carry ? "--carry" : ""].filter(Boolean).join(" ");
}

export const WORKTREE_PULL_COMMAND = "/worktree pull";

export interface PullResultView {
  tone: "success" | "neutral" | "warn" | "danger";
  title: string;
  /** The conflict handoff only makes sense with a prompt to hand over. */
  canHandoff: boolean;
}

export function pullResultView(result: WorktreePullResult): PullResultView {
  switch (result.status) {
    case "up_to_date":
      return { tone: "neutral", title: "Already up to date", canHandoff: false };
    case "updated":
      return { tone: "success", title: result.stashed ? "Updated, your changes were re-applied" : "Updated", canHandoff: false };
    case "conflict":
      return { tone: "warn", title: "Merge conflicts", canHandoff: !!result.handoff_prompt };
    case "stash_conflict":
      return { tone: "warn", title: "Pulled, but your changes conflict", canHandoff: !!result.handoff_prompt };
    case "failed":
      return { tone: "danger", title: "Pull failed", canHandoff: false };
  }
}

export interface WorktreeSetupStatus {
  state: "running" | "done" | "failed" | "interrupted";
  message: string;
}

const SETUP_FULL_CHARS = 4000;
const SETUP_PREVIEW_LINES = 3;

export interface WorktreeSetupBanner {
  tone: "neutral" | "warn";
  /** Everything worth reading: the end of the output, never its start. */
  message: string;
  /** The last few lines, for the collapsed banner. */
  preview: string;
  /** The preview hides part of the message, so the banner offers to expand it. */
  expandable: boolean;
  dismissible: boolean;
}

function tailOf(text: string, chars: number): string {
  return text.length > chars ? `…${text.slice(text.length - chars)}` : text;
}

/** Banner for the worktree's background copy + `setup`; nothing once there is nothing to say. */
export function worktreeSetupBanner(status: WorktreeSetupStatus | null | undefined): WorktreeSetupBanner | null {
  if (!status) return null;
  const make = (tone: "neutral" | "warn", head: string, detail: string, dismissible: boolean): WorktreeSetupBanner => {
    const body = tailOf(detail.trim(), SETUP_FULL_CHARS);
    const lines = body.split("\n");
    const shown = lines.slice(-SETUP_PREVIEW_LINES);
    const cut = lines.length > SETUP_PREVIEW_LINES;
    // At most three lines in all, so a three-line clamp always ends on the last line of output.
    const preview = cut ? `…${shown.join("\n")}` : body ? `${head} ${body}` : head;
    const message = body ? `${head} ${body}` : head;
    return { tone, message, preview, expandable: message !== preview, dismissible };
  };
  if (status.state === "running") {
    return make("neutral", "Setting up the worktree: copying files and running setup…", "", false);
  }
  if (status.state === "failed") return make("warn", "Worktree setup:", status.message, true);
  if (status.state === "interrupted") {
    return make("warn", "Worktree setup was interrupted, so it may be incomplete:", status.message, true);
  }
  return make("neutral", "Worktree ready.", status.message, true);
}
