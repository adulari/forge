// "Ship": commit, push, open the pull request and arm PR Watch in one call
// (`POST /api/sessions/{id}/ship`, crates/forge-cli/src/serve_git/ship.rs). Like the rest of the
// PR routes it needs a direct daemon connection. A refusal is data (`ok: false` + `refused`), not
// an HTTP error, so the sheet can show which step stopped and why.
import { request } from "./api";
import type { PrWatch, PrWatchMethod } from "./apiPr";

export type ShipStepKey = "branch" | "commit" | "push" | "pr" | "watch";

export interface ShipStepRow {
  step: ShipStepKey;
  status: "done" | "skipped" | "refused";
  detail: string;
}

export interface ShipResponse {
  ok: boolean;
  steps: ShipStepRow[];
  branch: string | null;
  base: string | null;
  commit: { sha: string; message: string } | null;
  pr: { number: number | null; url: string; title: string; existing: boolean } | null;
  watch: PrWatch | null;
  refused: { step: ShipStepKey; reason: string } | null;
}

export interface ShipRequest {
  message?: string;
  title?: string;
  body?: string;
  base?: string;
  draft?: boolean;
  method?: PrWatchMethod;
}

export const shipSession = (baseUrl: string, sessionId: string, body: ShipRequest) =>
  request<ShipResponse>(
    baseUrl,
    `/api/sessions/${encodeURIComponent(sessionId)}/ship`,
    { method: "POST", body: JSON.stringify(body) },
    // Commit, push and the PR call each run hooks and the network; the default timeout is too short.
    180_000,
  );
