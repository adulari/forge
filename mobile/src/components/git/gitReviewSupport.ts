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
