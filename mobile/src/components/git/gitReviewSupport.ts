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
