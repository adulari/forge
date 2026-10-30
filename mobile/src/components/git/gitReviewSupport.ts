// Git review runs entirely over `/api/git/*` (see the header comment in GitReviewDock.tsx).
//
// The READING half of that surface is now carried by the Forge Anywhere bridge: `git_status`,
// `git_branches` and `git_diff` are typed `RouteId` variants in
// forge-anywhere-protocol/src/bridge.rs, so a diff can be reviewed from the phone — the case
// Anywhere exists for. The MUTATING half (switch/stage/unstage/commit) is deliberately not
// bridged: the host refuses those methods on the git routes, so a relay command cannot alter the
// working tree. Over Anywhere the dock therefore renders read-only rather than offering controls
// whose requests would come back denied.
//
// Extracted from GitReviewDock so both predicates are unit-testable without rendering the dock.
import { supportsDirectDaemonEndpoints } from "../../lib/transport";

/** Reviewing (status/diff/branches) works on every transport. */
export function isGitReviewSupported(_baseUrl: string | null): boolean {
  return true;
}

/** Staging, committing and switching branches need a direct daemon connection. */
export function isGitReviewReadOnly(baseUrl: string | null): boolean {
  return !(!baseUrl || supportsDirectDaemonEndpoints(baseUrl));
}

export type GitStatusProblem =
  | { kind: "not-repo"; message: string }
  | { kind: "missing-session"; message: string }
  | { kind: "error"; message: string };

/** The daemon's repo-resolution failures are facts about the session, not transient faults, and a
 * 404 that carries the daemon's JSON `{error}` means the session is unknown to the server being
 * asked (a wrong token answers plain text, which stays the generic pairing message). Both get a
 * quiet empty state instead of an error with a Retry button. */
export function classifyGitStatusError(
  error: { message: string; status?: number; body?: unknown },
  host: string | null,
): GitStatusProblem {
  const { message } = error;
  if (message.includes("not inside a git repository")) {
    return { kind: "not-repo", message: "This session's folder is not a git repository." };
  }
  if (message.includes("no working directory")) {
    return { kind: "not-repo", message };
  }
  const body = error.body as { error?: unknown } | null | undefined;
  if (error.status === 404 && typeof body?.error === "string") {
    return {
      kind: "missing-session",
      message: `This session isn't known to ${host ?? "this server"}. Open it from the server it runs on.`,
    };
  }
  return { kind: "error", message };
}

/** Where the commit button's "last used" action is remembered. */
export type CommitAction = "commit" | "commit-push" | "commit-push-pr";

export const COMMIT_ACTION_LABEL: Record<CommitAction, string> = {
  commit: "Commit",
  "commit-push": "Commit & push",
  "commit-push-pr": "Push & create PR",
};

export function parseCommitAction(stored: string | null): CommitAction {
  return stored === "commit-push" || stored === "commit-push-pr" ? stored : "commit";
}

/** The Ctrl/Cmd+Enter chord, from a web key event's fields. Plain Enter must keep inserting a
 * newline in the message well. */
export function isCommitChord(event: { key?: string; ctrlKey?: boolean; metaKey?: boolean }): boolean {
  return event.key === "Enter" && (event.ctrlKey === true || event.metaKey === true);
}

/** git's refusal to fast-forward a diverged branch - the one pull failure with a one-tap remedy. */
export function isNotFastForward(message: string): boolean {
  return /fast-forward|diverging branches|not possible to fast/i.test(message);
}

/** The prompt "Resolve with Forge" sends: the files, what is unfinished, and what to leave alone. */
export function conflictResolutionPrompt(paths: readonly string[], operation: string | null | undefined): string {
  const list = paths.map((path) => `- ${path}`).join("\n");
  const header = operation
    ? `A ${operation} is in progress and stopped on conflicts in these files:`
    : "These files have merge conflicts:";
  return [
    header,
    list,
    "",
    "Resolve each conflict by keeping the intent of both sides, remove every conflict marker, and run the project's checks. Stage the resolved files with git add, but do not commit or push, and do not abort the operation.",
  ].join("\n");
}

/** `↑2 ↓1`-style label for the sync buttons; empty when level or when there is no upstream. */
export function aheadBehindLabel(ahead: number, behind: number): string {
  return [ahead > 0 ? `↑${ahead}` : "", behind > 0 ? `↓${behind}` : ""].filter(Boolean).join(" ");
}

export type DiscardKind = "tracked" | "untracked";

export function discardMessage(
  paths: readonly string[],
  kind: DiscardKind,
): { title: string; message: string } {
  if (paths.length === 1) {
    return kind === "untracked"
      ? { title: "Delete this file?", message: `${paths[0]} is untracked and is deleted from disk. This cannot be undone.` }
      : {
          title: "Discard changes?",
          message: `${paths[0]} goes back to its staged or committed version. This cannot be undone.`,
        };
  }
  return kind === "untracked"
    ? {
        title: `Delete ${paths.length} files?`,
        message: "These untracked files are deleted from disk. This cannot be undone.",
      }
    : {
        title: `Discard ${paths.length} files?`,
        message: "Their working-tree changes are thrown away. Staged changes stay. This cannot be undone.",
      };
}
