// Local CI over the session routes (docs/features/ci-parity.md): the repository's workflows as a
// plan, and a fail-fast run of it on the host. Both are bridged for Anywhere.
import { request } from "./api";

export type CiRowState = "pending" | "pass" | "cached" | "fail" | "not_run" | "skip";

export interface CiPlanRow {
  state: CiRowState;
  /** `workflow::job::step`; absent on a skip. */
  key?: string;
  job?: string;
  /** `job / step` for a step, the job or step that was left out for a skip. */
  name: string;
  /** First line of the command for a step, the reason for a skip. */
  detail: string;
  /** The step's whole script. */
  command?: string;
  /** `K=V` pairs the step runs with. */
  env?: string[];
  /** Output tail of a step that failed or timed out. */
  tail?: string;
  secs: number;
}

export interface CiPlanResponse {
  rows: CiPlanRow[];
  /** `Last run 3 min ago: 12 passed, 1 failed, 0 not run`. */
  summary?: string | null;
  noops: number;
  /** `auto`: goals and pushes wait on this plan. `off`: they do not. */
  gate: "auto" | "off";
  /** Present after a run: every step passed (or its tool is not installed on the host). */
  ok?: boolean;
}

export const getSessionCi = (baseUrl: string, sessionId: string) =>
  request<CiPlanResponse>(baseUrl, `/api/sessions/${encodeURIComponent(sessionId)}/ci`);

// A plan can take many minutes (the host's own total budget is two hours); the run keeps going on
// the host if this gives up, and the session snapshot reports it.
export const RUN_CI_TIMEOUT_MS = 30 * 60_000;

export const runSessionCi = (baseUrl: string, sessionId: string) =>
  request<CiPlanResponse>(
    baseUrl,
    `/api/sessions/${encodeURIComponent(sessionId)}/ci/run`,
    { method: "POST", body: "{}" },
    RUN_CI_TIMEOUT_MS,
  );
