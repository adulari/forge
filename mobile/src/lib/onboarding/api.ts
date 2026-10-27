// HTTP client for /api/onboarding/*. Loopback-only on the daemon: a remote or tunnelled origin
// gets 403, which callers treat as "setup is not available from here".
import { request } from "../api";
import type { Action, Brief, JobPoll, Recommended, Snapshot, Started, StepId } from "./types";

export function getOnboarding(baseUrl: string, options: { force?: boolean } = {}): Promise<Snapshot> {
  return request(baseUrl, `/api/onboarding${options.force ? "?force=1" : ""}`, undefined, 20_000);
}

export function getOnboardingBrief(baseUrl: string): Promise<Brief> {
  return request(baseUrl, "/api/onboarding?brief=1", undefined, 20_000);
}

export type StepOp = "done" | "skip" | "reset";

export function setStep(baseUrl: string, step: StepId, op: StepOp): Promise<Snapshot> {
  return request(
    baseUrl,
    `/api/onboarding/steps/${encodeURIComponent(step)}`,
    { method: "POST", body: JSON.stringify({ op }) },
    20_000,
  );
}

/** `postpone` leaves setup open for next launch; `complete` ends the run. */
export function setFlow(baseUrl: string, op: "postpone" | "complete"): Promise<Snapshot> {
  return request(
    baseUrl,
    "/api/onboarding/steps/flow",
    { method: "POST", body: JSON.stringify({ op }) },
    20_000,
  );
}

export function startAction(baseUrl: string, action: Action, step?: StepId): Promise<Started> {
  return request(baseUrl, "/api/onboarding/actions", {
    method: "POST",
    body: JSON.stringify({ action, step }),
  });
}

export async function getRecommended(baseUrl: string): Promise<Recommended[]> {
  const body = await request<{ items: Recommended[] }>(
    baseUrl,
    "/api/onboarding/actions",
    { method: "POST", body: JSON.stringify({ recommended: true }) },
    20_000,
  );
  return body.items;
}

/** Events after `cursor`. With `waitMs` the daemon holds the request open until something
 * happens, so a screen follows a job without hammering it. */
export function pollJob(
  baseUrl: string,
  job: number,
  cursor: number,
  waitMs = 0,
  signal?: AbortSignal,
): Promise<JobPoll> {
  return request(
    baseUrl,
    `/api/onboarding/jobs/${job}?cursor=${cursor}&wait=${waitMs}`,
    { signal },
    waitMs + 15_000,
  );
}

export function sendJobInput(baseUrl: string, job: number, text: string): Promise<{ ok: boolean }> {
  return request(baseUrl, `/api/onboarding/jobs/${job}/input`, {
    method: "POST",
    body: JSON.stringify({ text }),
  });
}

export function cancelJob(baseUrl: string, job: number): Promise<{ ok: boolean }> {
  return request(baseUrl, `/api/onboarding/jobs/${job}`, { method: "DELETE" });
}
