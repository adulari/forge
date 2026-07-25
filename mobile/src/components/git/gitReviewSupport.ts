// Git review runs entirely over `/api/git/*` (see the header comment in GitReviewDock.tsx). Like
// the terminal PTY, that surface is not one the Forge Anywhere bridge carries — there is no git
// variant of `RouteId` in forge-anywhere-protocol/src/bridge.rs, so every request would fail
// identically forever with an internal "route is not allowlisted" error and a dead retry button.
// Extracted from GitReviewDock so the predicate is unit-testable without rendering the dock.
import { supportsDirectDaemonEndpoints } from "../../lib/transport";

export function isGitReviewSupported(baseUrl: string | null): boolean {
  return !baseUrl || supportsDirectDaemonEndpoints(baseUrl);
}
